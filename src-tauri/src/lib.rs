pub mod detect;
pub mod folder_id;
pub mod handbrake;
pub mod makemkv;
pub mod metadata;
pub mod organize;
pub mod pipeline;
pub mod preview;
pub mod system;

use detect::{AutoSetupResult, BinariesCheckResult, BinaryKind, BinaryPathsConfig};
use folder_id::{FolderIdentifyResult, IdentifiedVideoFile};
use handbrake::{EncoderCapabilities, HandBrakePreset, QualityProfile, TrackSelectionOptions};
use makemkv::{DiscScanResult, OpticalDrive};
use metadata::{
    DiscLabelGuess, EpisodeInfo, ExternalIds, FolderOrganization, MediaKind, MediaMetadata,
    MetadataProvider, MetadataProviderConfig, MetadataSearchQuery, NamingConvention,
};
use organize::{
    OrganizeExecutionResult, OrganizePlan, OrganizePlanRequest, OrganizeUndoStore,
    UndoOrganizeResult,
};
use pipeline::{
    EnqueueJobRequest, PipelineController, PipelineEvent, QueueJob, QueueStateSnapshot,
    ReorderAction,
};
use preview::TrickplayPreview;
use system::StorageSpaceReport;
use std::path::Path;
use tauri::ipc::Channel;
use tauri::{Manager, RunEvent};

/// Detects external `makemkvcon` and `HandBrakeCLI` installations, available OS package
/// managers, default directories, and probes CLI versions.
#[tauri::command]
async fn check_binaries(
    config: Option<BinaryPathsConfig>,
) -> Result<BinariesCheckResult, String> {
    Ok(detect::check_binaries(config.unwrap_or_default()).await)
}

/// Installs a single external CLI using the host OS's native package manager (`winget`/`brew`/`flatpak`).
#[tauri::command]
async fn install_binary_cli(
    kind: BinaryKind,
    config: Option<BinaryPathsConfig>,
) -> Result<BinariesCheckResult, String> {
    detect::install_binary_via_package_manager(kind).await?;
    Ok(detect::check_binaries(config.unwrap_or_default()).await)
}

/// Auto-installs all missing CLIs supported by the detected OS package manager and re-probes paths.
#[tauri::command]
async fn auto_setup_dependencies(
    config: Option<BinaryPathsConfig>,
) -> Result<AutoSetupResult, String> {
    Ok(detect::auto_setup_missing_binaries(config.unwrap_or_default()).await)
}

/// Enumerates connected optical drives via `makemkvcon -r`.
#[tauri::command]
async fn scan_drives(config: Option<BinaryPathsConfig>) -> Result<Vec<OpticalDrive>, String> {
    let detected = detect::check_binaries(config.unwrap_or_default()).await;
    let cmd = detected.makemkv.command.filter(|_| detected.makemkv.found).ok_or_else(|| {
        detected
            .makemkv
            .error
            .unwrap_or_else(|| "makemkvcon is not installed or failed version check.".to_string())
    })?;
    makemkv::scan_drives(&cmd)
        .await
        .map_err(|err| err.to_string())
}

/// Scans an optical disc in `drive_index` and returns its titles and video/audio/subtitle streams.
#[tauri::command]
async fn scan_disc(
    drive_index: u32,
    min_length_seconds: Option<u64>,
    config: Option<BinaryPathsConfig>,
) -> Result<DiscScanResult, String> {
    let detected = detect::check_binaries(config.unwrap_or_default()).await;
    let cmd = detected.makemkv.command.filter(|_| detected.makemkv.found).ok_or_else(|| {
        detected
            .makemkv
            .error
            .unwrap_or_else(|| "makemkvcon is not installed or failed version check.".to_string())
    })?;
    makemkv::scan_disc(&cmd, drive_index, min_length_seconds.unwrap_or(120))
        .await
        .map_err(|err| err.to_string())
}

