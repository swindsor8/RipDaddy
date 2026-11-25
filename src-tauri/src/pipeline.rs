//! Decoupled two-stage rip-and-encode queue state machine, live Tokio workers,
//! atomic disk persistence (`queue.json`), dynamic job reordering, OS completion/eject
//! notifications, Tauri event emission, and child-process safety registry.
//!
//! Supported workflow modes (`PipelineMode`):
//! - `RipOnly`:      `QueuedForRip -> Ripping -> Completed`
//! - `EncodeOnly`:   `QueuedForEncode -> Encoding -> Completed`
//! - `RipAndEncode`: `QueuedForRip -> Ripping -> QueuedForEncode -> Encoding -> CleaningTemp -> Completed`
//!
//! Stage 1 (Rip Worker / `makemkvcon`) and Stage 2 (Encode Worker / `HandBrakeCLI`)
//! run independently over the ordered `jobs` Vec driven by `tokio::sync::watch` wake signals
//! so reordering queued jobs takes effect immediately on the next worker pick.

use crate::detect::{self, ResolvedCommand};
use crate::handbrake::{
    self, EncodeOptions, HandBrakeState, ProgressStreamParser, QualityProfile,
    TrackSelectionOptions,
};
use crate::makemkv::{self, RobotMessage};
use crate::metadata::{self, FolderOrganization, MediaMetadata, NamingConvention};
use crate::system;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, watch, RwLock};

const PROGRESS_THROTTLE: Duration = Duration::from_millis(200);
const PERSISTED_QUEUE_VERSION: u32 = 1;
const PERSISTED_QUEUE_FILENAME: &str = "queue.json";
pub const PIPELINE_EVENT_NAME: &str = "pipeline-event";

fn default_true() -> bool {
    true
}

/// Controls which pipeline stages a job executes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PipelineMode {
    RipOnly,
    EncodeOnly,
    RipAndEncode,
}

impl PipelineMode {
    /// Initial queue stage when a new job is enqueued in this mode.
    pub fn initial_stage(self) -> JobStage {
        match self {
            PipelineMode::RipOnly | PipelineMode::RipAndEncode => JobStage::QueuedForRip,
            PipelineMode::EncodeOnly => JobStage::QueuedForEncode,
        }
    }
}

/// Explicit finite states for a RipDaddy pipeline job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobStage {
    QueuedForRip,
    Ripping,
    QueuedForEncode,
    Encoding,
    CleaningTemp,
    Completed,
    Cancelled,
    Failed,
}

impl JobStage {
    /// Returns `true` if `self -> next` is a valid state transition for the given `PipelineMode`.
    pub fn can_transition_to_for_mode(self, mode: PipelineMode, next: JobStage) -> bool {
        use JobStage::*;
        match mode {
            PipelineMode::RipOnly => matches!(
                (self, next),
                (QueuedForRip, Ripping | Cancelled) | (Ripping, Completed | Cancelled | Failed)
            ),
            PipelineMode::EncodeOnly => matches!(
                (self, next),
                (QueuedForEncode, Encoding | Cancelled)
                    | (Encoding, Completed | Cancelled | Failed)
            ),
            PipelineMode::RipAndEncode => matches!(
                (self, next),
                (QueuedForRip, Ripping | Cancelled)
                    | (Ripping, QueuedForEncode | Cancelled | Failed)
                    | (QueuedForEncode, Encoding | Cancelled)
                    | (Encoding, CleaningTemp | Completed | Cancelled | Failed)
                    | (CleaningTemp, Completed | Failed)
            ),
        }
    }

    /// Returns `true` if `self -> next` is valid for the default `RipAndEncode` mode.
    pub fn can_transition_to(self, next: JobStage) -> bool {
        self.can_transition_to_for_mode(PipelineMode::RipAndEncode, next)
    }

    /// Returns `true` if the job is currently waiting in a queue and may be reordered.
    pub fn is_pending(self) -> bool {
        matches!(self, JobStage::QueuedForRip | JobStage::QueuedForEncode)
    }

    /// Returns `true` if a child process or cleanup is actively executing.
    pub fn is_running(self) -> bool {
        matches!(
            self,
            JobStage::Ripping | JobStage::Encoding | JobStage::CleaningTemp
        )
    }

    /// Returns `true` if no further transitions are permitted from this stage.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            JobStage::Completed | JobStage::Cancelled | JobStage::Failed
        )
    }
}

/// Direction for reordering a pending job within the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReorderAction {
    Top,
    Up,
    Down,
    Bottom,
}

/// Real-time progress snapshot associated with a queued or active job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    pub stage: JobStage,
    pub percent: f64,
    pub eta_seconds: Option<u64>,
    pub fps: Option<f64>,
    pub message: Option<String>,
}

/// Represents one disc title or local video file moving through the pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueJob {
    pub id: String,
    pub mode: PipelineMode,
    pub disc_name: String,
    pub drive_index: Option<u32>,
    pub title_id: Option<u32>,
    pub source_file_path: Option<String>,
    pub title_name: String,
    pub preset_name: Option<String>,
    pub temp_dir: String,
    pub temp_mkv_path: Option<String>,
    pub output_path: String,
    pub keep_temp_mkv: bool,
    pub naming_convention: NamingConvention,
    pub media_metadata: Option<MediaMetadata>,
    pub stage: JobStage,
    pub progress: JobProgress,
    pub error: Option<String>,
}

impl QueueJob {
    /// Validates and applies a mode-aware state transition on the job.
    pub fn transition_to(&mut self, next: JobStage) -> Result<(), PipelineError> {
        if !self.stage.can_transition_to_for_mode(self.mode, next) {
            return Err(PipelineError::InvalidTransition {
                from: self.stage,
                to: next,
            });
        }
        self.stage = next;
        self.progress.stage = next;
        if matches!(
            next,
            JobStage::QueuedForRip | JobStage::QueuedForEncode | JobStage::CleaningTemp
        ) {
            self.progress.percent = 0.0;
            self.progress.eta_seconds = None;
            self.progress.fps = None;
        } else if next == JobStage::Completed {
            self.progress.percent = 100.0;
            self.progress.eta_seconds = Some(0);
        }
        Ok(())
    }
}

/// Full queue snapshot returned to the frontend on load, pause/resume, or reorder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueStateSnapshot {
    pub jobs: Vec<QueueJob>,
    pub paused: bool,
    pub restored_pending_count: usize,
}

/// Request sent from the frontend to enqueue one disc title or local video file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnqueueJobRequest {
    pub mode: PipelineMode,
    pub disc_name: String,
    pub drive_index: Option<u32>,
    pub drive_device_path: Option<String>,
    pub title_id: Option<u32>,
    pub source_file_path: Option<String>,
    pub title_name: String,
    pub quality_profile: Option<QualityProfile>,
    pub custom_preset_file: Option<String>,
    pub custom_preset_name: Option<String>,
    pub temp_dir: String,
    pub output_dir: String,
    pub keep_temp_mkv: bool,
    pub auto_eject_disc: bool,
    #[serde(default = "default_true")]
    pub notify_on_complete: bool,
    pub naming_convention: NamingConvention,
    pub folder_organization: Option<FolderOrganization>,
    pub media_metadata: Option<MediaMetadata>,
    #[serde(default)]
    pub track_selection: TrackSelectionOptions,
}

/// Internal execution metadata retained alongside each `QueueJob` and persisted to `queue.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobExecutionSpec {
    makemkv_cmd: Option<ResolvedCommand>,
    handbrake_cmd: Option<ResolvedCommand>,
    quality_profile: Option<QualityProfile>,
    custom_preset_file: Option<String>,
    custom_preset_name: Option<String>,
    drive_device_path: Option<String>,
    auto_eject_disc: bool,
    #[serde(default = "default_true")]
    notify_on_complete: bool,
    #[serde(default)]
    track_selection: TrackSelectionOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedQueueEntry {
    job: QueueJob,
    spec: JobExecutionSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedQueueFile {
    version: u32,
    next_job_counter: u64,
    entries: Vec<PersistedQueueEntry>,
}

/// Tracks batch outcomes so a single native OS completion alert fires when the queue empties.
#[derive(Debug, Default)]
struct BatchNotifyTracker {
    completed: usize,
    failed: usize,
    last_title: String,
    last_mode: Option<PipelineMode>,
    last_ejected_rip_only: bool,
}

/// Structured events streamed from Rust to the Svelte frontend via Tauri events / `Channel`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PipelineEvent {
    #[serde(rename_all = "camelCase")]
    JobUpdated { job: QueueJob },
    #[serde(rename_all = "camelCase")]
    QueueSnapshot {
        jobs: Vec<QueueJob>,
        paused: bool,
    },
    #[serde(rename_all = "camelCase")]
    Progress {
        job_id: String,
        progress: JobProgress,
    },
    #[serde(rename_all = "camelCase")]
    StageChanged {
        job_id: String,
        from: JobStage,
        to: JobStage,
    },
    #[serde(rename_all = "camelCase")]
    LogLine {
        job_id: String,
        stage: JobStage,
        stream: String,
        line: String,
    },
    #[serde(rename_all = "camelCase")]
    TempCleaned {
        job_id: String,
        removed_path: String,
    },
    #[serde(rename_all = "camelCase")]
    DiscEjected {
        job_id: String,
        disc_name: String,
    },
    #[serde(rename_all = "camelCase")]
    BatchCompleted {
        completed: usize,
        failed: usize,
        title: String,
        body: String,
    },
}

