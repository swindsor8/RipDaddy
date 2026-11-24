//! Decoupled two-stage rip-and-encode queue state machine, live Tokio workers,
//! Tauri event emission, and child-process safety registry.
//!
//! Supported workflow modes (`PipelineMode`):
//! - `RipOnly`:      `QueuedForRip -> Ripping -> Completed`
//! - `EncodeOnly`:   `QueuedForEncode -> Encoding -> Completed`
//! - `RipAndEncode`: `QueuedForRip -> Ripping -> QueuedForEncode -> Encoding -> CleaningTemp -> Completed`
//!
//! Stage 1 (Rip Worker / `makemkvcon`) and Stage 2 (Encode Worker / `HandBrakeCLI`)
//! run on independent bounded `tokio::sync::mpsc` queues so optical disc ripping and
//! video encoding overlap smoothly without blocking each other.

use crate::detect::{self, ResolvedCommand};
use crate::handbrake::{
    self, EncodeOptions, HandBrakeState, ProgressStreamParser, QualityProfile,
    TrackSelectionOptions,
};
use crate::makemkv::{self, RobotMessage};
use crate::metadata::{self, FolderOrganization, MediaMetadata, NamingConvention};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, watch, RwLock};

/// Default bounded channel capacity for each decoupled worker queue.
const WORKER_QUEUE_CAPACITY: usize = 64;
const PROGRESS_THROTTLE: Duration = Duration::from_millis(200);
pub const PIPELINE_EVENT_NAME: &str = "pipeline-event";

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

    /// Returns `true` if no further transitions are permitted from this stage.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            JobStage::Completed | JobStage::Cancelled | JobStage::Failed
        )
    }
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
    pub naming_convention: NamingConvention,
    pub folder_organization: Option<FolderOrganization>,
    pub media_metadata: Option<MediaMetadata>,
    #[serde(default)]
    pub track_selection: TrackSelectionOptions,
}

/// Internal execution metadata retained alongside each `QueueJob`.
#[derive(Debug, Clone)]
struct JobExecutionSpec {
    makemkv_cmd: Option<ResolvedCommand>,
    handbrake_cmd: Option<ResolvedCommand>,
    quality_profile: Option<QualityProfile>,
    custom_preset_file: Option<String>,
    custom_preset_name: Option<String>,
    drive_device_path: Option<String>,
    auto_eject_disc: bool,
    track_selection: TrackSelectionOptions,
}

/// Structured events streamed from Rust to the Svelte frontend via Tauri events / `Channel`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PipelineEvent {
    #[serde(rename_all = "camelCase")]
    JobUpdated { job: QueueJob },
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

/// Decoupled receiver ends owned by the Rip Worker and Encode Worker tasks once spawned.
pub struct WorkerChannels {
    pub rip_rx: mpsc::Receiver<String>,
    pub encode_rx: mpsc::Receiver<String>,
}

/// Tauri-managed state coordinating the decoupled Rip -> Encode -> Cleanup pipeline.
pub struct PipelineController {
    jobs: Arc<RwLock<Vec<QueueJob>>>,
    specs: Arc<RwLock<HashMap<String, JobExecutionSpec>>>,
    processes: Arc<ProcessRegistry>,
    rip_tx: mpsc::Sender<String>,
    encode_tx: mpsc::Sender<String>,
    worker_channels: Mutex<Option<WorkerChannels>>,
    app_handle: Mutex<Option<AppHandle>>,
    job_counter: AtomicU64,
}

impl Default for PipelineController {
    fn default() -> Self {
        Self::new()
    }
}

impl PipelineController {
    pub fn new() -> Self {
        let (rip_tx, rip_rx) = mpsc::channel(WORKER_QUEUE_CAPACITY);
        let (encode_tx, encode_rx) = mpsc::channel(WORKER_QUEUE_CAPACITY);

        Self {
            jobs: Arc::new(RwLock::new(Vec::new())),
            specs: Arc::new(RwLock::new(HashMap::new())),
            processes: Arc::new(ProcessRegistry::new()),
            rip_tx,
            encode_tx,
            worker_channels: Mutex::new(Some(WorkerChannels { rip_rx, encode_rx })),
            app_handle: Mutex::new(None),
            job_counter: AtomicU64::new(1),
        }
    }