/// Lists HandBrakeCLI built-in presets via `HandBrakeCLI --preset-list`.
#[tauri::command]
async fn list_handbrake_presets(
    config: Option<BinaryPathsConfig>,
) -> Result<Vec<HandBrakePreset>, String> {
    let detected = detect::check_binaries(config.unwrap_or_default()).await;
    let cmd = detected
        .handbrake
        .command
        .filter(|_| detected.handbrake.found)
        .ok_or_else(|| {
            detected.handbrake.error.unwrap_or_else(|| {
                "HandBrakeCLI is not installed or failed version check.".to_string()
            })
        })?;
    handbrake::list_presets(&cmd)
        .await
        .map_err(|err| err.to_string())
}

/// Probes `HandBrakeCLI --help` and local GPU hardware to detect supported CPU and GPU encoders
/// (`nvenc_h265`, `vaapi_h265`, `qsv_h265`, `vt_h265`).
#[tauri::command]
async fn probe_video_encoders(
    config: Option<BinaryPathsConfig>,
) -> Result<EncoderCapabilities, String> {
    let detected = detect::check_binaries(config.unwrap_or_default()).await;
    let cmd = detected
        .handbrake
        .found
        .then_some(detected.handbrake.command)
        .flatten();
    Ok(handbrake::probe_encoders(cmd.as_ref(), detected.handbrake.version).await)
}

/// Inspects free and total disk space for the configured Temp staging and Output library volumes.
#[tauri::command]
async fn get_disk_space(
    temp_dir: Option<String>,
    output_dir: Option<String>,
) -> Result<StorageSpaceReport, String> {
    Ok(system::inspect_storage_space(temp_dir.as_deref(), output_dir.as_deref()).await)
}

/// Sends a native OS desktop notification (used for testing notifications in Settings).
#[tauri::command]
async fn send_os_notification(title: String, body: String) -> Result<(), String> {
    system::send_desktop_notification(&title, &body).await
}

/// Generates a 10-second WebView-playable MP4 sample clip using the selected quality profile.
#[tauri::command]
async fn generate_sample_clip(
    input_path: String,
    temp_dir: String,
    profile: QualityProfile,
    start_seconds: Option<u64>,
    config: Option<BinaryPathsConfig>,
) -> Result<String, String> {
    let detected = detect::check_binaries(config.unwrap_or_default()).await;
    let cmd = detected
        .handbrake
        .command
        .filter(|_| detected.handbrake.found)
        .ok_or_else(|| {
            detected.handbrake.error.unwrap_or_else(|| {
                "HandBrakeCLI is not installed or failed version check.".to_string()
            })
        })?;
    let sample_path = handbrake::generate_sample_clip(
        &cmd,
        Path::new(&input_path),
        Path::new(&temp_dir),
        &profile,
        start_seconds.unwrap_or(0),
        10,
    )
    .await
    .map_err(|err| err.to_string())?;
    Ok(sample_path.to_string_lossy().into_owned())
}

/// Automatically extracts evenly spaced trickplay still frames and probes resolution,
/// aspect ratio, codec, and audio metadata for a selected local video file.
#[tauri::command]
async fn generate_trickplay_preview(
    input_path: String,
    temp_dir: Option<String>,
    frame_count: Option<usize>,
    force_regenerate: Option<bool>,
    config: Option<BinaryPathsConfig>,
) -> Result<TrickplayPreview, String> {
    let resolved_temp = temp_dir
        .filter(|s| !s.trim().is_empty() && !s.trim().eq_ignore_ascii_case("auto"))
        .unwrap_or_else(|| detect::detect_default_paths().temp_dir);
    let detected = detect::check_binaries(config.unwrap_or_default()).await;
    let handbrake_cmd = detected
        .handbrake
        .found
        .then_some(detected.handbrake.command)
        .flatten();
    preview::generate_trickplay_preview(
        Path::new(&input_path),
        Path::new(&resolved_temp),
        handbrake_cmd.as_ref(),
        frame_count,
        force_regenerate.unwrap_or(false),
    )
    .await
}

