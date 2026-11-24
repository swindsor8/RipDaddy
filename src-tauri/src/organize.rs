//! Safe directory reorganization & batch renaming engine for already-encoded media files.
//!
//! Features:
//! - Read-only dry-run planning (`plan_organize`) preserving original video extensions
//! - Companion subtitle & metadata sidecar matching (`.srt`, `.ass`, `.vtt`, `.nfo`, etc.)
//!   with language/flag suffix preservation (e.g. `.en.forced.srt`)
//! - Duplicate-target and existing-file collision detection (never silently overwrites)
//! - Instant same-volume `fs::rename` with atomic cross-device copy-verify-swap fallback
//! - Optional empty source subdirectory pruning (never deletes the connected root folder)
//! - In-memory one-click Undo manifest for move operations

use crate::metadata::{self, FolderOrganization, MediaMetadata, NamingConvention};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const SIDECAR_EXTENSIONS: &[&str] = &[
    "srt", "ass", "ssa", "vtt", "sub", "idx", "sup", "smi", "nfo",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OrganizeTransferMode {
    Move,
    Copy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizeFileRequest {
    pub source_path: String,
    pub media_metadata: MediaMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizePlanRequest {
    pub items: Vec<OrganizeFileRequest>,
    pub destination_dir: String,
    pub naming_convention: NamingConvention,
    pub folder_organization: FolderOrganization,
    pub transfer_mode: OrganizeTransferMode,
    pub include_sidecars: bool,
    pub prune_empty_dirs: bool,
    pub source_root_dir: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OrganizeItemStatus {
    Ready,
    Unchanged,
    Conflict,
    MissingSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedSidecarOp {
    pub source_path: String,
    pub target_path: String,
    pub suffix: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedOrganizeItem {
    pub source_path: String,
    pub target_path: String,
    pub relative_target_path: String,
    pub status: OrganizeItemStatus,
    pub reason: Option<String>,
    pub sidecars: Vec<PlannedSidecarOp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizePlan {
    pub destination_dir: String,
    pub transfer_mode: OrganizeTransferMode,
    pub include_sidecars: bool,
    pub prune_empty_dirs: bool,
    pub source_root_dir: Option<String>,
    pub items: Vec<PlannedOrganizeItem>,
    pub ready_count: usize,
    pub unchanged_count: usize,
    pub conflict_count: usize,
    pub sidecar_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletedFileOp {
    pub source_path: String,
    pub target_path: String,
    pub transfer_mode: OrganizeTransferMode,
    pub is_sidecar: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizeExecutionResult {
    pub batch_id: String,
    pub transfer_mode: OrganizeTransferMode,
    pub moved_count: usize,
    pub sidecars_moved_count: usize,
    pub skipped_count: usize,
    pub failed_count: usize,
    pub pruned_dirs_count: usize,
    pub can_undo: bool,
    pub operations: Vec<CompletedFileOp>,
    pub errors: Vec<String>,
    pub status_message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoOrganizeResult {
    pub restored_count: usize,
    pub failed_count: usize,
    pub errors: Vec<String>,
    pub status_message: String,
}

#[derive(Debug, Clone)]
struct UndoBatch {
    destination_root: PathBuf,
    operations: Vec<CompletedFileOp>,
}

/// Thread-safe in-memory store holding the most recent move batch for one-click Undo.
#[derive(Debug, Default, Clone)]
pub struct OrganizeUndoStore {
    last_batch: Arc<Mutex<Option<UndoBatch>>>,
}

impl OrganizeUndoStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn save_batch(&self, batch: UndoBatch) {
        if let Ok(mut guard) = self.last_batch.lock() {
            *guard = Some(batch);
        }
    }

    fn take_batch(&self) -> Option<UndoBatch> {
        self.last_batch.lock().ok().and_then(|mut guard| guard.take())
    }
}

/// Lexical path normalization without requiring the target path to exist on disk.
fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(comp);
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// Canonicalizes a path when possible, or falls back to canonicalizing its nearest
/// existing ancestor and appending remaining components so non-existent destinations
/// compare accurately against existing source paths (handling symlinks like `/var/home` vs `/home`).
fn normalize_path_best_effort(path: &Path) -> PathBuf {
    if let Ok(canon) = fs::canonicalize(path) {
        return canon;
    }
    let mut missing_tail = Vec::new();
    let mut cursor = path;
    while !cursor.exists() {
        if let Some(name) = cursor.file_name() {
            missing_tail.push(name.to_os_string());
        } else {
            break;
        }
        match cursor.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => cursor = parent,
            _ => break,
        }
    }
    let mut base = fs::canonicalize(cursor).unwrap_or_else(|_| normalize_lexical(cursor));
    for seg in missing_tail.into_iter().rev() {
        base.push(seg);
    }
    normalize_lexical(&base)
}

/// Case-folded key used for cross-platform collision detection within a batch.
fn path_collision_key(path: &Path) -> String {
    normalize_path_best_effort(path)
        .to_string_lossy()
        .to_ascii_lowercase()
}

/// Returns `true` when two paths refer to the exact same filesystem entry (including
/// case-only differences on case-insensitive filesystems).
fn is_same_exact_path(a: &Path, b: &Path) -> bool {
    normalize_path_best_effort(a) == normalize_path_best_effort(b)
}

fn is_case_only_rename(src: &Path, dst: &Path) -> bool {
    if is_same_exact_path(src, dst) {
        return false;
    }
    let src_norm = normalize_path_best_effort(src);
    let dst_norm = normalize_path_best_effort(dst);
    src_norm
        .to_string_lossy()
        .eq_ignore_ascii_case(&dst_norm.to_string_lossy())
}

fn is_sidecar_extension(ext: &str) -> bool {
    SIDECAR_EXTENSIONS
        .iter()
        .any(|allowed| ext.eq_ignore_ascii_case(allowed))
}

/// Finds companion subtitle / `.nfo` sidecars in the same folder as `video_src`
/// whose filename begins with `<video_stem>.` and maps them onto `<target_stem><suffix>`.
fn discover_sidecars(
    video_src: &Path,
    video_dst: &Path,
    claimed_targets: &HashSet<String>,
) -> Vec<PlannedSidecarOp> {
    let (Some(src_dir), Some(src_stem), Some(dst_dir), Some(dst_stem)) = (
        video_src.parent(),
        video_src.file_stem().and_then(|s| s.to_str()),
        video_dst.parent(),
        video_dst.file_stem().and_then(|s| s.to_str()),
    ) else {
        return Vec::new();
    };

    if src_stem.is_empty() || dst_stem.is_empty() {
        return Vec::new();
    }

    let Ok(entries) = fs::read_dir(src_dir) else {
        return Vec::new();
    };

    let prefix_lower = format!("{}.", src_stem.to_ascii_lowercase());
    let mut found = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path == video_src || !path.is_file() {
            continue;
        }
        let Some(fname) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if fname.starts_with('.') || fname.starts_with("._") {
            continue;
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !is_sidecar_extension(ext) {
            continue;
        }

        let fname_lower = fname.to_ascii_lowercase();
        if !fname_lower.starts_with(&prefix_lower) {
            continue;
        }

        // Preserve exact language / forced / sdh suffix (e.g. `.en.forced.srt` or `.srt`)
        let suffix = &fname[src_stem.len()..];
        let target_name = format!("{dst_stem}{suffix}");
        let target_path = dst_dir.join(&target_name);

        if is_same_exact_path(&path, &target_path) {
            continue;
        }
        let key = path_collision_key(&target_path);
        if claimed_targets.contains(&key) {
            continue;
        }
        if target_path.exists() && !is_case_only_rename(&path, &target_path) {
            continue;
        }

        found.push(PlannedSidecarOp {
            source_path: path.to_string_lossy().into_owned(),
            target_path: target_path.to_string_lossy().into_owned(),
            suffix: suffix.to_string(),
        });
    }

    found.sort_by(|a, b| a.suffix.to_ascii_lowercase().cmp(&b.suffix.to_ascii_lowercase()));
    found
}

/// Computes a read-only dry-run reorganization plan without modifying any files on disk.
pub fn plan_organize(req: &OrganizePlanRequest) -> Result<OrganizePlan, String> {
    let dest_trimmed = req.destination_dir.trim();
    if dest_trimmed.is_empty() {
        return Err("Choose a destination folder before organizing.".to_string());
    }
    let dest_root = PathBuf::from(dest_trimmed);

    // Track source paths inside this batch so moving A->B while B->C isn't falsely flagged
    // unless B is NOT moving away. For safety, we only allow swapping if B is actually planned
    // to move somewhere else; simplest and safest rule is marking existing destination as Conflict
    // unless it's a case-only rename of the exact same file.
    let mut claimed_targets: HashSet<String> = HashSet::new();
    let mut items = Vec::with_capacity(req.items.len());

    let mut ready_count = 0usize;
    let mut unchanged_count = 0usize;
    let mut conflict_count = 0usize;
    let mut sidecar_count = 0usize;

    for file_req in &req.items {
        let src_path = PathBuf::from(&file_req.source_path);
        let orig_ext = src_path
            .extension()
            .and_then(|e| e.to_str())
            .filter(|e| !e.trim().is_empty())
            .unwrap_or("mkv");

        let rel_target = metadata::format_output_relative_path_with_org(
            &file_req.media_metadata,
            req.naming_convention,
            orig_ext,
            req.folder_organization,
        );

        let mut target_path = dest_root.clone();
        for segment in rel_target.split('/') {
            if !segment.is_empty() {
                target_path.push(segment);
            }
        }
        let target_str = target_path.to_string_lossy().into_owned();
        let target_key = path_collision_key(&target_path);

        if !src_path.exists() || !src_path.is_file() {
            conflict_count += 1;
            items.push(PlannedOrganizeItem {
                source_path: file_req.source_path.clone(),
                target_path: target_str,
                relative_target_path: rel_target,
                status: OrganizeItemStatus::MissingSource,
                reason: Some("Source file no longer exists on disk.".to_string()),
                sidecars: Vec::new(),
            });
            continue;
        }

        if is_same_exact_path(&src_path, &target_path) {
            unchanged_count += 1;
            claimed_targets.insert(target_key);
            items.push(PlannedOrganizeItem {
                source_path: file_req.source_path.clone(),
                target_path: target_str,
                relative_target_path: rel_target,
                status: OrganizeItemStatus::Unchanged,
                reason: Some("Already matches target name and folder.".to_string()),
                sidecars: Vec::new(),
            });
            continue;
        }

        if claimed_targets.contains(&target_key) {
            conflict_count += 1;
            items.push(PlannedOrganizeItem {
                source_path: file_req.source_path.clone(),
                target_path: target_str,
                relative_target_path: rel_target,
                status: OrganizeItemStatus::Conflict,
                reason: Some(
                    "Another selected file maps to the exact same output path — check title or episode number."
                        .to_string(),
                ),
                sidecars: Vec::new(),
            });
            continue;
        }

        let case_only = is_case_only_rename(&src_path, &target_path);
        if target_path.exists() && !case_only {
            conflict_count += 1;
            items.push(PlannedOrganizeItem {
                source_path: file_req.source_path.clone(),
                target_path: target_str,
                relative_target_path: rel_target,
                status: OrganizeItemStatus::Conflict,
                reason: Some(
                    "A file already exists at this destination (skipped to avoid overwriting)."
                        .to_string(),
                ),
                sidecars: Vec::new(),
            });
            continue;
        }

        claimed_targets.insert(target_key);

        let sidecars = if req.include_sidecars {
            let found = discover_sidecars(&src_path, &target_path, &claimed_targets);
            for sc in &found {
                claimed_targets.insert(path_collision_key(Path::new(&sc.target_path)));
            }
            found
        } else {
            Vec::new()
        };

        sidecar_count += sidecars.len();
        ready_count += 1;
        items.push(PlannedOrganizeItem {
            source_path: file_req.source_path.clone(),
            target_path: target_str,
            relative_target_path: rel_target,
            status: OrganizeItemStatus::Ready,
            reason: None,
            sidecars,
        });
    }

    Ok(OrganizePlan {
        destination_dir: dest_root.to_string_lossy().into_owned(),
        transfer_mode: req.transfer_mode,
        include_sidecars: req.include_sidecars,
        prune_empty_dirs: req.prune_empty_dirs,
        source_root_dir: req.source_root_dir.clone(),
        items,
        ready_count,
        unchanged_count,
        conflict_count,
        sidecar_count,
    })
}

fn unique_temp_sibling(target: &Path) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let fname = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let tmp_name = format!(".ripdaddy_tmp_{nanos}_{fname}");
    target.with_file_name(tmp_name)
}

/// Safely moves or copies a single file, handling case-only renames and cross-volume moves
/// without risking partial files or silent overwrites.
fn transfer_single_file(
    src: &Path,
    dst: &Path,
    mode: OrganizeTransferMode,
) -> Result<(), String> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "Failed to create folder '{}': {err}",
                parent.display()
            )
        })?;
    }

    let case_only = is_case_only_rename(src, dst);
    if dst.exists() && !case_only {
        return Err(format!(
            "Destination already exists: {}",
            dst.display()
        ));
    }

    match mode {
        OrganizeTransferMode::Copy => {
            let tmp = unique_temp_sibling(dst);
            let src_len = fs::metadata(src).map(|m| m.len()).unwrap_or(0);
            let copied = fs::copy(src, &tmp).map_err(|err| {
                let _ = fs::remove_file(&tmp);
                format!("Failed to copy '{}': {err}", src.display())
            })?;
            if src_len > 0 && copied != src_len {
                let _ = fs::remove_file(&tmp);
                return Err(format!(
                    "Incomplete copy for '{}' ({copied} of {src_len} bytes).",
                    src.display()
                ));
            }
            fs::rename(&tmp, dst).map_err(|err| {
                let _ = fs::remove_file(&tmp);
                format!("Failed to finalize copy '{}': {err}", dst.display())
            })?;
            Ok(())
        }
        OrganizeTransferMode::Move => {
            if case_only {
                let tmp = unique_temp_sibling(dst);
                fs::rename(src, &tmp).map_err(|err| {
                    format!("Failed to stage case-only rename '{}': {err}", src.display())
                })?;
                if let Err(err) = fs::rename(&tmp, dst) {
                    let _ = fs::rename(&tmp, src);
                    return Err(format!(
                        "Failed to complete case-only rename '{}': {err}",
                        dst.display()
                    ));
                }
                return Ok(());
            }

            match fs::rename(src, dst) {
                Ok(()) => Ok(()),
                Err(_) => {
                    // Cross-volume fallback: atomic copy to temp sibling -> verify -> rename -> remove source
                    let tmp = unique_temp_sibling(dst);
                    let src_len = fs::metadata(src).map(|m| m.len()).unwrap_or(0);
                    let copied = fs::copy(src, &tmp).map_err(|err| {
                        let _ = fs::remove_file(&tmp);
                        format!(
                            "Failed to move '{}' across drives: {err}",
                            src.display()
                        )
                    })?;
                    if src_len > 0 && copied != src_len {
                        let _ = fs::remove_file(&tmp);
                        return Err(format!(
                            "Cross-drive move size mismatch for '{}' ({copied} of {src_len} bytes); source untouched.",
                            src.display()
                        ));
                    }
                    fs::rename(&tmp, dst).map_err(|err| {
                        let _ = fs::remove_file(&tmp);
                        format!("Failed to finalize cross-drive move '{}': {err}", dst.display())
                    })?;
                    fs::remove_file(src).map_err(|err| {
                        format!(
                            "Moved to '{}', but could not remove original '{}': {err}",
                            dst.display(),
                            src.display()
                        )
                    })?;
                    Ok(())
                }
            }
        }
    }
}

/// Removes empty directories walking upward from `candidate_dirs`, strictly stopping before
/// `protected_roots` (so neither the connected root folder nor the destination root is removed).
fn prune_empty_directories(candidate_dirs: &[PathBuf], protected_roots: &[PathBuf]) -> usize {
    let protected: HashSet<PathBuf> = protected_roots
        .iter()
        .map(|p| normalize_path_best_effort(p))
        .collect();

    let mut dirs: Vec<PathBuf> = candidate_dirs
        .iter()
        .map(|p| normalize_path_best_effort(p))
        .collect();
    // Process deepest directories first
    dirs.sort_by(|a, b| b.components().count().cmp(&a.components().count()));
    dirs.dedup();

    let mut removed = 0usize;
    let mut visited = HashSet::new();

    for start in dirs {
        let mut cursor = start;
        loop {
            if protected.contains(&cursor) || !visited.insert(cursor.clone()) {
                break;
            }
            if !cursor.is_dir() {
                break;
            }
            let is_empty = fs::read_dir(&cursor)
                .map(|mut it| it.next().is_none())
                .unwrap_or(false);
            if !is_empty {
                break;
            }
            if fs::remove_dir(&cursor).is_ok() {
                removed += 1;
                match cursor.parent() {
                    Some(parent) => cursor = parent.to_path_buf(),
                    None => break,
                }
            } else {
                break;
            }
        }
    }
    removed
}

/// Executes a folder reorganization plan and records completed move ops for one-click Undo.
pub fn execute_organize(
    req: &OrganizePlanRequest,
    undo_store: Option<&OrganizeUndoStore>,
) -> Result<OrganizeExecutionResult, String> {
    let plan = plan_organize(req)?;
    let batch_id = format!(
        "org-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );

    let mut moved_count = 0usize;
    let mut sidecars_moved_count = 0usize;
    let mut skipped_count = 0usize;
    let mut failed_count = 0usize;
    let mut operations: Vec<CompletedFileOp> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut candidate_prune_dirs: Vec<PathBuf> = Vec::new();

    for item in &plan.items {
        if item.status != OrganizeItemStatus::Ready {
            skipped_count += 1;
            continue;
        }

        let src = Path::new(&item.source_path);
        let dst = Path::new(&item.target_path);

        match transfer_single_file(src, dst, plan.transfer_mode) {
            Ok(()) => {
                moved_count += 1;
                if let Some(parent) = src.parent() {
                    candidate_prune_dirs.push(parent.to_path_buf());
                }
                operations.push(CompletedFileOp {
                    source_path: item.source_path.clone(),
                    target_path: item.target_path.clone(),
                    transfer_mode: plan.transfer_mode,
                    is_sidecar: false,
                });

                // Transfer matching companion sidecars only after the video itself succeeds
                for sc in &item.sidecars {
                    let sc_src = Path::new(&sc.source_path);
                    let sc_dst = Path::new(&sc.target_path);
                    match transfer_single_file(sc_src, sc_dst, plan.transfer_mode) {
                        Ok(()) => {
                            sidecars_moved_count += 1;
                            operations.push(CompletedFileOp {
                                source_path: sc.source_path.clone(),
                                target_path: sc.target_path.clone(),
                                transfer_mode: plan.transfer_mode,
                                is_sidecar: true,
                            });
                        }
                        Err(err) => {
                            errors.push(format!("Sidecar '{}': {err}", sc.suffix));
                        }
                    }
                }
            }
            Err(err) => {
                failed_count += 1;
                errors.push(err);
            }
        }
    }

    let pruned_dirs_count = if plan.prune_empty_dirs
        && plan.transfer_mode == OrganizeTransferMode::Move
        && moved_count > 0
    {
        let mut protected = vec![PathBuf::from(&plan.destination_dir)];
        if let Some(root) = &plan.source_root_dir {
            protected.push(PathBuf::from(root));
        }
        prune_empty_directories(&candidate_prune_dirs, &protected)
    } else {
        0
    };

    let can_undo = plan.transfer_mode == OrganizeTransferMode::Move && !operations.is_empty();
    if can_undo {
        if let Some(store) = undo_store {
            store.save_batch(UndoBatch {
                destination_root: PathBuf::from(&plan.destination_dir),
                operations: operations.clone(),
            });
        }
    }

    let verb = match plan.transfer_mode {
        OrganizeTransferMode::Move => "Organized",
        OrganizeTransferMode::Copy => "Copied",
    };

    let mut summary_parts = vec![format!(
        "{verb} {moved_count} video{}",
        if moved_count == 1 { "" } else { "s" }
    )];
    if sidecars_moved_count > 0 {
        summary_parts.push(format!(
            "+ {sidecars_moved_count} sidecar{}",
            if sidecars_moved_count == 1 { "" } else { "s" }
        ));
    }
    if skipped_count > 0 {
        summary_parts.push(format!("{skipped_count} skipped"));
    }
    if pruned_dirs_count > 0 {
        summary_parts.push(format!(
            "{pruned_dirs_count} empty folder{} pruned",
            if pruned_dirs_count == 1 { "" } else { "s" }
        ));
    }
    if failed_count > 0 {
        summary_parts.push(format!("{failed_count} failed"));
    }

    Ok(OrganizeExecutionResult {
        batch_id,
        transfer_mode: plan.transfer_mode,
        moved_count,
        sidecars_moved_count,
        skipped_count,
        failed_count,
        pruned_dirs_count,
        can_undo,
        operations,
        errors,
        status_message: summary_parts.join(" · "),
    })
}

/// Reverses the last completed `Move` reorganization pass, restoring moved video files
/// and companion sidecars back to their original paths and pruning empty created subfolders.
pub fn undo_last_organize(undo_store: &OrganizeUndoStore) -> Result<UndoOrganizeResult, String> {
    let Some(batch) = undo_store.take_batch() else {
        return Err("Nothing to undo — no recent move reorganization found.".to_string());
    };

    let mut restored_count = 0usize;
    let mut failed_count = 0usize;
    let mut errors: Vec<String> = Vec::new();
    let mut created_dirs: Vec<PathBuf> = Vec::new();
    let mut remaining_ops: Vec<CompletedFileOp> = Vec::new();

    for op in batch.operations.iter().rev() {
        if op.transfer_mode != OrganizeTransferMode::Move {
            continue;
        }
        let current = Path::new(&op.target_path);
        let original = Path::new(&op.source_path);
        if !current.exists() {
            failed_count += 1;
            errors.push(format!(
                "Cannot restore '{}': moved file no longer exists.",
                current.display()
            ));
            continue;
        }
        match transfer_single_file(current, original, OrganizeTransferMode::Move) {
            Ok(()) => {
                restored_count += 1;
                if let Some(parent) = current.parent() {
                    created_dirs.push(parent.to_path_buf());
                }
            }
            Err(err) => {
                failed_count += 1;
                errors.push(err);
                remaining_ops.push(op.clone());
            }
        }
    }

    let mut protected = vec![batch.destination_root.clone()];
    // Also protect original parent directories so we don't prune them
    let orig_parents: HashMap<PathBuf, ()> = batch
        .operations
        .iter()
        .filter_map(|op| Path::new(&op.source_path).parent().map(|p| (p.to_path_buf(), ())))
        .collect();
    protected.extend(orig_parents.into_keys());
    prune_empty_directories(&created_dirs, &protected);

    if !remaining_ops.is_empty() {
        remaining_ops.reverse();
        undo_store.save_batch(UndoBatch {
            destination_root: batch.destination_root,
            operations: remaining_ops,
        });
    }

    let status_message = if failed_count == 0 {
        format!(
            "Restored {restored_count} file{} back to original locations.",
            if restored_count == 1 { "" } else { "s" }
        )
    } else {
        format!(
            "Restored {restored_count} file{} ({failed_count} failed).",
            if restored_count == 1 { "" } else { "s" }
        )
    };

    Ok(UndoOrganizeResult {
        restored_count,
        failed_count,
        errors,
        status_message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{ExternalIds, MediaKind};

    fn make_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("ripdaddy_organize_test_{label}_{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_tv_meta(season: u32, episode: u32, ep_title: &str) -> MediaMetadata {
        MediaMetadata {
            kind: MediaKind::TvEpisode,
            title: "Breaking Bad".to_string(),
            year: Some(2008),
            season: Some(season),
            episode: Some(episode),
            episode_title: Some(ep_title.to_string()),
            ids: ExternalIds {
                imdb_id: Some("tt0903747".to_string()),
                tvdb_id: Some("81189".to_string()),
                tmdb_id: None,
            },
            overview: None,
            poster_url: None,
        }
    }

    #[test]
    fn plans_moves_sidecars_prunes_and_undoes_cleanly() {
        let root = make_temp_dir("move_undo");
        let messy_sub = root.join("Messy_S1_Pack");
        fs::create_dir_all(&messy_sub).unwrap();

        let vid = messy_sub.join("bb.s01e01.1080p.mp4");
        let sub_en = messy_sub.join("bb.s01e01.1080p.en.forced.srt");
        fs::write(&vid, b"fake mp4 bytes").unwrap();
        fs::write(&sub_en, b"1\n00:00:01,000 --> 00:00:02,000\nYo\n").unwrap();

        let req = OrganizePlanRequest {
            items: vec![OrganizeFileRequest {
                source_path: vid.to_string_lossy().into_owned(),
                media_metadata: sample_tv_meta(1, 1, "Pilot"),
            }],
            destination_dir: root.to_string_lossy().into_owned(),
            naming_convention: NamingConvention::Plex,
            folder_organization: FolderOrganization::ShowAndSeason,
            transfer_mode: OrganizeTransferMode::Move,
            include_sidecars: true,
            prune_empty_dirs: true,
            source_root_dir: Some(root.to_string_lossy().into_owned()),
        };

        let plan = plan_organize(&req).unwrap();
        assert_eq!(plan.ready_count, 1);
        assert_eq!(plan.sidecar_count, 1);
        assert!(plan.items[0]
            .relative_target_path
            .ends_with("Breaking Bad (2008) - S01E01 - Pilot.mp4"));
        assert_eq!(plan.items[0].sidecars[0].suffix, ".en.forced.srt");

        let store = OrganizeUndoStore::new();
        let exec = execute_organize(&req, Some(&store)).unwrap();
        assert_eq!(exec.moved_count, 1);
        assert_eq!(exec.sidecars_moved_count, 1);
        assert_eq!(exec.pruned_dirs_count, 1);
        assert!(exec.can_undo);
        assert!(!vid.exists());
        assert!(!messy_sub.exists());
        assert!(Path::new(&plan.items[0].target_path).exists());
        assert!(Path::new(&plan.items[0].sidecars[0].target_path).exists());

        let undo = undo_last_organize(&store).unwrap();
        assert_eq!(undo.restored_count, 2);
        assert_eq!(undo.failed_count, 0);
        assert!(vid.exists());
        assert!(sub_en.exists());
        assert!(!Path::new(&plan.items[0].target_path).exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn flags_duplicate_targets_and_existing_files_as_conflicts() {
        let root = make_temp_dir("conflicts");
        let a = root.join("file_a.mkv");
        let b = root.join("file_b.mkv");
        let existing_dir = root.join("The Matrix (1999) {imdb-tt0133093}");
        fs::create_dir_all(&existing_dir).unwrap();
        let existing = existing_dir.join("The Matrix (1999) {imdb-tt0133093}.mkv");
        fs::write(&a, b"a").unwrap();
        fs::write(&b, b"b").unwrap();
        fs::write(&existing, b"already there").unwrap();

        let movie_meta = MediaMetadata {
            kind: MediaKind::Movie,
            title: "The Matrix".to_string(),
            year: Some(1999),
            season: None,
            episode: None,
            episode_title: None,
            ids: ExternalIds {
                imdb_id: Some("tt0133093".to_string()),
                tvdb_id: None,
                tmdb_id: None,
            },
            overview: None,
            poster_url: None,
        };

        let req = OrganizePlanRequest {
            items: vec![
                OrganizeFileRequest {
                    source_path: a.to_string_lossy().into_owned(),
                    media_metadata: movie_meta.clone(),
                },
                OrganizeFileRequest {
                    source_path: b.to_string_lossy().into_owned(),
                    media_metadata: sample_tv_meta(1, 1, "Pilot"),
                },
                OrganizeFileRequest {
                    source_path: b.to_string_lossy().into_owned(),
                    media_metadata: sample_tv_meta(1, 1, "Pilot"),
                },
            ],
            destination_dir: root.to_string_lossy().into_owned(),
            naming_convention: NamingConvention::Plex,
            folder_organization: FolderOrganization::ShowAndSeason,
            transfer_mode: OrganizeTransferMode::Move,
            include_sidecars: false,
            prune_empty_dirs: false,
            source_root_dir: Some(root.to_string_lossy().into_owned()),
        };

        let plan = plan_organize(&req).unwrap();
        assert_eq!(plan.items[0].status, OrganizeItemStatus::Conflict);
        assert_eq!(plan.items[1].status, OrganizeItemStatus::Ready);
        assert_eq!(plan.items[2].status, OrganizeItemStatus::Conflict);
        assert_eq!(plan.ready_count, 1);
        assert_eq!(plan.conflict_count, 2);

        let _ = fs::remove_dir_all(&root);
    }
}