    /// Spawns the independent Stage 1 (Rip) and Stage 2 (Encode) Tokio worker tasks.
    /// Safe to call once during Tauri `.setup(...)`.
    pub fn start_workers(&self, app_handle: AppHandle) {
        {
            let mut slot = self.app_handle.lock().unwrap_or_else(|e| e.into_inner());
            *slot = Some(app_handle.clone());
        }

        let Some(channels) = self.take_worker_channels() else {
            return;
        };

        // Stage 1: Sequential Optical Disc Rip Worker (makemkvcon)
        let rip_jobs = Arc::clone(&self.jobs);
        let rip_specs = Arc::clone(&self.specs);
        let rip_processes = Arc::clone(&self.processes);
        let encode_tx = self.encode_tx.clone();
        let rip_app = app_handle.clone();
        tauri::async_runtime::spawn(async move {
            run_rip_worker(
                channels.rip_rx,
                encode_tx,
                rip_jobs,
                rip_specs,
                rip_processes,
                rip_app,
            )
            .await;
        });

        // Stage 2: Independent Encode + Temp Cleanup Worker (HandBrakeCLI)
        let enc_jobs = Arc::clone(&self.jobs);
        let enc_specs = Arc::clone(&self.specs);
        let enc_processes = Arc::clone(&self.processes);
        let enc_app = app_handle;
        tauri::async_runtime::spawn(async move {
            run_encode_worker(
                channels.encode_rx,
                enc_jobs,
                enc_specs,
                enc_processes,
                enc_app,
            )
            .await;
        });
    }