/// Returns the 8 recommended Resolution (480p/720p/1080p/4K) × Delivery (Local/Remote) profiles.
#[tauri::command]
fn get_recommended_quality_profiles() -> Vec<QualityProfile> {
    handbrake::recommended_quality_profiles()
}

/// Generates the exact `HandBrakeCLI` flags for a customizable `QualityProfile` + track options.
#[tauri::command]
fn preview_quality_cli_args(
    profile: QualityProfile,
    track_selection: Option<TrackSelectionOptions>,
) -> Vec<String> {
    handbrake::build_quality_flags_with_tracks(&profile, &track_selection.unwrap_or_default())
}

/// Cleans an optical disc volume label into a title, year, and season guess.
#[tauri::command]
fn clean_disc_label(raw_label: String) -> DiscLabelGuess {
    metadata::clean_disc_label(&raw_label)
}

/// Generates a live Plex / Jellyfin / Plain relative output path from IMDb/TVDB/TMDB metadata.
#[tauri::command]
fn preview_output_filename(
    media: MediaMetadata,
    convention: NamingConvention,
    extension: Option<String>,
    folder_organization: Option<FolderOrganization>,
) -> String {
    let org = folder_organization.unwrap_or_else(|| FolderOrganization::default_for(convention));
    metadata::format_output_relative_path_with_org(
        &media,
        convention,
        extension.as_deref().unwrap_or("mkv"),
        org,
    )
}

/// Searches IMDb (OMDb), TVDB v4, TMDB, or keyless TVMaze for matching movies/shows.
#[tauri::command]
async fn search_metadata(
    query: MetadataSearchQuery,
    config: Option<MetadataProviderConfig>,
) -> Result<Vec<MediaMetadata>, String> {
    metadata::search_metadata(query, config.unwrap_or_default())
        .await
        .map_err(|err| err.to_string())
}

/// Fetches IMDb and TVDB external IDs for a selected TMDB movie or TV show.
#[tauri::command]
async fn fetch_tmdb_external_ids(
    kind: MediaKind,
    tmdb_id: String,
    config: Option<MetadataProviderConfig>,
) -> Result<ExternalIds, String> {
    metadata::fetch_tmdb_external_ids(kind, &tmdb_id, config.unwrap_or_default())
        .await
        .map_err(|err| err.to_string())
}

/// Scans a folder of video files and automatically identifies the TV show, external IDs,
/// season/episode numbers, and official episode titles via TVDB, TMDB, or keyless TVMaze.
#[tauri::command]
async fn identify_video_folder(
    folder_path: String,
    config: Option<MetadataProviderConfig>,
) -> Result<FolderIdentifyResult, String> {
    folder_id::identify_video_folder(Path::new(&folder_path), config.unwrap_or_default()).await
}

/// Identifies a TV show and official episode titles for an already-selected list of local files.
#[tauri::command]
async fn identify_video_files(
    file_paths: Vec<String>,
    config: Option<MetadataProviderConfig>,
) -> Result<FolderIdentifyResult, String> {
    folder_id::identify_video_file_list(&file_paths, config.unwrap_or_default()).await
}

/// Identifies any mix of dropped folders and/or video files from native drag-and-drop.
#[tauri::command]
async fn identify_dropped_paths(
    paths: Vec<String>,
    config: Option<MetadataProviderConfig>,
) -> Result<FolderIdentifyResult, String> {
    folder_id::identify_dropped_paths(&paths, config.unwrap_or_default()).await
}

/// Fetches the official episode catalog for a manually selected TV series so all queued rows
/// immediately adopt canonical episode titles and uniform naming.
#[tauri::command]
async fn fetch_series_episodes(
    title: String,
    ids: ExternalIds,
    seasons_hint: Option<Vec<u32>>,
    config: Option<MetadataProviderConfig>,
) -> Result<(MetadataProvider, Vec<EpisodeInfo>), String> {
    metadata::fetch_series_episodes(
        &title,
        &ids,
        seasons_hint.as_deref().unwrap_or(&[]),
        &config.unwrap_or_default(),
    )
    .await
    .map_err(|err| err.to_string())
}