/// Errors produced by the pipeline state machine and managed subprocess runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineError {
    InvalidTransition { from: JobStage, to: JobStage },
    JobNotFound(String),
    ShuttingDown,
    Cancelled,
    ProcessSpawnFailed(String),
    Io(String),
}

impl fmt::Display for PipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PipelineError::InvalidTransition { from, to } => {
                write!(f, "Invalid pipeline stage transition: {from:?} -> {to:?}")
            }
            PipelineError::JobNotFound(id) => write!(f, "Pipeline job not found: {id}"),
            PipelineError::ShuttingDown => write!(f, "Pipeline is shutting down"),
            PipelineError::Cancelled => write!(f, "Pipeline job was cancelled"),
            PipelineError::ProcessSpawnFailed(msg) => {
                write!(f, "Failed to spawn external binary: {msg}")
            }
            PipelineError::Io(msg) => write!(f, "Pipeline I/O error: {msg}"),
        }
    }
}

impl std::error::Error for PipelineError {}

#[derive(Debug)]
struct TrackedProcess {
    _stage: JobStage,
    pid: Option<u32>,
    cancel_tx: watch::Sender<bool>,
}

/// Tracks every active external child process (`makemkvcon`, `HandBrakeCLI`) so that
/// cancelling a job or closing the Tauri application cleanly terminates all children.
#[derive(Debug, Default)]
pub struct ProcessRegistry {
    active: Mutex<HashMap<String, TrackedProcess>>,
    shutting_down: AtomicBool,
}

impl ProcessRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a newly spawned child process and returns its cancellation watcher.
    pub fn register(
        &self,
        job_id: String,
        stage: JobStage,
        pid: Option<u32>,
    ) -> Result<watch::Receiver<bool>, PipelineError> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(PipelineError::ShuttingDown);
        }

        let (cancel_tx, cancel_rx) = watch::channel(false);
        let mut guard = self.active.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(
            job_id,
            TrackedProcess {
                _stage: stage,
                pid,
                cancel_tx,
            },
        );
        Ok(cancel_rx)
    }

    /// Removes a child process entry once it has exited and been reaped.
    pub fn unregister(&self, job_id: &str) {
        let mut guard = self.active.lock().unwrap_or_else(|e| e.into_inner());
        guard.remove(job_id);
    }

    /// Signals a running job to terminate its child process immediately.
    pub fn cancel(&self, job_id: &str) -> bool {
        let guard = self.active.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = guard.get(job_id) {
            let _ = entry.cancel_tx.send(true);
            true
        } else {
            false
        }
    }

    /// Returns the number of currently tracked active child processes.
    pub fn active_count(&self) -> usize {
        self.active.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Synchronous emergency teardown invoked from Tauri's `RunEvent::Exit`.
    pub fn terminate_all_sync(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);

        let pids: Vec<u32> = {
            let mut guard = self.active.lock().unwrap_or_else(|e| e.into_inner());
            let mut collected = Vec::new();
            for (_, entry) in guard.drain() {
                let _ = entry.cancel_tx.send(true);
                if let Some(pid) = entry.pid {
                    collected.push(pid);
                }
            }
            collected
        };

        for pid in pids {
            terminate_pid_sync(pid);
        }
    }
}

struct RegistryGuard {
    registry: Arc<ProcessRegistry>,
    job_id: String,
}

impl Drop for RegistryGuard {
    fn drop(&mut self) {
        self.registry.unregister(&self.job_id);
    }
}

#[cfg(windows)]
fn configure_platform_command(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_platform_command(_cmd: &mut Command) {}

#[cfg(windows)]
fn terminate_pid_sync(pid: u32) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

#[cfg(unix)]
fn terminate_pid_sync(pid: u32) {
    let pid_str = pid.to_string();
    let _ = std::process::Command::new("kill")
        .args(["-TERM", &pid_str])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = std::process::Command::new("kill")
        .args(["-KILL", &pid_str])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(any(unix, windows)))]
fn terminate_pid_sync(_pid: u32) {}

async fn next_optional_line<R: tokio::io::AsyncBufRead + Unpin>(
    lines: &mut Option<tokio::io::Lines<R>>,
) -> std::io::Result<Option<String>> {
    match lines.as_mut() {
        Some(stream) => stream.next_line().await,
        None => Ok(None),
    }
}