    /// Validates, constructs, persists in memory, and dispatches one or more pipeline jobs.
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
                track_selection: req.track_selection,
            };

            self.specs.write().await.insert(job_id.clone(), spec);
            self.jobs.write().await.push(job.clone());
            self.emit_event(&PipelineEvent::JobUpdated { job: job.clone() });

            let send_res = match job.mode {
                PipelineMode::RipOnly | PipelineMode::RipAndEncode => {
                    self.rip_tx.send(job_id.clone()).await
                }
                PipelineMode::EncodeOnly => self.encode_tx.send(job_id.clone()).await,
            };

            if send_res.is_err() {
                return Err("Pipeline worker channel is closed.".to_string());
            }

            created_jobs.push(job);
        }

        Ok(created_jobs)
    }

    /// Retries a `Failed` or `Cancelled` job by resetting its stage and re-dispatching it onto
    /// the appropriate Tokio worker with refreshed external CLI paths.
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

        self.emit_event(&PipelineEvent::LogLine {
            job_id: job.id.clone(),
            stage: job.stage,
            stream: "system".to_string(),
            line: "--- Retrying job ---".to_string(),
        });
        self.emit_event(&PipelineEvent::JobUpdated { job: job.clone() });

        let send_res = match job.mode {
            PipelineMode::RipOnly | PipelineMode::RipAndEncode => {
                self.rip_tx.send(job.id.clone()).await
            }
            PipelineMode::EncodeOnly => self.encode_tx.send(job.id.clone()).await,
        };
        if send_res.is_err() {
            return Err("Pipeline worker channel is closed.".to_string());
        }

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

    /// Returns a snapshot of all jobs currently tracked by the queue.
    pub async fn list_jobs(&self) -> Vec<QueueJob> {
        self.jobs.read().await.clone()
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
            self.emit_event(&PipelineEvent::JobUpdated { job });
            return true;
        }
        signalled
    }

    /// Takes ownership of the decoupled worker queue receivers when starting background workers.
    pub fn take_worker_channels(&self) -> Option<WorkerChannels> {
        self.worker_channels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    /// Sender handle for Stage 1 (Rip Worker).
    pub fn rip_sender(&self) -> mpsc::Sender<String> {
        self.rip_tx.clone()
    }

    /// Sender handle for Stage 2 (Encode Worker).
    pub fn encode_sender(&self) -> mpsc::Sender<String> {
        self.encode_tx.clone()
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
    pub fn shutdown_sync(&self) {
        self.processes.terminate_all_sync();
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

async fn run_rip_worker(
    mut rip_rx: mpsc::Receiver<String>,
    encode_tx: mpsc::Sender<String>,
    jobs: Arc<RwLock<Vec<QueueJob>>>,
    specs: Arc<RwLock<HashMap<String, JobExecutionSpec>>>,
    processes: Arc<ProcessRegistry>,
    app: AppHandle,
) {
    while let Some(job_id) = rip_rx.recv().await {
        let Some(job_snapshot) = get_job_snapshot(&jobs, &job_id).await else {
            continue;
        };
        if job_snapshot.stage.is_terminal() {
            continue;
        }
        let Some(spec) = specs.read().await.get(&job_id).cloned() else {
            mark_job_failed(&jobs, &app, &job_id, "Missing execution spec for rip job.").await;
            continue;
        };
        let Some(base_cmd) = spec.makemkv_cmd else {
            mark_job_failed(&jobs, &app, &job_id, "makemkvcon binary not configured.").await;
            continue;
        };
        let (Some(drive_index), Some(title_id)) = (job_snapshot.drive_index, job_snapshot.title_id)
        else {
            mark_job_failed(&jobs, &app, &job_id, "Missing drive index or title ID.").await;
            continue;
        };

        if !transition_and_emit(&jobs, &app, &job_id, JobStage::Ripping).await {
            continue;
        }

        // Stage inside the output folder's parent for RipOnly (same-FS instant rename),
        // or inside disk-backed `temp_dir/<job_id>` for RipAndEncode.
        let staging_dir = if job_snapshot.mode == PipelineMode::RipOnly {
            let out_parent = Path::new(&job_snapshot.output_path)
                .parent()
                .unwrap_or_else(|| Path::new(&job_snapshot.temp_dir));
            out_parent.join(format!(".ripdaddy-staging-{job_id}"))
        } else {
            PathBuf::from(&job_snapshot.temp_dir).join(&job_id)
        };

        if let Err(err) = tokio::fs::create_dir_all(&staging_dir).await {
            mark_job_failed(
                &jobs,
                &app,
                &job_id,
                format!("Failed creating staging directory {}: {err}", staging_dir.display()),
            )
            .await;
            continue;
        }

        let mounted_cmd = base_cmd.with_flatpak_paths(&[staging_dir.as_path()]);
        let args = makemkv::build_rip_title_args(drive_index, title_id, &staging_dir);
        emit_job_log(
            &app,
            &job_id,
            JobStage::Ripping,
            "system",
            format!("$ {} {}", mounted_cmd.program, args.join(" ")),
        );

        let (prog_tx, mut prog_rx) = mpsc::channel::<JobProgress>(32);
        let jobs_prog = Arc::clone(&jobs);
        let app_prog = app.clone();
        let id_prog = job_id.clone();
        let progress_task = tokio::spawn(async move {
            while let Some(prog) = prog_rx.recv().await {
                update_job_progress(&jobs_prog, &app_prog, &id_prog, prog).await;
            }
        });

        let mut last_emit = Instant::now() - PROGRESS_THROTTLE;
        let mut last_error_msg: Option<String> = None;
        let app_log = app.clone();
        let id_log = job_id.clone();
        let run_result = spawn_and_stream_resolved(
            &mounted_cmd,
            &args,
            &job_id,
            JobStage::Ripping,
            Arc::clone(&processes),
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
                                emit_job_log(&app_log, &id_log, JobStage::Ripping, "stdout", text.clone());
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
                let _ = transition_and_emit(&jobs, &app, &job_id, JobStage::Cancelled).await;
                continue;
            }
            Err(err) => {
                let _ = tokio::fs::remove_dir_all(&staging_dir).await;
                mark_job_failed(&jobs, &app, &job_id, err.to_string()).await;
                continue;
            }
            Ok(status) if !status.success() => {
                let _ = tokio::fs::remove_dir_all(&staging_dir).await;
                let detail = match last_error_msg {
                    Some(msg) => format!("makemkvcon failed (exit {:?}): {msg}", status.code()),
                    None => format!("makemkvcon exited with status {:?}", status.code()),
                };
                mark_job_failed(&jobs, &app, &job_id, detail).await;
                continue;
            }
            Ok(_) => {}
        }

        if get_job_snapshot(&jobs, &job_id)
            .await
            .map_or(true, |j| j.stage.is_terminal())
        {
            let _ = tokio::fs::remove_dir_all(&staging_dir).await;
            continue;
        }

        let Some(ripped_mkv) = find_largest_mkv(&staging_dir).await else {
            let _ = tokio::fs::remove_dir_all(&staging_dir).await;
            mark_job_failed(
                &jobs,
                &app,
                &job_id,
                "makemkvcon finished but no .mkv file was produced in staging directory.",
            )
            .await;
            continue;
        };

        if spec.auto_eject_disc {
            let dev = spec.drive_device_path.clone();
            tokio::spawn(async move {
                makemkv::eject_optical_drive(dev.as_deref()).await;
            });
        }

        if job_snapshot.mode == PipelineMode::RipOnly {
            let dest = PathBuf::from(&job_snapshot.output_path);
            if let Err(err) = move_file_safe(&ripped_mkv, &dest).await {
                let _ = tokio::fs::remove_dir_all(&staging_dir).await;
                mark_job_failed(&jobs, &app, &job_id, format!("Failed moving MKV: {err}")).await;
                continue;
            }
            let _ = tokio::fs::remove_dir_all(&staging_dir).await;
            let _ = transition_and_emit(&jobs, &app, &job_id, JobStage::Completed).await;
        } else {
            // Stage 1 complete -> hand off to Stage 2 Encode Worker immediately
            {
                let mut guard = jobs.write().await;
                if let Some(job) = guard.iter_mut().find(|j| j.id == job_id) {
                    job.temp_mkv_path = Some(ripped_mkv.to_string_lossy().into_owned());
                }
            }
            if transition_and_emit(&jobs, &app, &job_id, JobStage::QueuedForEncode).await {
                let _ = encode_tx.send(job_id.clone()).await;
            }
        }
    }
}

async fn run_encode_worker(
    mut encode_rx: mpsc::Receiver<String>,
    jobs: Arc<RwLock<Vec<QueueJob>>>,
    specs: Arc<RwLock<HashMap<String, JobExecutionSpec>>>,
    processes: Arc<ProcessRegistry>,
    app: AppHandle,
) {
    while let Some(job_id) = encode_rx.recv().await {
        let Some(job_snapshot) = get_job_snapshot(&jobs, &job_id).await else {
            continue;
        };
        if job_snapshot.stage.is_terminal() {
            continue;
        }
        let Some(spec) = specs.read().await.get(&job_id).cloned() else {
            mark_job_failed(&jobs, &app, &job_id, "Missing execution spec for encode job.").await;
            continue;
        };
        let Some(base_cmd) = spec.handbrake_cmd else {
            mark_job_failed(&jobs, &app, &job_id, "HandBrakeCLI binary not configured.").await;
            continue;
        };

        let input_str = match job_snapshot.mode {
            PipelineMode::EncodeOnly => job_snapshot.source_file_path.clone(),
            PipelineMode::RipAndEncode => job_snapshot.temp_mkv_path.clone(),
            PipelineMode::RipOnly => None,
        };
        let Some(input_str) = input_str else {
            mark_job_failed(&jobs, &app, &job_id, "Missing input video path for encode.").await;
            continue;
        };

        let input_path = PathBuf::from(&input_str);
        let output_path = PathBuf::from(&job_snapshot.output_path);
        if let Some(parent) = output_path.parent() {
            if let Err(err) = tokio::fs::create_dir_all(parent).await {
                mark_job_failed(
                    &jobs,
                    &app,
                    &job_id,
                    format!("Failed creating output directory {}: {err}", parent.display()),
                )
                .await;
                continue;
            }
        }

        if !transition_and_emit(&jobs, &app, &job_id, JobStage::Encoding).await {
            continue;
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
            &app,
            &job_id,
            JobStage::Encoding,
            "system",
            format!("$ {} {}", mounted_cmd.program, args.join(" ")),
        );

        let (prog_tx, mut prog_rx) = mpsc::channel::<JobProgress>(32);
        let jobs_prog = Arc::clone(&jobs);
        let app_prog = app.clone();
        let id_prog = job_id.clone();
        let progress_task = tokio::spawn(async move {
            while let Some(prog) = prog_rx.recv().await {
                update_job_progress(&jobs_prog, &app_prog, &id_prog, prog).await;
            }
        });

        let mut parser = ProgressStreamParser::new();
        let mut last_emit = Instant::now() - PROGRESS_THROTTLE;
        let mut last_stderr: Option<String> = None;
        let app_log = app.clone();
        let id_log = job_id.clone();
        let run_result = spawn_and_stream_resolved(
            &mounted_cmd,
            &args,
            &job_id,
            JobStage::Encoding,
            Arc::clone(&processes),
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
                let _ = transition_and_emit(&jobs, &app, &job_id, JobStage::Cancelled).await;
                continue;
            }
            Err(err) => {
                let _ = PipelineController::cleanup_temp_file(&output_path).await;
                mark_job_failed(&jobs, &app, &job_id, err.to_string()).await;
                continue;
            }
            Ok(status) if !status.success() => {
                let _ = PipelineController::cleanup_temp_file(&output_path).await;
                let detail = match last_stderr {
                    Some(msg) => format!("HandBrakeCLI failed (exit {:?}): {msg}", status.code()),
                    None => format!("HandBrakeCLI exited with status {:?}", status.code()),
                };
                mark_job_failed(&jobs, &app, &job_id, detail).await;
                continue;
            }
            Ok(_) => {}
        }

        if get_job_snapshot(&jobs, &job_id)
            .await
            .map_or(true, |j| j.stage.is_terminal())
        {
            let _ = PipelineController::cleanup_temp_file(&output_path).await;
            continue;
        }

        // Stage 3: Temp MKV Cleanup (only for RipAndEncode when keep_temp_mkv is false)
        if job_snapshot.mode == PipelineMode::RipAndEncode && !job_snapshot.keep_temp_mkv {
            if let Some(ref temp_mkv) = job_snapshot.temp_mkv_path {
                let _ = transition_and_emit(&jobs, &app, &job_id, JobStage::CleaningTemp).await;
                let temp_path = Path::new(temp_mkv);
                if let Err(err) = PipelineController::cleanup_temp_file(temp_path).await {
                    mark_job_failed(&jobs, &app, &job_id, format!("Temp cleanup failed: {err}"))
                        .await;
                    continue;
                }
                if let Some(temp_parent) = temp_path.parent() {
                    let _ = tokio::fs::remove_dir(temp_parent).await;
                }
                let _ = app.emit(
                    PIPELINE_EVENT_NAME,
                    &PipelineEvent::TempCleaned {
                        job_id: job_id.clone(),
                        removed_path: temp_mkv.clone(),
                    },
                );
            }
        }

        let _ = transition_and_emit(&jobs, &app, &job_id, JobStage::Completed).await;
    }
}

async fn get_job_snapshot(jobs: &Arc<RwLock<Vec<QueueJob>>>, job_id: &str) -> Option<QueueJob> {
    jobs.read()
        .await
        .iter()
        .find(|j| j.id == job_id)
        .cloned()
}

async fn transition_and_emit(
    jobs: &Arc<RwLock<Vec<QueueJob>>>,
    app: &AppHandle,
    job_id: &str,
    next: JobStage,
) -> bool {
    let transition_info = {
        let mut guard = jobs.write().await;
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
        let _ = app.emit(
            PIPELINE_EVENT_NAME,
            &PipelineEvent::StageChanged {
                job_id: job_id.to_string(),
                from,
                to: next,
            },
        );
        let _ = app.emit(PIPELINE_EVENT_NAME, &PipelineEvent::JobUpdated { job });
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
    jobs: &Arc<RwLock<Vec<QueueJob>>>,
    app: &AppHandle,
    job_id: &str,
    error_message: impl Into<String>,
) {
    let msg = error_message.into();
    let updated = {
        let mut guard = jobs.write().await;
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
        let _ = app.emit(PIPELINE_EVENT_NAME, &PipelineEvent::JobUpdated { job });
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
}
