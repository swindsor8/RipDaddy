//! Decoupled two-stage rip-and-encode queue state machine and child-process safety registry.
//!
//! Supported workflow modes (`PipelineMode`):
//! - `RipOnly`:      `QueuedForRip -> Ripping -> Completed`
//! - `EncodeOnly`:   `QueuedForEncode -> Encoding -> Completed`
//! - `RipAndEncode`: `QueuedForRip -> Ripping -> QueuedForEncode -> Encoding -> CleaningTemp -> Completed`
//!
//! Stage 1 (Rip Worker / `makemkvcon`) and Stage 2 (Encode Worker / `HandBrakeCLI`)
//! communicate over separate bounded `tokio::sync::mpsc` queues so disc ripping and
//! software/hardware encoding never block each other.

use crate::metadata::{MediaMetadata, NamingConvention};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, watch, RwLock};

/// Default bounded channel capacity for each decoupled worker queue.
const WORKER_QUEUE_CAPACITY: usize = 64;

/// Controls which pipeline stages a job executes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PipelineMode {
    /// Stage 1 only (`makemkvcon` -> final output directory).
    RipOnly,
    /// Stage 2 only (local video file -> `HandBrakeCLI` -> final output directory).
    EncodeOnly,
    /// Full decoupled pipeline (`makemkvcon` -> temp MKV -> `HandBrakeCLI` -> cleanup).
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

/// Structured events streamed from Rust to the Svelte frontend via `tauri::ipc::Channel`.
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

/// Spawns an external CLI binary under `tokio`, streams `stdout` and `stderr` line-by-line,
/// and guarantees immediate termination + reaping if cancelled or dropped.
pub async fn spawn_and_stream<F>(
    program: &Path,
    args: &[String],
    job_id: &str,
    stage: JobStage,
    registry: Arc<ProcessRegistry>,
    mut on_line: F,
) -> Result<ExitStatus, PipelineError>
where
    F: FnMut(&'static str, String),
{
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_platform_command(&mut cmd);

    let mut child = cmd
        .spawn()
        .map_err(|err| PipelineError::ProcessSpawnFailed(format!("{}: {err}", program.display())))?;

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

/// Decoupled receiver ends owned by the Rip Worker and Encode Worker tasks once spawned.
pub struct WorkerChannels {
    pub rip_rx: mpsc::Receiver<String>,
    pub encode_rx: mpsc::Receiver<String>,
}

/// Tauri-managed state coordinating the two-stage rip -> encode -> cleanup pipeline.
pub struct PipelineController {
    jobs: Arc<RwLock<Vec<QueueJob>>>,
    processes: Arc<ProcessRegistry>,
    rip_tx: mpsc::Sender<String>,
    encode_tx: mpsc::Sender<String>,
    worker_channels: Mutex<Option<WorkerChannels>>,
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
            processes: Arc::new(ProcessRegistry::new()),
            rip_tx,
            encode_tx,
            worker_channels: Mutex::new(Some(WorkerChannels { rip_rx, encode_rx })),
        }
    }

    /// Returns a snapshot of all jobs currently tracked by the queue.
    pub async fn list_jobs(&self) -> Vec<QueueJob> {
        self.jobs.read().await.clone()
    }

    /// Cancels a queued or active job and signals its running child process (if any) to stop.
    pub async fn cancel_job(&self, job_id: &str) -> bool {
        let signalled = self.processes.cancel(job_id);
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|item| item.id == job_id) {
            if !job.stage.is_terminal()
                && job
                    .stage
                    .can_transition_to_for_mode(job.mode, JobStage::Cancelled)
            {
                let _ = job.transition_to(JobStage::Cancelled);
                return true;
            }
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
                .then(|| "HQ 1080p30 Surround".to_string()),
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
    fn valid_rip_only_transitions() {
        let mut job = sample_job(PipelineMode::RipOnly);
        assert_eq!(job.stage, JobStage::QueuedForRip);
        assert!(job.transition_to(JobStage::Ripping).is_ok());
        assert!(job.transition_to(JobStage::QueuedForEncode).is_err());
        assert!(job.transition_to(JobStage::Completed).is_ok());
    }

    #[test]
    fn valid_encode_only_transitions() {
        let mut job = sample_job(PipelineMode::EncodeOnly);
        assert_eq!(job.stage, JobStage::QueuedForEncode);
        assert!(job.transition_to(JobStage::Ripping).is_err());
        assert!(job.transition_to(JobStage::Encoding).is_ok());
        assert!(job.transition_to(JobStage::CleaningTemp).is_err());
        assert!(job.transition_to(JobStage::Completed).is_ok());
    }

    #[test]
    fn rejects_invalid_pipeline_transitions() {
        let mut job = sample_job(PipelineMode::RipAndEncode);
        assert!(matches!(
            job.transition_to(JobStage::Encoding),
            Err(PipelineError::InvalidTransition {
                from: JobStage::QueuedForRip,
                to: JobStage::Encoding
            })
        ));
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
}