/// Spawns a `ResolvedCommand` (native, Flatpak, or toolbox host-delegated) under `tokio`,
/// streams `stdout` and `stderr` line-by-line, and guarantees immediate kill + reaping on cancel.
pub async fn spawn_and_stream_resolved<F>(
    resolved: &ResolvedCommand,
    args: &[String],
    job_id: &str,
    stage: JobStage,
    registry: Arc<ProcessRegistry>,
    mut on_line: F,
) -> Result<ExitStatus, PipelineError>
where
    F: FnMut(&'static str, String),
{
    let mut cmd = Command::new(&resolved.program);
    cmd.args(&resolved.prefix_args)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_platform_command(&mut cmd);

    let mut child = cmd
        .spawn()
        .map_err(|err| PipelineError::ProcessSpawnFailed(format!("{}: {err}", resolved.program)))?;

    let mut cancel_rx = match registry.register(job_id.to_string(), stage, child.id()) {
        Ok(rx) => rx,
        Err(err) => {
            let _ = child.kill().await;
            return Err(err);
        }
    };

    let _guard = RegistryGuard {
        registry,
        job_id: job_id.to_string(),
    };

    let mut stdout_lines = child.stdout.take().map(|out| BufReader::new(out).lines());
    let mut stderr_lines = child.stderr.take().map(|err| BufReader::new(err).lines());
    let mut stdout_open = stdout_lines.is_some();
    let mut stderr_open = stderr_lines.is_some();

    while stdout_open || stderr_open {
        tokio::select! {
            changed = cancel_rx.changed() => {
                if changed.is_err() || *cancel_rx.borrow() {
                    let _ = child.kill().await;
                    return Err(PipelineError::Cancelled);
                }
            }
            line = next_optional_line(&mut stdout_lines), if stdout_open => {
                match line {
                    Ok(Some(text)) => on_line("stdout", text),
                    Ok(None) | Err(_) => stdout_open = false,
                }
            }
            line = next_optional_line(&mut stderr_lines), if stderr_open => {
                match line {
                    Ok(Some(text)) => on_line("stderr", text),
                    Ok(None) | Err(_) => stderr_open = false,
                }
            }
        }
    }

    tokio::select! {
        changed = cancel_rx.changed() => {
            if changed.is_err() || *cancel_rx.borrow() {
                let _ = child.kill().await;
                return Err(PipelineError::Cancelled);
            }
            child.wait().await.map_err(|err| PipelineError::Io(err.to_string()))
        }
        status = child.wait() => {
            status.map_err(|err| PipelineError::Io(err.to_string()))
        }
    }
}

/// Convenience wrapper for spawning a direct binary path without prefix arguments.
pub async fn spawn_and_stream<F>(
    program: &Path,
    args: &[String],
    job_id: &str,
    stage: JobStage,
    registry: Arc<ProcessRegistry>,
    on_line: F,
) -> Result<ExitStatus, PipelineError>
where
    F: FnMut(&'static str, String),
{
    let resolved = ResolvedCommand {
        program: program.to_string_lossy().into_owned(),
        prefix_args: Vec::new(),
    };
    spawn_and_stream_resolved(&resolved, args, job_id, stage, registry, on_line).await
}

/// Shared runtime handles passed to the Stage 1 (Rip) and Stage 2 (Encode) workers.
#[derive(Clone)]
struct WorkerShared {
    jobs: Arc<RwLock<Vec<QueueJob>>>,
    specs: Arc<RwLock<HashMap<String, JobExecutionSpec>>>,
    processes: Arc<ProcessRegistry>,
    wake_tx: watch::Sender<u64>,
    paused: Arc<AtomicBool>,
    notifications_enabled: Arc<AtomicBool>,
    batch_tracker: Arc<Mutex<BatchNotifyTracker>>,
    store_path: Arc<Mutex<Option<PathBuf>>>,
    job_counter: Arc<AtomicU64>,
    app: AppHandle,
}

impl WorkerShared {
    fn wake_workers(&self) {
        self.wake_tx.send_modify(|seq| *seq = seq.wrapping_add(1));
    }

    async fn persist_queue(&self) {
        let path_opt = self
            .store_path
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(path) = path_opt else {
            return;
        };
        let jobs = self.jobs.read().await.clone();
        let specs = self.specs.read().await.clone();
        let next_counter = self.job_counter.load(Ordering::Relaxed);
        let _ = write_queue_snapshot_sync(&path, &jobs, &specs, next_counter);
    }

    async fn record_terminal_outcome(
        &self,
        job_snapshot: &QueueJob,
        succeeded: bool,
        notify_allowed: bool,
        ejected_rip_only: bool,
    ) {
        let all_terminal = self
            .jobs
            .read()
            .await
            .iter()
            .all(|j| j.stage.is_terminal());

        let maybe_alert = {
            let mut tracker = self.batch_tracker.lock().unwrap_or_else(|e| e.into_inner());
            if succeeded {
                tracker.completed += 1;
            } else {
                tracker.failed += 1;
            }
            tracker.last_title = job_snapshot.title_name.clone();
            tracker.last_mode = Some(job_snapshot.mode);
            tracker.last_ejected_rip_only = ejected_rip_only;

            if all_terminal && (tracker.completed + tracker.failed) > 0 {
                let completed = tracker.completed;
                let failed = tracker.failed;
                let last_title = std::mem::take(&mut tracker.last_title);
                let last_mode = tracker.last_mode.take();
                let ejected = tracker.last_ejected_rip_only;
                tracker.completed = 0;
                tracker.failed = 0;
                tracker.last_ejected_rip_only = false;
                Some((completed, failed, last_title, last_mode, ejected))
            } else {
                None
            }
        };

        if let Some((completed, failed, last_title, last_mode, ejected)) = maybe_alert {
            let (title, body) = format_batch_notification(
                completed,
                failed,
                &last_title,
                last_mode,
                ejected,
            );
            let _ = self.app.emit(
                PIPELINE_EVENT_NAME,
                &PipelineEvent::BatchCompleted {
                    completed,
                    failed,
                    title: title.clone(),
                    body: body.clone(),
                },
            );
            if notify_allowed && self.notifications_enabled.load(Ordering::Relaxed) {
                tokio::spawn(async move {
                    let _ = system::send_desktop_notification(&title, &body).await;
                });
            }
        }
    }
}

fn format_batch_notification(
    completed: usize,
    failed: usize,
    last_title: &str,
    last_mode: Option<PipelineMode>,
    ejected: bool,
) -> (String, String) {
    let eject_suffix = if ejected {
        " Disc ejected — ready for the next disc."
    } else {
        ""
    };

    if failed == 0 && completed == 1 {
        let verb = if last_mode == Some(PipelineMode::RipOnly) {
            "Rip Complete"
        } else {
            "Encode Complete"
        };
        (
            format!("RipDaddy · {verb}"),
            format!("{last_title} is ready in your output folder.{eject_suffix}"),
        )
    } else if failed == 0 {
        (
            "RipDaddy · Batch Complete".to_string(),
            format!("All {completed} queued jobs finished successfully.{eject_suffix}"),
        )
    } else if completed > 0 {
        (
            "RipDaddy · Batch Finished with Errors".to_string(),
            format!(
                "{completed} completed, {failed} failed. Open Queue Logs for details.{eject_suffix}"
            ),
        )
    } else {
        (
            "RipDaddy · Job Failed".to_string(),
            format!("{last_title} failed. Open Queue Logs for details."),
        )
    }
}

/// Tauri-managed state coordinating the decoupled, persistent, reorderable Rip -> Encode -> Cleanup pipeline.
pub struct PipelineController {
    jobs: Arc<RwLock<Vec<QueueJob>>>,
    specs: Arc<RwLock<HashMap<String, JobExecutionSpec>>>,
    processes: Arc<ProcessRegistry>,
    wake_tx: watch::Sender<u64>,
    paused: Arc<AtomicBool>,
    restored_pending_count: AtomicUsize,
    notifications_enabled: Arc<AtomicBool>,
    batch_tracker: Arc<Mutex<BatchNotifyTracker>>,
    store_path: Arc<Mutex<Option<PathBuf>>>,
    app_handle: Mutex<Option<AppHandle>>,
    workers_started: AtomicBool,
    job_counter: Arc<AtomicU64>,
}

impl Default for PipelineController {
    fn default() -> Self {
        Self::new()
    }
}

impl PipelineController {
    pub fn new() -> Self {
        let (wake_tx, _wake_rx) = watch::channel(0u64);

        Self {
            jobs: Arc::new(RwLock::new(Vec::new())),
            specs: Arc::new(RwLock::new(HashMap::new())),
            processes: Arc::new(ProcessRegistry::new()),
            wake_tx,
            paused: Arc::new(AtomicBool::new(false)),
            restored_pending_count: AtomicUsize::new(0),
            notifications_enabled: Arc::new(AtomicBool::new(true)),
            batch_tracker: Arc::new(Mutex::new(BatchNotifyTracker::default())),
            store_path: Arc::new(Mutex::new(None)),
            app_handle: Mutex::new(None),
            workers_started: AtomicBool::new(false),
            job_counter: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Configures a custom queue persistence file path (primarily for deterministic testing).
    pub fn set_store_path(&self, path: PathBuf) {
        let mut slot = self.store_path.lock().unwrap_or_else(|e| e.into_inner());
        *slot = Some(path);
    }

    /// Updates whether native OS completion & disc-eject desktop notifications are enabled.
    pub fn set_notifications_enabled(&self, enabled: bool) {
        self.notifications_enabled.store(enabled, Ordering::Relaxed);
    }

    /// Loads saved queue entries from disk, recovers any jobs that were mid-flight when the app
    /// exited, and pauses worker auto-start when pending jobs were restored.
    pub fn restore_persisted_queue_sync(&self) -> usize {
        let path_opt = self
            .store_path
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(path) = path_opt else {
            return 0;
        };
        let Ok(raw) = std::fs::read_to_string(&path) else {
            return 0;
        };
        let Ok(parsed) = serde_json::from_str::<PersistedQueueFile>(&raw) else {
            return 0;
        };

        let mut restored_jobs = Vec::with_capacity(parsed.entries.len());
        let mut restored_specs = HashMap::with_capacity(parsed.entries.len());
        let mut pending_count = 0usize;

        for mut entry in parsed.entries {
            match entry.job.stage {
                JobStage::Ripping => {
                    // Interrupted mid-rip: clean partial staging folder and requeue cleanly
                    let staging_dir = staging_dir_for_job(&entry.job);
                    let _ = std::fs::remove_dir_all(&staging_dir);
                    entry.job.stage = JobStage::QueuedForRip;
                    entry.job.progress = JobProgress {
                        stage: JobStage::QueuedForRip,
                        percent: 0.0,
                        eta_seconds: None,
                        fps: None,
                        message: Some("Interrupted — restored to queue".to_string()),
                    };
                    pending_count += 1;
                }
                JobStage::Encoding => {
                    // Interrupted mid-encode: remove incomplete output file and requeue cleanly
                    let out_path = Path::new(&entry.job.output_path);
                    if out_path.exists() {
                        let _ = std::fs::remove_file(out_path);
                    }
                    entry.job.stage = JobStage::QueuedForEncode;
                    entry.job.progress = JobProgress {
                        stage: JobStage::QueuedForEncode,
                        percent: 0.0,
                        eta_seconds: None,
                        fps: None,
                        message: Some("Interrupted — restored to queue".to_string()),
                    };
                    pending_count += 1;
                }
                JobStage::CleaningTemp => {
                    let out_exists = Path::new(&entry.job.output_path).exists();
                    if out_exists {
                        if !entry.job.keep_temp_mkv {
                            if let Some(ref tmp) = entry.job.temp_mkv_path {
                                let tmp_path = Path::new(tmp);
                                let _ = std::fs::remove_file(tmp_path);
                                if let Some(parent) = tmp_path.parent() {
                                    let _ = std::fs::remove_dir(parent);
                                }
                            }
                        }
                        entry.job.stage = JobStage::Completed;
                        entry.job.progress = JobProgress {
                            stage: JobStage::Completed,
                            percent: 100.0,
                            eta_seconds: Some(0),
                            fps: None,
                            message: None,
                        };
                    } else {
                        entry.job.stage = JobStage::QueuedForEncode;
                        entry.job.progress = JobProgress {
                            stage: JobStage::QueuedForEncode,
                            percent: 0.0,
                            eta_seconds: None,
                            fps: None,
                            message: Some("Interrupted — restored to queue".to_string()),
                        };
                        pending_count += 1;
                    }
                }
                JobStage::QueuedForRip | JobStage::QueuedForEncode => {
                    if entry.job.progress.message.is_none() {
                        entry.job.progress.message =
                            Some("Restored from previous session".to_string());
                    }
                    pending_count += 1;
                }
                JobStage::Completed | JobStage::Cancelled | JobStage::Failed => {}
            }

            restored_specs.insert(entry.job.id.clone(), entry.spec);
            restored_jobs.push(entry.job);
        }

        let max_counter = parsed.next_job_counter.max(restored_jobs.len() as u64 + 1);
        self.job_counter.store(max_counter, Ordering::Relaxed);

        if let Ok(mut jobs_guard) = self.jobs.try_write() {
            *jobs_guard = restored_jobs.clone();
        }
        if let Ok(mut specs_guard) = self.specs.try_write() {
            *specs_guard = restored_specs.clone();
        }

        if pending_count > 0 {
            self.paused.store(true, Ordering::SeqCst);
            self.restored_pending_count
                .store(pending_count, Ordering::Relaxed);
        }

        let _ = write_queue_snapshot_sync(&path, &restored_jobs, &restored_specs, max_counter);
        pending_count
    }

    /// Spawns the independent Stage 1 (Rip) and Stage 2 (Encode) Tokio worker tasks
    /// after restoring any persisted jobs from `<app_data_dir>/queue.json`.
    pub fn start_workers(&self, app_handle: AppHandle) {
        {
            let mut slot = self.app_handle.lock().unwrap_or_else(|e| e.into_inner());
            *slot = Some(app_handle.clone());
        }

        if let Ok(data_dir) = app_handle.path().app_data_dir() {
            let mut slot = self.store_path.lock().unwrap_or_else(|e| e.into_inner());
            if slot.is_none() {
                *slot = Some(data_dir.join(PERSISTED_QUEUE_FILENAME));
            }
        }

        self.restore_persisted_queue_sync();

        if self.workers_started.swap(true, Ordering::SeqCst) {
            return;
        }

        let shared = WorkerShared {
            jobs: Arc::clone(&self.jobs),
            specs: Arc::clone(&self.specs),
            processes: Arc::clone(&self.processes),
            wake_tx: self.wake_tx.clone(),
            paused: Arc::clone(&self.paused),
            notifications_enabled: Arc::clone(&self.notifications_enabled),
            batch_tracker: Arc::clone(&self.batch_tracker),
            store_path: Arc::clone(&self.store_path),
            job_counter: Arc::clone(&self.job_counter),
            app: app_handle,
        };

        let rip_shared = shared.clone();
        let rip_wake = self.wake_tx.subscribe();
        tauri::async_runtime::spawn(async move {
            run_rip_worker(rip_wake, rip_shared).await;
        });

        let enc_shared = shared;
        let enc_wake = self.wake_tx.subscribe();
        tauri::async_runtime::spawn(async move {
            run_encode_worker(enc_wake, enc_shared).await;
        });
    }

    fn wake_workers(&self) {
        self.wake_tx.send_modify(|seq| *seq = seq.wrapping_add(1));
    }

    async fn persist_queue_async(&self) {
        let path_opt = self
            .store_path
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(path) = path_opt else {
            return;
        };
        let jobs = self.jobs.read().await.clone();
        let specs = self.specs.read().await.clone();
        let next_counter = self.job_counter.load(Ordering::Relaxed);
        let _ = write_queue_snapshot_sync(&path, &jobs, &specs, next_counter);
    }

    /// Validates, constructs, persists to disk, and wakes workers for one or more pipeline jobs.
    pub async fn enqueue_jobs(
        &self,
        requests: Vec<EnqueueJobRequest>,
        makemkv_cmd: Option<ResolvedCommand>,
        handbrake_cmd: Option<ResolvedCommand>,
    ) -> Result<Vec<QueueJob>, String> {
        if requests.is_empty() {
            return Err("No titles or video files selected to queue.".to_string());
        }

        let defaults = detect::detect_default_paths();
        let mut used_outputs: Vec<String> = self
            .jobs
            .read()
            .await
            .iter()
            .filter(|j| !j.stage.is_terminal())
            .map(|j| j.output_path.clone())
            .collect();
        let mut created_jobs = Vec::with_capacity(requests.len());

        for req in requests {
            let needs_rip = matches!(req.mode, PipelineMode::RipOnly | PipelineMode::RipAndEncode);
            let needs_encode =
                matches!(req.mode, PipelineMode::EncodeOnly | PipelineMode::RipAndEncode);

            if needs_rip && makemkv_cmd.is_none() {
                return Err(
                    "makemkvcon is not available. Check Settings → External CLIs.".to_string(),
                );
            }
            if needs_encode && handbrake_cmd.is_none() {
                return Err(
                    "HandBrakeCLI is not available. Check Settings → External CLIs.".to_string(),
                );
            }
            if needs_rip && (req.drive_index.is_none() || req.title_id.is_none()) {
                return Err("Rip job requires a valid optical drive index and title ID.".to_string());
            }
            if req.mode == PipelineMode::EncodeOnly && req.source_file_path.is_none() {
                return Err("Encode Only job requires a local source video file.".to_string());
            }

            let seq = self.job_counter.fetch_add(1, Ordering::Relaxed);
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let job_id = format!("job-{seq}-{stamp}");

            let extension = if req.mode == PipelineMode::RipOnly {
                "mkv".to_string()
            } else {
                req.quality_profile
                    .as_ref()
                    .map(|p| p.container.clone())
                    .unwrap_or_else(|| "mkv".to_string())
            };

            let folder_org = req
                .folder_organization
                .unwrap_or_else(|| FolderOrganization::default_for(req.naming_convention));
            let relative_output = if let Some(ref meta) = req.media_metadata {
                if !meta.title.trim().is_empty() {
                    metadata::format_output_relative_path_with_org(
                        meta,
                        req.naming_convention,
                        &extension,
                        folder_org,
                    )
                } else {
                    fallback_filename(&req.title_name, &extension)
                }
            } else {
                fallback_filename(&req.title_name, &extension)
            };

            let resolved_out_dir = if req.output_dir.trim().is_empty()
                || req.output_dir.trim().eq_ignore_ascii_case("auto")
            {
                defaults.output_dir.clone()
            } else {
                req.output_dir
            };
            let resolved_temp_dir = if req.temp_dir.trim().is_empty()
                || req.temp_dir.trim().eq_ignore_ascii_case("auto")
            {
                defaults.temp_dir.clone()
            } else {
                req.temp_dir
            };

            let candidate_output = PathBuf::from(&resolved_out_dir)
                .join(relative_output)
                .to_string_lossy()
                .into_owned();
            let output_path = ensure_unique_output_path(&candidate_output, &used_outputs);
            used_outputs.push(output_path.clone());

            let preset_label = if req.mode == PipelineMode::RipOnly {
                None
            } else {
                req.quality_profile
                    .as_ref()
                    .map(|p| p.label.clone())
                    .or_else(|| req.custom_preset_name.clone())
            };

            let initial_stage = req.mode.initial_stage();
            let job = QueueJob {
                id: job_id.clone(),
                mode: req.mode,
                disc_name: req.disc_name,
                drive_index: req.drive_index,
                title_id: req.title_id,
                source_file_path: req.source_file_path,
                title_name: req.title_name,
                preset_name: preset_label,
                temp_dir: resolved_temp_dir,
                temp_mkv_path: None,
                output_path,
                keep_temp_mkv: req.keep_temp_mkv,
                naming_convention: req.naming_convention,
                media_metadata: req.media_metadata,
                stage: initial_stage,
                progress: JobProgress {
                    stage: initial_stage,
                    percent: 0.0,
                    eta_seconds: None,
                    fps: None,
                    message: None,
                },
                error: None,
            };

            let spec = JobExecutionSpec {
                makemkv_cmd: makemkv_cmd.clone(),
                handbrake_cmd: handbrake_cmd.clone(),
                quality_profile: req.quality_profile,
                custom_preset_file: req.custom_preset_file,
                custom_preset_name: req.custom_preset_name,
                drive_device_path: req.drive_device_path,
                auto_eject_disc: req.auto_eject_disc,
                notify_on_complete: req.notify_on_complete,
                track_selection: req.track_selection,
            };

            self.specs.write().await.insert(job_id.clone(), spec);
            self.jobs.write().await.push(job.clone());
            self.emit_event(&PipelineEvent::JobUpdated { job: job.clone() });

            created_jobs.push(job);
        }

        // Enqueueing new work automatically resumes a restored/paused queue so jobs run right away.
        self.paused.store(false, Ordering::SeqCst);
        self.restored_pending_count.store(0, Ordering::Relaxed);
        self.persist_queue_async().await;
        self.emit_snapshot().await;
        self.wake_workers();

        Ok(created_jobs)
    }

    /// Reorders a pending (`QueuedForRip` or `QueuedForEncode`) job relative to other pending jobs.
    pub async fn move_job(
        &self,
        job_id: &str,
        action: ReorderAction,
    ) -> Result<Vec<QueueJob>, String> {
        let updated = {
            let mut jobs = self.jobs.write().await;
            let Some(cur) = jobs.iter().position(|j| j.id == job_id) else {
                return Err(format!("Queue job not found: {job_id}"));
            };
            if !jobs[cur].stage.is_pending() {
                return Err("Only queued (not yet running) jobs can be reordered.".to_string());
            }

            let pending_indices: Vec<usize> = jobs
                .iter()
                .enumerate()
                .filter_map(|(i, j)| j.stage.is_pending().then_some(i))
                .collect();

            let target = match action {
                ReorderAction::Top => pending_indices.first().copied(),
                ReorderAction::Bottom => pending_indices.last().copied(),
                ReorderAction::Up => pending_indices.iter().copied().rev().find(|&i| i < cur),
                ReorderAction::Down => pending_indices.iter().copied().find(|&i| i > cur),
            };

            if let Some(dest) = target {
                if dest != cur {
                    let item = jobs.remove(cur);
                    jobs.insert(dest, item);
                }
            }
            jobs.clone()
        };

        self.persist_queue_async().await;
        self.emit_snapshot().await;
        Ok(updated)
    }

    /// Moves a pending job directly to `target_index` (clamped within the pending jobs band).
    pub async fn reorder_job_to_index(
        &self,
        job_id: &str,
        target_index: usize,
    ) -> Result<Vec<QueueJob>, String> {
        let updated = {
            let mut jobs = self.jobs.write().await;
            let Some(cur) = jobs.iter().position(|j| j.id == job_id) else {
                return Err(format!("Queue job not found: {job_id}"));
            };
            if !jobs[cur].stage.is_pending() {
                return Err("Only queued (not yet running) jobs can be reordered.".to_string());
            }

            let first_pending = jobs.iter().position(|j| j.stage.is_pending()).unwrap_or(cur);
            let last_pending = jobs.iter().rposition(|j| j.stage.is_pending()).unwrap_or(cur);
            let clamped = target_index.clamp(first_pending, last_pending);

            if clamped != cur {
                let item = jobs.remove(cur);
                jobs.insert(clamped, item);
            }
            jobs.clone()
        };

        self.persist_queue_async().await;
        self.emit_snapshot().await;
        Ok(updated)
    }

    /// Removes a single queued or terminal job from the queue and persisted storage.
    pub async fn remove_job(&self, job_id: &str) -> Result<Vec<QueueJob>, String> {
        let updated = {
            let mut jobs = self.jobs.write().await;
            let Some(idx) = jobs.iter().position(|j| j.id == job_id) else {
                return Err(format!("Queue job not found: {job_id}"));
            };
            if jobs[idx].stage.is_running() {
                return Err("Cancel a running job before removing it from the queue.".to_string());
            }
            jobs.remove(idx);
            jobs.clone()
        };
        self.specs.write().await.remove(job_id);
        self.persist_queue_async().await;
        self.emit_snapshot().await;
        Ok(updated)
    }

    /// Pauses or resumes queue dispatch. When resuming, refreshes binary paths on any pending
    /// restored jobs so they pick up current CLI installations cleanly.
    pub async fn set_paused(
        &self,
        paused: bool,
        makemkv_cmd: Option<ResolvedCommand>,
        handbrake_cmd: Option<ResolvedCommand>,
    ) -> QueueStateSnapshot {
        if !paused {
            let pending_ids: Vec<String> = self
                .jobs
                .read()
                .await
                .iter()
                .filter(|j| j.stage.is_pending())
                .map(|j| j.id.clone())
                .collect();
            if !pending_ids.is_empty() {
                let mut specs = self.specs.write().await;
                for id in pending_ids {
                    if let Some(spec) = specs.get_mut(&id) {
                        if makemkv_cmd.is_some() {
                            spec.makemkv_cmd = makemkv_cmd.clone();
                        }
                        if handbrake_cmd.is_some() {
                            spec.handbrake_cmd = handbrake_cmd.clone();
                        }
                    }
                }
            }
            self.restored_pending_count.store(0, Ordering::Relaxed);
        }

        self.paused.store(paused, Ordering::SeqCst);
        self.persist_queue_async().await;
        let snapshot = self.queue_state().await;
        self.emit_snapshot().await;
        if !paused {
            self.wake_workers();
        }
        snapshot
    }

    /// Retries a `Failed` or `Cancelled` job by resetting its stage and waking the workers.
    pub async fn retry_job(
        &self,
        job_id: &str,
        makemkv_cmd: Option<ResolvedCommand>,
        handbrake_cmd: Option<ResolvedCommand>,
    ) -> Result<QueueJob, String> {
        let job = {
            let mut guard = self.jobs.write().await;
            let Some(job) = guard.iter_mut().find(|j| j.id == job_id) else {
                return Err(format!("Queue job not found: {job_id}"));
            };
            if !matches!(job.stage, JobStage::Failed | JobStage::Cancelled) {
                return Err("Only failed or cancelled jobs can be retried.".to_string());
            }

            let needs_rip = matches!(job.mode, PipelineMode::RipOnly | PipelineMode::RipAndEncode);
            let needs_encode =
                matches!(job.mode, PipelineMode::EncodeOnly | PipelineMode::RipAndEncode);
            if needs_rip && makemkv_cmd.is_none() {
                return Err(
                    "makemkvcon is not available. Check Settings → External CLIs.".to_string(),
                );
            }
            if needs_encode && handbrake_cmd.is_none() {
                return Err(
                    "HandBrakeCLI is not available. Check Settings → External CLIs.".to_string(),
                );
            }

            let initial = job.mode.initial_stage();
            job.stage = initial;
            job.error = None;
            job.temp_mkv_path = None;
            job.progress = JobProgress {
                stage: initial,
                percent: 0.0,
                eta_seconds: None,
                fps: None,
                message: Some("Retrying...".to_string()),
            };
            job.clone()
        };
        {
            let mut specs = self.specs.write().await;
            if let Some(spec) = specs.get_mut(job_id) {
                if makemkv_cmd.is_some() {
                    spec.makemkv_cmd = makemkv_cmd;
                }
                if handbrake_cmd.is_some() {
                    spec.handbrake_cmd = handbrake_cmd;
                }
            }
        }

        self.paused.store(false, Ordering::SeqCst);
        self.restored_pending_count.store(0, Ordering::Relaxed);
        self.persist_queue_async().await;

        self.emit_event(&PipelineEvent::LogLine {
            job_id: job.id.clone(),
            stage: job.stage,
            stream: "system".to_string(),
            line: "--- Retrying job ---".to_string(),
        });
        self.emit_event(&PipelineEvent::JobUpdated { job: job.clone() });
        self.emit_snapshot().await;
        self.wake_workers();

        Ok(job)
    }

    fn emit_event(&self, event: &PipelineEvent) {
        let handle = self
            .app_handle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(app) = handle {
            let _ = app.emit(PIPELINE_EVENT_NAME, event);
        }
    }

    async fn emit_snapshot(&self) {
        let jobs = self.jobs.read().await.clone();
        let paused = self.paused.load(Ordering::SeqCst);
        self.emit_event(&PipelineEvent::QueueSnapshot { jobs, paused });
    }

    /// Returns a snapshot of all jobs currently tracked by the queue.
    pub async fn list_jobs(&self) -> Vec<QueueJob> {
        self.jobs.read().await.clone()
    }

    /// Returns the full queue snapshot including pause & startup-recovery state.
    pub async fn queue_state(&self) -> QueueStateSnapshot {
        QueueStateSnapshot {
            jobs: self.jobs.read().await.clone(),
            paused: self.paused.load(Ordering::SeqCst),
            restored_pending_count: self.restored_pending_count.load(Ordering::Relaxed),
        }
    }

    /// Removes completed, cancelled, and failed jobs from the queue list.
    pub async fn clear_finished(&self) -> Vec<QueueJob> {
        let mut jobs = self.jobs.write().await;
        let removed_ids: Vec<String> = jobs
            .iter()
            .filter(|j| j.stage.is_terminal())
            .map(|j| j.id.clone())
            .collect();
        jobs.retain(|j| !j.stage.is_terminal());
        let remaining = jobs.clone();
        drop(jobs);

        let mut specs = self.specs.write().await;
        for id in removed_ids {
            specs.remove(&id);
        }
        drop(specs);

        self.persist_queue_async().await;
        self.emit_snapshot().await;
        remaining
    }

    /// Cancels a queued or active job and signals its running child process (if any) to stop.
    pub async fn cancel_job(&self, job_id: &str) -> bool {
        let signalled = self.processes.cancel(job_id);
        let mut updated_job = None;

        {
            let mut jobs = self.jobs.write().await;
            if let Some(job) = jobs.iter_mut().find(|item| item.id == job_id) {
                if !job.stage.is_terminal()
                    && job
                        .stage
                        .can_transition_to_for_mode(job.mode, JobStage::Cancelled)
                {
                    let _ = job.transition_to(JobStage::Cancelled);
                    updated_job = Some(job.clone());
                }
            }
        }

        if let Some(job) = updated_job {
            self.persist_queue_async().await;
            self.emit_event(&PipelineEvent::JobUpdated { job });
            return true;
        }
        signalled
    }

    /// Shared reference to the active child-process registry.
    pub fn process_registry(&self) -> Arc<ProcessRegistry> {
        Arc::clone(&self.processes)
    }

    /// Removes an intermediate MKV file after Stage 2 encoding completes.
    pub async fn cleanup_temp_file(path: &Path) -> Result<(), PipelineError> {
        if tokio::fs::try_exists(path)
            .await
            .map_err(|err| PipelineError::Io(err.to_string()))?
        {
            tokio::fs::remove_file(path)
                .await
                .map_err(|err| PipelineError::Io(err.to_string()))?;
        }
        Ok(())
    }

    /// Synchronous shutdown hook invoked on `tauri::RunEvent::Exit`.
    /// Persists the current queue snapshot and kills all active child processes cleanly.
    pub fn shutdown_sync(&self) {
        if let Some(path) = self
            .store_path
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            if let (Ok(jobs), Ok(specs)) = (self.jobs.try_read(), self.specs.try_read()) {
                let next_counter = self.job_counter.load(Ordering::Relaxed);
                let _ = write_queue_snapshot_sync(&path, &jobs, &specs, next_counter);
            }
        }
        self.processes.terminate_all_sync();
    }
}

fn write_queue_snapshot_sync(
    path: &Path,
    jobs: &[QueueJob],
    specs: &HashMap<String, JobExecutionSpec>,
    next_job_counter: u64,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Failed creating queue storage dir: {err}"))?;
    }
    let entries: Vec<PersistedQueueEntry> = jobs
        .iter()
        .filter_map(|job| {
            specs.get(&job.id).cloned().map(|spec| PersistedQueueEntry {
                job: job.clone(),
                spec,
            })
        })
        .collect();
    let payload = PersistedQueueFile {
        version: PERSISTED_QUEUE_VERSION,
        next_job_counter,
        entries,
    };
    let json = serde_json::to_string_pretty(&payload)
        .map_err(|err| format!("Failed serializing queue.json: {err}"))?;
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, json)
        .map_err(|err| format!("Failed writing temp queue file: {err}"))?;
    std::fs::rename(&tmp_path, path)
        .map_err(|err| format!("Failed committing queue.json: {err}"))?;
    Ok(())
}

fn staging_dir_for_job(job: &QueueJob) -> PathBuf {
    if job.mode == PipelineMode::RipOnly {
        let out_parent = Path::new(&job.output_path)
            .parent()
            .unwrap_or_else(|| Path::new(&job.temp_dir));
        out_parent.join(format!(".ripdaddy-staging-{}", job.id))
    } else {
        PathBuf::from(&job.temp_dir).join(&job.id)
    }
}

fn fallback_filename(title: &str, ext: &str) -> String {
    let clean = metadata::sanitize_filename(title);
    let base = if clean.is_empty() {
        "Untitled".to_string()
    } else {
        clean
    };
    format!("{base}.{ext}")
}

/// Atomically finds and transitions the first pending job matching `expected_stage` into `next_stage`.
async fn claim_next_pending_job(
    shared: &WorkerShared,
    expected_stage: JobStage,
    next_stage: JobStage,
) -> Option<QueueJob> {
    let claimed = {
        let mut guard = shared.jobs.write().await;
        let job = guard.iter_mut().find(|j| j.stage == expected_stage)?;
        let from = job.stage;
        if job.transition_to(next_stage).is_ok() {
            job.progress.message = None;
            Some((from, job.clone()))
        } else {
            None
        }
    };

    if let Some((from, job)) = claimed {
        shared.persist_queue().await;
        let _ = shared.app.emit(
            PIPELINE_EVENT_NAME,
            &PipelineEvent::StageChanged {
                job_id: job.id.clone(),
                from,
                to: next_stage,
            },
        );
        let _ = shared.app.emit(
            PIPELINE_EVENT_NAME,
            &PipelineEvent::JobUpdated { job: job.clone() },
        );
        Some(job)
    } else {
        None
    }
}

async fn run_rip_worker(mut wake_rx: watch::Receiver<u64>, shared: WorkerShared) {
    loop {
        let _ = wake_rx.borrow_and_update();

        let job_snapshot = if shared.paused.load(Ordering::SeqCst) {
            None
        } else {
            claim_next_pending_job(&shared, JobStage::QueuedForRip, JobStage::Ripping).await
        };

        let Some(job_snapshot) = job_snapshot else {
            if wake_rx.changed().await.is_err() {
                break;
            }
            continue;
        };

        let job_id = job_snapshot.id.clone();
        let Some(spec) = shared.specs.read().await.get(&job_id).cloned() else {
            mark_job_failed(
                &shared,
                &job_id,
                "Missing execution spec for rip job.",
                true,
            )
            .await;
            continue;
        };
        let Some(base_cmd) = spec.makemkv_cmd else {
            mark_job_failed(
                &shared,
                &job_id,
                "makemkvcon binary not configured.",
                spec.notify_on_complete,
            )
            .await;
            continue;
        };
        let (Some(drive_index), Some(title_id)) = (job_snapshot.drive_index, job_snapshot.title_id)
        else {
            mark_job_failed(
                &shared,
                &job_id,
                "Missing drive index or title ID.",
                spec.notify_on_complete,
            )
            .await;
            continue;
        };

        let staging_dir = staging_dir_for_job(&job_snapshot);
        if let Err(err) = tokio::fs::create_dir_all(&staging_dir).await {
            mark_job_failed(
                &shared,
                &job_id,
                format!("Failed creating staging directory {}: {err}", staging_dir.display()),
                spec.notify_on_complete,
            )
            .await;
            continue;
        }

        let mounted_cmd = base_cmd.with_flatpak_paths(&[staging_dir.as_path()]);
        let args = makemkv::build_rip_title_args(drive_index, title_id, &staging_dir);
        emit_job_log(
            &shared.app,
            &job_id,
            JobStage::Ripping,
            "system",
            format!("$ {} {}", mounted_cmd.program, args.join(" ")),
        );

        let (prog_tx, mut prog_rx) = mpsc::channel::<JobProgress>(32);
        let jobs_prog = Arc::clone(&shared.jobs);
        let app_prog = shared.app.clone();
        let id_prog = job_id.clone();
        let progress_task = tokio::spawn(async move {
            while let Some(prog) = prog_rx.recv().await {
                update_job_progress(&jobs_prog, &app_prog, &id_prog, prog).await;
            }
        });

        let mut last_emit = Instant::now() - PROGRESS_THROTTLE;
        let mut last_error_msg: Option<String> = None;
        let app_log = shared.app.clone();
        let id_log = job_id.clone();
        let run_result = spawn_and_stream_resolved(
            &mounted_cmd,
            &args,
            &job_id,
            JobStage::Ripping,
            Arc::clone(&shared.processes),
            |stream, line| {
                if stream == "stdout" {
                    match makemkv::parse_robot_line(&line) {
                        Ok(Some(RobotMessage::Prgv(prgv))) => {
                            let pct = prgv.total_percent();
                            if last_emit.elapsed() >= PROGRESS_THROTTLE || pct >= 99.9 {
                                last_emit = Instant::now();
                                let _ = prog_tx.try_send(JobProgress {
                                    stage: JobStage::Ripping,
                                    percent: pct,
                                    eta_seconds: None,
                                    fps: None,
                                    message: None,
                                });
                            }
                        }
                        Ok(Some(RobotMessage::Other { prefix, payload })) if prefix == "MSG" => {
                            let fields = makemkv::split_robot_fields(&payload);
                            if let Some(text) = fields.get(3).filter(|s| !s.trim().is_empty()) {
                                last_error_msg = Some(text.clone());
                                emit_job_log(
                                    &app_log,
                                    &id_log,
                                    JobStage::Ripping,
                                    "stdout",
                                    text.clone(),
                                );
                            }
                        }
                        _ => {}
                    }
                } else {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        last_error_msg = Some(trimmed.to_string());
                        emit_job_log(
                            &app_log,
                            &id_log,
                            JobStage::Ripping,
                            "stderr",
                            trimmed.to_string(),
                        );
                    }
                }
            },
        )
        .await;

        drop(prog_tx);
        let _ = progress_task.await;

        match run_result {
            Err(PipelineError::Cancelled) => {
                let _ = tokio::fs::remove_dir_all(&staging_dir).await;
                let _ = transition_and_emit(&shared, &job_id, JobStage::Cancelled).await;
                continue;
            }
            Err(err) => {
                let _ = tokio::fs::remove_dir_all(&staging_dir).await;
                mark_job_failed(&shared, &job_id, err.to_string(), spec.notify_on_complete).await;
                continue;
            }
            Ok(status) if !status.success() => {
                let _ = tokio::fs::remove_dir_all(&staging_dir).await;
                let detail = match last_error_msg {
                    Some(msg) => format!("makemkvcon failed (exit {:?}): {msg}", status.code()),
                    None => format!("makemkvcon exited with status {:?}", status.code()),
                };
                mark_job_failed(&shared, &job_id, detail, spec.notify_on_complete).await;
                continue;
            }
            Ok(_) => {}
        }

        if get_job_snapshot(&shared.jobs, &job_id)
            .await
            .map_or(true, |j| j.stage.is_terminal())
        {
            let _ = tokio::fs::remove_dir_all(&staging_dir).await;
            continue;
        }

        let Some(ripped_mkv) = find_largest_mkv(&staging_dir).await else {
            let _ = tokio::fs::remove_dir_all(&staging_dir).await;
            mark_job_failed(
                &shared,
                &job_id,
                "makemkvcon finished but no .mkv file was produced in staging directory.",
                spec.notify_on_complete,
            )
            .await;
            continue;
        };

        if spec.auto_eject_disc {
            let dev = spec.drive_device_path.clone();
            let disc_label = if job_snapshot.disc_name.trim().is_empty() {
                job_snapshot.title_name.clone()
            } else {
                job_snapshot.disc_name.clone()
            };
            let should_send_eject_alert = job_snapshot.mode == PipelineMode::RipAndEncode
                && spec.notify_on_complete
                && shared.notifications_enabled.load(Ordering::Relaxed);
            let _ = shared.app.emit(
                PIPELINE_EVENT_NAME,
                &PipelineEvent::DiscEjected {
                    job_id: job_id.clone(),
                    disc_name: disc_label.clone(),
                },
            );
            tokio::spawn(async move {
                makemkv::eject_optical_drive(dev.as_deref()).await;
                if should_send_eject_alert {
                    let _ = system::send_desktop_notification(
                        "RipDaddy · Disc Ready to Swap",
                        &format!(
                            "{disc_label} finished ripping and ejected. Insert the next disc while encoding continues."
                        ),
                    )
                    .await;
                }
            });
        }

        if job_snapshot.mode == PipelineMode::RipOnly {
            let dest = PathBuf::from(&job_snapshot.output_path);
            if let Err(err) = move_file_safe(&ripped_mkv, &dest).await {
                let _ = tokio::fs::remove_dir_all(&staging_dir).await;
                mark_job_failed(
                    &shared,
                    &job_id,
                    format!("Failed moving MKV: {err}"),
                    spec.notify_on_complete,
                )
                .await;
                continue;
            }
            let _ = tokio::fs::remove_dir_all(&staging_dir).await;
            if transition_and_emit(&shared, &job_id, JobStage::Completed).await {
                shared
                    .record_terminal_outcome(
                        &job_snapshot,
                        true,
                        spec.notify_on_complete,
                        spec.auto_eject_disc,
                    )
                    .await;
            }
        } else {
            // Stage 1 complete -> hand off to Stage 2 Encode Worker immediately
            {
                let mut guard = shared.jobs.write().await;
                if let Some(job) = guard.iter_mut().find(|j| j.id == job_id) {
                    job.temp_mkv_path = Some(ripped_mkv.to_string_lossy().into_owned());
                }
            }
            if transition_and_emit(&shared, &job_id, JobStage::QueuedForEncode).await {
                shared.wake_workers();
            }
        }
    }
}