/// Re-parses and fuzzy-matches loaded local video files against a chosen TV show and episode
/// catalog (used when switching a misclassified folder to TV or selecting a show manually).
#[tauri::command]
async fn rematch_tv_files(
    file_paths: Vec<String>,
    show_title: String,
    series_metadata: Option<MediaMetadata>,
    episodes_catalog: Vec<EpisodeInfo>,
    default_season: Option<u32>,
) -> Vec<IdentifiedVideoFile> {
    folder_id::rematch_files_to_series(
        &file_paths,
        &show_title,
        series_metadata,
        &episodes_catalog,
        default_season,
    )
    .await
}

/// Builds a read-only dry-run reorganization & rename plan for already-encoded media files.
#[tauri::command]
async fn plan_organize_folder(request: OrganizePlanRequest) -> Result<OrganizePlan, String> {
    tokio::task::spawn_blocking(move || organize::plan_organize(&request))
        .await
        .map_err(|err| format!("Organize planning task failed: {err}"))?
}

/// Executes a folder rename & reorganization pass (move or copy, optional sidecars & pruning)
/// and records completed move operations for one-click Undo.
#[tauri::command]
async fn execute_organize_folder(
    request: OrganizePlanRequest,
    undo_state: tauri::State<'_, OrganizeUndoStore>,
) -> Result<OrganizeExecutionResult, String> {
    let store = undo_state.inner().clone();
    tokio::task::spawn_blocking(move || organize::execute_organize(&request, Some(&store)))
        .await
        .map_err(|err| format!("Organize execution task failed: {err}"))?
}

/// Reverses the last completed Move reorganization pass back to original file paths.
#[tauri::command]
async fn undo_last_organize(
    undo_state: tauri::State<'_, OrganizeUndoStore>,
) -> Result<UndoOrganizeResult, String> {
    let store = undo_state.inner().clone();
    tokio::task::spawn_blocking(move || organize::undo_last_organize(&store))
        .await
        .map_err(|err| format!("Undo task failed: {err}"))?
}

/// Enqueues one or more RipOnly, EncodeOnly, or RipAndEncode jobs onto the live Tokio workers.
#[tauri::command]
async fn enqueue_jobs(
    requests: Vec<EnqueueJobRequest>,
    config: Option<BinaryPathsConfig>,
    state: tauri::State<'_, PipelineController>,
) -> Result<Vec<QueueJob>, String> {
    let detected = detect::check_binaries(config.unwrap_or_default()).await;
    let makemkv_cmd = detected.makemkv.found.then_some(detected.makemkv.command).flatten();
    let handbrake_cmd = detected
        .handbrake
        .found
        .then_some(detected.handbrake.command)
        .flatten();
    state
        .enqueue_jobs(requests, makemkv_cmd, handbrake_cmd)
        .await
}

/// Returns all jobs currently tracked by the two-stage pipeline queue.
#[tauri::command]
async fn get_queue(state: tauri::State<'_, PipelineController>) -> Result<Vec<QueueJob>, String> {
    Ok(state.list_jobs().await)
}

/// Returns the complete queue snapshot including paused & restored-session status.
#[tauri::command]
async fn get_queue_state(
    state: tauri::State<'_, PipelineController>,
) -> Result<QueueStateSnapshot, String> {
    Ok(state.queue_state().await)
}

/// Moves a pending job to the top, up, down, or bottom of the queue.
#[tauri::command]
async fn move_queue_job(
    job_id: String,
    action: ReorderAction,
    state: tauri::State<'_, PipelineController>,
) -> Result<Vec<QueueJob>, String> {
    state.move_job(&job_id, action).await
}

