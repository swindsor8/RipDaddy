pub mod detect;
pub mod folder_id;
pub mod handbrake;
pub mod makemkv;
pub mod metadata;
pub mod organize;
pub mod pipeline;
pub mod preview;

use detect::{AutoSetupResult, BinariesCheckResult, BinaryKind, BinaryPathsConfig};
use folder_id::{FolderIdentifyResult, IdentifiedVideoFile};
use handbrake::{HandBrakePreset, QualityProfile};
use makemkv::{DiscScanResult, OpticalDrive};
use metadata::{
    DiscLabelGuess, EpisodeInfo, ExternalIds, FolderOrganization, MediaKind, MediaMetadata,
    MetadataProvider, MetadataProviderConfig, MetadataSearchQuery, NamingConvention,
};
use organize::{
    OrganizeExecutionResult, OrganizePlan, OrganizePlanRequest, OrganizeUndoStore,
    UndoOrganizeResult,
};
use pipeline::{EnqueueJobRequest, PipelineController, PipelineEvent, QueueJob};
use preview::TrickplayPreview;
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

/// Generates the exact `HandBrakeCLI` flags for a customizable `QualityProfile`.
#[tauri::command]
fn preview_quality_cli_args(profile: QualityProfile) -> Vec<String> {
    handbrake::build_quality_flags(&profile)
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
    episodes_catalog: Vec<EpisodeInfo>,
    default_season: Option<u32>,
) -> Vec<IdentifiedVideoFile> {
    folder_id::rematch_files_to_series(
        &file_paths,
        &show_title,
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

/// Cancels a queued or running job and terminates its active child process if running.
#[tauri::command]
async fn cancel_job(
    job_id: String,
    state: tauri::State<'_, PipelineController>,
) -> Result<bool, String> {
    Ok(state.cancel_job(&job_id).await)
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
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::default().build())
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
            fetch_series_episodes,
            rematch_tv_files,
            plan_organize_folder,
            execute_organize_folder,
            undo_last_organize,
            enqueue_jobs,
            get_queue,
            cancel_job,
            clear_finished_jobs,
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