async fn run_encode_worker(mut wake_rx: watch::Receiver<u64>, shared: WorkerShared) {
    loop {
        let _ = wake_rx.borrow_and_update();

        let job_snapshot = if shared.paused.load(Ordering::SeqCst) {
            None
        } else {
            claim_next_pending_job(&shared, JobStage::QueuedForEncode, JobStage::Encoding).await
        };

        let Some(job_snapshot) = job_snapshot else {
            if wake_rx.changed().await.is_err() {
                break;
            }
            continue;
        };

        let job_id = job_snapshot.id.clone();
        let Some(spec) = shared.specs.read().await.get(&job_id).cloned() else {
            mark_job_failed(
                &shared,
                &job_id,
                "Missing execution spec for encode job.",
                true,
            )
            .await;
            continue;
        };
        let Some(base_cmd) = spec.handbrake_cmd else {
            mark_job_failed(
                &shared,
                &job_id,
                "HandBrakeCLI binary not configured.",
                spec.notify_on_complete,
            )
            .await;
            continue;
        };

        let input_str = match job_snapshot.mode {
            PipelineMode::EncodeOnly => job_snapshot.source_file_path.clone(),
            PipelineMode::RipAndEncode => job_snapshot.temp_mkv_path.clone(),
            PipelineMode::RipOnly => None,
        };
        let Some(input_str) = input_str else {
            mark_job_failed(
                &shared,
                &job_id,
                "Missing input video path for encode.",
                spec.notify_on_complete,
            )
            .await;
            continue;
        };

        let input_path = PathBuf::from(&input_str);
        let output_path = PathBuf::from(&job_snapshot.output_path);
        if let Some(parent) = output_path.parent() {
            if let Err(err) = tokio::fs::create_dir_all(parent).await {
                mark_job_failed(
                    &shared,
                    &job_id,
                    format!("Failed creating output directory {}: {err}", parent.display()),
                    spec.notify_on_complete,
                )
                .await;
                continue;
            }
        }

        let args = if let (Some(custom_file), Some(custom_name)) = (
            spec.custom_preset_file.as_ref().filter(|s| !s.trim().is_empty()),
            spec.custom_preset_name.as_ref().filter(|s| !s.trim().is_empty()),
        ) {
            handbrake::build_encode_args(
                &input_path,
                &output_path,
                &EncodeOptions {
                    preset_name: custom_name.clone(),
                    custom_preset_file: Some(custom_file.clone()),
                    container: output_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("mkv")
                        .to_string(),
                    extra_args: Vec::new(),
                },
            )
        } else {
            let profile = spec.quality_profile.unwrap_or_else(|| {
                handbrake::recommended_quality_profiles()
                    .into_iter()
                    .find(|p| p.id == "p1080:local")
                    .expect("p1080:local default")
            });
            handbrake::build_quality_profile_encode_args_with_tracks(
                &input_path,
                &output_path,
                &profile,
                &spec.track_selection,
            )
        };

        let mut mount_dirs: Vec<&Path> = Vec::new();
        if let Some(in_parent) = input_path.parent() {
            mount_dirs.push(in_parent);
        }
        if let Some(out_parent) = output_path.parent() {
            mount_dirs.push(out_parent);
        }
        if let Some(ref custom_file) = spec.custom_preset_file {
            if let Some(preset_parent) = Path::new(custom_file).parent() {
                mount_dirs.push(preset_parent);
            }
        }
        let mounted_cmd = base_cmd.with_flatpak_paths(&mount_dirs);
        emit_job_log(
            &shared.app,
            &job_id,
            JobStage::Encoding,
            "system",
            format!("$ {} {}", mounted_cmd.program, args.join(" ")),
        );

        let (prog_tx, mut prog_rx) = mpsc::channel::<JobProgress>(32);
        let jobs_prog = Arc::clone(&shared.jobs);
        let app_prog = shared.app.clone();
        let id_prog = job_id.clone();
        let progress_task = tokio::spawn(async move {
            while let Some(prog) = prog_rx.recv().await {
                update_job_progress(&jobs_prog, &app_prog, &id_prog, prog).await;
            }
        });

        let mut parser = ProgressStreamParser::new();
        let mut last_emit = Instant::now() - PROGRESS_THROTTLE;
        let mut last_stderr: Option<String> = None;
        let app_log = shared.app.clone();
        let id_log = job_id.clone();
        let run_result = spawn_and_stream_resolved(
            &mounted_cmd,
            &args,
            &job_id,
            JobStage::Encoding,
            Arc::clone(&shared.processes),
            |stream, line| {
                if stream == "stdout" {
                    if let Ok(Some(enc)) = parser.push_line(&line) {
                        if last_emit.elapsed() >= PROGRESS_THROTTLE
                            || enc.state == HandBrakeState::Muxing
                            || enc.state == HandBrakeState::WorkDone
                        {
                            last_emit = Instant::now();
                            let msg = match enc.state {
                                HandBrakeState::Muxing => Some("Muxing container...".to_string()),
                                HandBrakeState::Scanning => Some("Scanning source...".to_string()),
                                _ => None,
                            };
                            let _ = prog_tx.try_send(JobProgress {
                                stage: JobStage::Encoding,
                                percent: enc.percent,
                                eta_seconds: enc.eta_seconds,
                                fps: enc.fps,
                                message: msg,
                            });
                        }
                    }
                } else {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        last_stderr = Some(trimmed.to_string());
                        emit_job_log(
                            &app_log,
                            &id_log,
                            JobStage::Encoding,
                            "stderr",
                            trimmed.to_string(),
                        );
                    }
                }
            },
        )
        .await;

        drop(prog_tx);
        let _ = progress_task.await;

        match run_result {
            Err(PipelineError::Cancelled) => {
                let _ = PipelineController::cleanup_temp_file(&output_path).await;
                let _ = transition_and_emit(&shared, &job_id, JobStage::Cancelled).await;
                continue;
            }
            Err(err) => {
                let _ = PipelineController::cleanup_temp_file(&output_path).await;
                mark_job_failed(&shared, &job_id, err.to_string(), spec.notify_on_complete).await;
                continue;
            }
            Ok(status) if !status.success() => {
                let _ = PipelineController::cleanup_temp_file(&output_path).await;
                let detail = match last_stderr {
                    Some(msg) => format!("HandBrakeCLI failed (exit {:?}): {msg}", status.code()),
                    None => format!("HandBrakeCLI exited with status {:?}", status.code()),
                };
                mark_job_failed(&shared, &job_id, detail, spec.notify_on_complete).await;
                continue;
            }
            Ok(_) => {}
        }

        if get_job_snapshot(&shared.jobs, &job_id)
            .await
            .map_or(true, |j| j.stage.is_terminal())
        {
            let _ = PipelineController::cleanup_temp_file(&output_path).await;
            continue;
        }

        // Stage 3: Temp MKV Cleanup (only for RipAndEncode when keep_temp_mkv is false)
        if job_snapshot.mode == PipelineMode::RipAndEncode && !job_snapshot.keep_temp_mkv {
            if let Some(ref temp_mkv) = job_snapshot.temp_mkv_path {
                let _ = transition_and_emit(&shared, &job_id, JobStage::CleaningTemp).await;
                let temp_path = Path::new(temp_mkv);
                if let Err(err) = PipelineController::cleanup_temp_file(temp_path).await {
                    mark_job_failed(
                        &shared,
                        &job_id,
                        format!("Temp cleanup failed: {err}"),
                        spec.notify_on_complete,
                    )
                    .await;
                    continue;
                }
                if let Some(temp_parent) = temp_path.parent() {
                    let _ = tokio::fs::remove_dir(temp_parent).await;
                }
                let _ = shared.app.emit(
                    PIPELINE_EVENT_NAME,
                    &PipelineEvent::TempCleaned {
                        job_id: job_id.clone(),
                        removed_path: temp_mkv.clone(),
                    },
                );
            }
        }

        if transition_and_emit(&shared, &job_id, JobStage::Completed).await {
            shared
                .record_terminal_outcome(&job_snapshot, true, spec.notify_on_complete, false)
                .await;
        }
    }
}