/// Reorders a pending job directly to a target index (e.g. via drag-and-drop).
#[tauri::command]
async fn reorder_queue_job(
    job_id: String,
    target_index: usize,
    state: tauri::State<'_, PipelineController>,
) -> Result<Vec<QueueJob>, String> {
    state.reorder_job_to_index(&job_id, target_index).await
}

/// Removes a single queued or finished job from the queue and persisted `queue.json`.
#[tauri::command]
async fn remove_queue_job(
    job_id: String,
    state: tauri::State<'_, PipelineController>,
) -> Result<Vec<QueueJob>, String> {
    state.remove_job(&job_id).await
}

/// Pauses or resumes queue dispatch (refreshing external CLI paths on resume).
#[tauri::command]
async fn set_queue_paused(
    paused: bool,
    config: Option<BinaryPathsConfig>,
    state: tauri::State<'_, PipelineController>,
) -> Result<QueueStateSnapshot, String> {
    let (makemkv_cmd, handbrake_cmd) = if paused {
        (None, None)
    } else {
        let detected = detect::check_binaries(config.unwrap_or_default()).await;
        (
            detected.makemkv.found.then_some(detected.makemkv.command).flatten(),
            detected
                .handbrake
                .found
                .then_some(detected.handbrake.command)
                .flatten(),
        )
    };
    Ok(state.set_paused(paused, makemkv_cmd, handbrake_cmd).await)
}

/// Enables or disables native OS desktop completion & disc-eject notifications.
#[tauri::command]
fn set_desktop_notifications(
    enabled: bool,
    state: tauri::State<'_, PipelineController>,
) -> Result<(), String> {
    state.set_notifications_enabled(enabled);
    Ok(())
}

/// Cancels a queued or running job and terminates its active child process if running.
#[tauri::command]
async fn cancel_job(
    job_id: String,
    state: tauri::State<'_, PipelineController>,
) -> Result<bool, String> {
    Ok(state.cancel_job(&job_id).await)
}

/// Retries a failed or cancelled job on the live pipeline workers.
#[tauri::command]
async fn retry_job(
    job_id: String,
    config: Option<BinaryPathsConfig>,
    state: tauri::State<'_, PipelineController>,
) -> Result<QueueJob, String> {
    let detected = detect::check_binaries(config.unwrap_or_default()).await;
    let makemkv_cmd = detected.makemkv.found.then_some(detected.makemkv.command).flatten();
    let handbrake_cmd = detected
        .handbrake
        .found
        .then_some(detected.handbrake.command)
        .flatten();
    state.retry_job(&job_id, makemkv_cmd, handbrake_cmd).await
}

/// Reveals a file or opens a directory in the host OS's native file manager
/// (Finder on macOS, Explorer on Windows, Nautilus/Dolphin/xdg-open on Linux).
#[tauri::command]
fn reveal_in_file_manager(path: String) -> Result<(), String> {
    let target = Path::new(path.trim());
    if path.trim().is_empty() {
        return Err("No path provided to open.".to_string());
    }
    let dir_fallback = if target.is_dir() {
        target.to_path_buf()
    } else {
        target
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| target.to_path_buf())
    };
    if !target.exists() && !dir_fallback.exists() {
        return Err(format!("Path does not exist: {}", target.display()));
    }

    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new("open");
        if target.is_file() {
            cmd.arg("-R").arg(target);
        } else {
            cmd.arg(&dir_fallback);
        }
        cmd.spawn()
            .map_err(|err| format!("Failed opening Finder: {err}"))?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        let mut cmd = std::process::Command::new("explorer");
        if target.is_file() {
            cmd.arg(format!("/select,{}", target.display()));
        } else {
            cmd.arg(&dir_fallback);
        }
        cmd.spawn()
            .map_err(|err| format!("Failed opening Explorer: {err}"))?;
        return Ok(());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if target.is_file() {
            if let Ok(canon) = std::fs::canonicalize(target) {
                let uri = format!("file://{}", canon.to_string_lossy());
                let mut dbus_cmd = std::process::Command::new("dbus-send");
                dbus_cmd.args([
                    "--session",
                    "--dest=org.freedesktop.FileManager1",
                    "--type=method_call",
                    "/org/freedesktop/FileManager1",
                    "org.freedesktop.FileManager1.ShowItems",
                    &format!("array:string:{uri}"),
                    "string:",
                ]);
                system::configure_external_std_command(&mut dbus_cmd);
                if dbus_cmd.status().is_ok_and(|s| s.success()) {
                    return Ok(());
                }
            }
        }
        let mut open_cmd = std::process::Command::new("xdg-open");
        open_cmd.arg(&dir_fallback);
        system::configure_external_std_command(&mut open_cmd);
        open_cmd
            .spawn()
            .map_err(|err| format!("Failed opening file manager: {err}"))?;
        Ok(())
    }
}

