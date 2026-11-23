pub mod detect;
pub mod handbrake;
pub mod makemkv;
pub mod metadata;
pub mod pipeline;

use detect::{AutoSetupResult, BinariesCheckResult, BinaryKind, BinaryPathsConfig};
use handbrake::QualityProfile;
use metadata::{
    DiscLabelGuess, ExternalIds, MediaKind, MediaMetadata, MetadataProviderConfig,
    MetadataSearchQuery, NamingConvention,
};
use pipeline::{PipelineController, PipelineEvent, QueueJob};
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

/// Installs a single external CLI using the host OS's native package manager (`winget`/`brew`).
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
) -> String {
    metadata::format_output_relative_path(
        &media,
        convention,
        extension.as_deref().unwrap_or("mkv"),
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
        .invoke_handler(tauri::generate_handler![
            check_binaries,
            install_binary_cli,
            auto_setup_dependencies,
            get_recommended_quality_profiles,
            preview_quality_cli_args,
            clean_disc_label,
            preview_output_filename,
            search_metadata,
            fetch_tmdb_external_ids,
            get_queue,
            cancel_job,
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