async fn get_job_snapshot(jobs: &Arc<RwLock<Vec<QueueJob>>>, job_id: &str) -> Option<QueueJob> {
    jobs.read()
        .await
        .iter()
        .find(|j| j.id == job_id)
        .cloned()
}

async fn transition_and_emit(shared: &WorkerShared, job_id: &str, next: JobStage) -> bool {
    let transition_info = {
        let mut guard = shared.jobs.write().await;
        if let Some(job) = guard.iter_mut().find(|j| j.id == job_id) {
            let from = job.stage;
            if job.transition_to(next).is_ok() {
                Some((from, job.clone()))
            } else {
                None
            }
        } else {
            None
        }
    };

    if let Some((from, job)) = transition_info {
        shared.persist_queue().await;
        let _ = shared.app.emit(
            PIPELINE_EVENT_NAME,
            &PipelineEvent::StageChanged {
                job_id: job_id.to_string(),
                from,
                to: next,
            },
        );
        let _ = shared
            .app
            .emit(PIPELINE_EVENT_NAME, &PipelineEvent::JobUpdated { job });
        true
    } else {
        false
    }
}

fn emit_job_log(
    app: &AppHandle,
    job_id: &str,
    stage: JobStage,
    stream: &str,
    line: String,
) {
    let _ = app.emit(
        PIPELINE_EVENT_NAME,
        &PipelineEvent::LogLine {
            job_id: job_id.to_string(),
            stage,
            stream: stream.to_string(),
            line,
        },
    );
}