/// Opens an HTTPS URL in the user's default system browser (e.g. GitHub Releases fallback).
#[tauri::command]
fn open_external_url(url: String) -> Result<(), String> {
    let trimmed = url.trim();
    if !trimmed.starts_with("https://") && !trimmed.starts_with("http://") {
        return Err("Only http:// or https:// URLs can be opened.".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(trimmed)
            .spawn()
            .map_err(|err| format!("Failed opening browser: {err}"))?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("cmd")
            .args(["/C", "start", "", trimmed])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|err| format!("Failed opening browser: {err}"))?;
        return Ok(());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut cmd = std::process::Command::new("xdg-open");
        cmd.arg(trimmed);
        system::configure_external_std_command(&mut cmd);
        cmd.spawn()
            .map_err(|err| format!("Failed opening browser: {err}"))?;
        Ok(())
    }
}

/// Removes completed, cancelled, and failed jobs from the queue list.
#[tauri::command]
async fn clear_finished_jobs(
    state: tauri::State<'_, PipelineController>,
) -> Result<Vec<QueueJob>, String> {
    Ok(state.clear_finished().await)
}

/// Subscribes a Tauri v2 IPC Channel to receive real-time pipeline events.
#[tauri::command]
async fn subscribe_pipeline_events(
    on_event: Channel<PipelineEvent>,
    state: tauri::State<'_, PipelineController>,
) -> Result<(), String> {
    for job in state.list_jobs().await {
        on_event
            .send(PipelineEvent::JobUpdated { job })
            .map_err(|err| err.to_string())?;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_store::Builder::default().build());

    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());

    let app = builder
        .manage(PipelineController::new())
        .manage(OrganizeUndoStore::new())
        .setup(|app| {
            app.state::<PipelineController>()
                .start_workers(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            check_binaries,
            install_binary_cli,
            auto_setup_dependencies,
            scan_drives,
            scan_disc,
            list_handbrake_presets,
            probe_video_encoders,
            get_disk_space,
            send_os_notification,
            generate_sample_clip,
            generate_trickplay_preview,
            get_recommended_quality_profiles,
            preview_quality_cli_args,
            clean_disc_label,
            preview_output_filename,
            search_metadata,
            fetch_tmdb_external_ids,
            identify_video_folder,
            identify_video_files,
            identify_dropped_paths,
            fetch_series_episodes,
            rematch_tv_files,
            plan_organize_folder,
            execute_organize_folder,
            undo_last_organize,
            enqueue_jobs,
            get_queue,
            get_queue_state,
            move_queue_job,
            reorder_queue_job,
            remove_queue_job,
            set_queue_paused,
            set_desktop_notifications,
            cancel_job,
            retry_job,
            clear_finished_jobs,
            reveal_in_file_manager,
            open_external_url,
            subscribe_pipeline_events
        ])
        .build(tauri::generate_context!())
        .expect("error while building RipDaddy");

    app.run(|app_handle, event| {
        if let RunEvent::Exit = event {
            app_handle.state::<PipelineController>().shutdown_sync();
        }
    });
}