async fn update_job_progress(
    jobs: &Arc<RwLock<Vec<QueueJob>>>,
    app: &AppHandle,
    job_id: &str,
    progress: JobProgress,
) {
    {
        let mut guard = jobs.write().await;
        if let Some(job) = guard.iter_mut().find(|j| j.id == job_id) {
            if job.stage == progress.stage {
                job.progress = progress.clone();
            } else {
                return;
            }
        } else {
            return;
        }
    }

    let _ = app.emit(
        PIPELINE_EVENT_NAME,
        &PipelineEvent::Progress {
            job_id: job_id.to_string(),
            progress,
        },
    );
}

async fn mark_job_failed(
    shared: &WorkerShared,
    job_id: &str,
    error_message: impl Into<String>,
    notify_allowed: bool,
) {
    let msg = error_message.into();
    let updated = {
        let mut guard = shared.jobs.write().await;
        if let Some(job) = guard.iter_mut().find(|j| j.id == job_id) {
            job.error = Some(msg);
            if job
                .stage
                .can_transition_to_for_mode(job.mode, JobStage::Failed)
            {
                let _ = job.transition_to(JobStage::Failed);
            } else {
                job.stage = JobStage::Failed;
                job.progress.stage = JobStage::Failed;
            }
            Some(job.clone())
        } else {
            None
        }
    };

    if let Some(job) = updated {
        shared.persist_queue().await;
        let _ = shared.app.emit(
            PIPELINE_EVENT_NAME,
            &PipelineEvent::JobUpdated { job: job.clone() },
        );
        shared
            .record_terminal_outcome(&job, false, notify_allowed, false)
            .await;
    }
}

async fn find_largest_mkv(dir: &Path) -> Option<PathBuf> {
    let mut entries = tokio::fs::read_dir(dir).await.ok()?;
    let mut best: Option<(u64, PathBuf)> = None;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let is_mkv = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mkv"));
        if is_mkv {
            let size = entry.metadata().await.map(|m| m.len()).unwrap_or(0);
            if best.as_ref().map_or(true, |(best_sz, _)| size >= *best_sz) {
                best = Some((size, path));
            }
        }
    }
    best.map(|(_, path)| path)
}

fn ensure_unique_output_path(candidate: &str, existing: &[String]) -> String {
    if !existing.iter().any(|p| p == candidate) {
        return candidate.to_string();
    }
    let path = Path::new(candidate);
    let parent = path.parent();
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let ext = path.extension().and_then(|e| e.to_str());

    let mut suffix = 2u32;
    loop {
        let next_name = match ext {
            Some(e) => format!("{stem} ({suffix}).{e}"),
            None => format!("{stem} ({suffix})"),
        };
        let next_path = match parent {
            Some(dir) if !dir.as_os_str().is_empty() => dir.join(&next_name),
            _ => PathBuf::from(&next_name),
        };
        let next_str = next_path.to_string_lossy().into_owned();
        if !existing.iter().any(|p| p == &next_str) {
            return next_str;
        }
        suffix += 1;
    }
}

async fn move_file_safe(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    if let Some(parent) = dst.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    match tokio::fs::rename(src, dst).await {
        Ok(()) => Ok(()),
        Err(_) => {
            tokio::fs::copy(src, dst).await?;
            tokio::fs::remove_file(src).await?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_job(mode: PipelineMode) -> QueueJob {
        let initial = mode.initial_stage();
        QueueJob {
            id: "job-1".to_string(),
            mode,
            disc_name: "DISC_01".to_string(),
            drive_index: (mode != PipelineMode::EncodeOnly).then_some(0),
            title_id: (mode != PipelineMode::EncodeOnly).then_some(0),
            source_file_path: (mode == PipelineMode::EncodeOnly)
                .then(|| "/media/source.mkv".to_string()),
            title_name: "Title 0".to_string(),
            preset_name: (mode != PipelineMode::RipOnly)
                .then(|| "1080p Local (Blu-ray Quality)".to_string()),
            temp_dir: "/tmp/ripdaddy".to_string(),
            temp_mkv_path: None,
            output_path: "/media/output.mkv".to_string(),
            keep_temp_mkv: false,
            naming_convention: NamingConvention::Plex,
            media_metadata: None,
            stage: initial,
            progress: JobProgress {
                stage: initial,
                percent: 0.0,
                eta_seconds: None,
                fps: None,
                message: None,
            },
            error: None,
        }
    }

    fn sample_spec() -> JobExecutionSpec {
        JobExecutionSpec {
            makemkv_cmd: None,
            handbrake_cmd: Some(ResolvedCommand {
                program: "/usr/bin/HandBrakeCLI".to_string(),
                prefix_args: Vec::new(),
            }),
            quality_profile: None,
            custom_preset_file: None,
            custom_preset_name: None,
            drive_device_path: None,
            auto_eject_disc: false,
            notify_on_complete: true,
            track_selection: TrackSelectionOptions::default(),
        }
    }

    #[test]
    fn valid_rip_and_encode_pipeline_transitions() {
        let mut job = sample_job(PipelineMode::RipAndEncode);
        assert!(job.transition_to(JobStage::Ripping).is_ok());
        assert!(job.transition_to(JobStage::QueuedForEncode).is_ok());
        assert!(job.transition_to(JobStage::Encoding).is_ok());
        assert!(job.transition_to(JobStage::CleaningTemp).is_ok());
        assert!(job.transition_to(JobStage::Completed).is_ok());
        assert_eq!(job.progress.percent, 100.0);
        assert!(job.stage.is_terminal());
    }

    #[test]
    fn valid_rip_only_and_encode_only_transitions() {
        let mut rip = sample_job(PipelineMode::RipOnly);
        assert!(rip.transition_to(JobStage::Ripping).is_ok());
        assert!(rip.transition_to(JobStage::QueuedForEncode).is_err());
        assert!(rip.transition_to(JobStage::Completed).is_ok());

        let mut enc = sample_job(PipelineMode::EncodeOnly);
        assert!(enc.transition_to(JobStage::Ripping).is_err());
        assert!(enc.transition_to(JobStage::Encoding).is_ok());
        assert!(enc.transition_to(JobStage::CleaningTemp).is_err());
        assert!(enc.transition_to(JobStage::Completed).is_ok());
    }

    #[test]
    fn process_registry_cancels_and_clears_entries() {
        let registry = ProcessRegistry::new();
        let rx = registry
            .register("job-42".to_string(), JobStage::Ripping, None)
            .expect("register");
        assert_eq!(registry.active_count(), 1);
        assert!(!*rx.borrow());

        assert!(registry.cancel("job-42"));
        assert!(*rx.borrow());

        registry.unregister("job-42");
        assert_eq!(registry.active_count(), 0);
    }

    #[test]
    fn deduplicates_colliding_output_paths_in_batch() {
        let existing = vec!["/videos/The Matrix (1999).mkv".to_string()];
        let unique = ensure_unique_output_path("/videos/The Matrix (1999).mkv", &existing);
        assert_eq!(unique, "/videos/The Matrix (1999) (2).mkv");

        let existing2 = vec![
            "/videos/The Matrix (1999).mkv".to_string(),
            "/videos/The Matrix (1999) (2).mkv".to_string(),
        ];
        let unique2 = ensure_unique_output_path("/videos/The Matrix (1999).mkv", &existing2);
        assert_eq!(unique2, "/videos/The Matrix (1999) (3).mkv");
    }

    #[tokio::test]
    async fn reorders_pending_jobs_without_displacing_running_jobs() {
        let ctrl = PipelineController::new();
        let mut j0 = sample_job(PipelineMode::EncodeOnly);
        j0.id = "job-running".to_string();
        j0.stage = JobStage::Encoding;

        let mut j1 = sample_job(PipelineMode::EncodeOnly);
        j1.id = "job-a".to_string();
        let mut j2 = sample_job(PipelineMode::EncodeOnly);
        j2.id = "job-b".to_string();
        let mut j3 = sample_job(PipelineMode::EncodeOnly);
        j3.id = "job-urgent".to_string();

        {
            let mut guard = ctrl.jobs.write().await;
            *guard = vec![j0, j1, j2, j3];
        }

        // Move urgent job to top of pending queue (stays right after running job-running)
        let after_top = ctrl
            .move_job("job-urgent", ReorderAction::Top)
            .await
            .expect("move top");
        let ids: Vec<&str> = after_top.iter().map(|j| j.id.as_str()).collect();
        assert_eq!(ids, vec!["job-running", "job-urgent", "job-a", "job-b"]);

        // Cannot reorder running job
        assert!(ctrl.move_job("job-running", ReorderAction::Down).await.is_err());

        // Move job-b up one slot
        let after_up = ctrl
            .move_job("job-b", ReorderAction::Up)
            .await
            .expect("move up");
        let ids2: Vec<&str> = after_up.iter().map(|j| j.id.as_str()).collect();
        assert_eq!(ids2, vec!["job-running", "job-urgent", "job-b", "job-a"]);
    }

    #[tokio::test]
    async fn persists_and_recovers_interrupted_jobs_across_restarts() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temp_file = std::env::temp_dir().join(format!("ripdaddy-queue-test-{stamp}.json"));

        let mut active_enc = sample_job(PipelineMode::EncodeOnly);
        active_enc.id = "job-10".to_string();
        active_enc.stage = JobStage::Encoding;
        active_enc.output_path = std::env::temp_dir()
            .join(format!("ripdaddy-nonexistent-out-{stamp}.mkv"))
            .to_string_lossy()
            .into_owned();

        let mut queued_enc = sample_job(PipelineMode::EncodeOnly);
        queued_enc.id = "job-11".to_string();
        queued_enc.stage = JobStage::QueuedForEncode;

        let mut specs = HashMap::new();
        specs.insert(active_enc.id.clone(), sample_spec());
        specs.insert(queued_enc.id.clone(), sample_spec());

        write_queue_snapshot_sync(&temp_file, &[active_enc, queued_enc], &specs, 12)
            .expect("write snapshot");

        let restored_ctrl = PipelineController::new();
        restored_ctrl.set_store_path(temp_file.clone());
        let pending = restored_ctrl.restore_persisted_queue_sync();
        assert_eq!(pending, 2);

        let state = restored_ctrl.queue_state().await;
        assert!(state.paused);
        assert_eq!(state.restored_pending_count, 2);
        assert_eq!(state.jobs.len(), 2);
        assert_eq!(state.jobs[0].stage, JobStage::QueuedForEncode);
        assert_eq!(state.jobs[1].stage, JobStage::QueuedForEncode);

        let _ = std::fs::remove_file(&temp_file);
    }
}
