//! Cross-platform OS desktop notifications and Temp / Output free disk space inspection.
//!
//! Zero external crate dependencies:
//! - Desktop notifications dispatch via `notify-send` / `gdbus` on Linux (with container host
//!   delegation via `flatpak-spawn --host`), `osascript` on macOS, and WinRT Toast / NotifyIcon
//!   PowerShell on Windows.
//! - Free disk space queries POSIX `df -k -P` (+ Linux `/proc/mounts` `tmpfs` detection) on Unix
//!   and Win32 `GetDiskFreeSpaceExW` on Windows.

use crate::detect;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

const NOTIFICATION_TIMEOUT: Duration = Duration::from_secs(6);
const DISK_SPACE_TIMEOUT: Duration = Duration::from_secs(5);

/// 20 GiB threshold below which Blu-ray remuxing is likely to fail mid-disc.
pub const CRITICAL_FREE_BYTES: u64 = 20 * 1024 * 1024 * 1024;
/// 80 GiB threshold recommended for comfortable dual-layer / UHD Blu-ray staging + encoding.
pub const LOW_FREE_BYTES: u64 = 80 * 1024 * 1024 * 1024;

/// Headroom classification for a staging or output storage volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiskSpaceSeverity {
    Ok,
    Low,
    Critical,
}

/// Free/total storage telemetry for a configured RipDaddy directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskSpaceInfo {
    pub path: String,
    pub resolved_mount: String,
    pub available_bytes: u64,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub is_tmpfs: bool,
    pub severity: DiskSpaceSeverity,
}

/// Combined Temp staging and Output library free-space snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageSpaceReport {
    pub temp: Option<DiskSpaceInfo>,
    pub output: Option<DiskSpaceInfo>,
    pub same_volume: bool,
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
const APPIMAGE_DROP_VARS: &[&str] = &["APPDIR", "APPIMAGE", "ARGV0", "OWD"];

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
const APPIMAGE_PATH_LIST_VARS: &[&str] = &[
    "LD_LIBRARY_PATH",
    "LD_PRELOAD",
    "PATH",
    "XDG_DATA_DIRS",
    "XDG_CONFIG_DIRS",
    "GIO_EXTRA_MODULES",
    "GTK_PATH",
    "GST_PLUGIN_PATH",
    "GST_PLUGIN_SYSTEM_PATH",
    "GST_PLUGIN_SYSTEM_PATH_1_0",
    "QT_PLUGIN_PATH",
    "QML2_IMPORT_PATH",
    "LIBVA_DRIVERS_PATH",
    "LIBGL_DRIVERS_PATH",
    "GCONV_PATH",
    "PYTHONPATH",
];

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn is_appimage_path_entry(entry: &str, appdirs: &[String]) -> bool {
    let trimmed = entry.trim_end_matches('/');
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.contains("/.mount_") {
        return true;
    }
    appdirs.iter().any(|dir| {
        trimmed == dir.as_str()
            || (trimmed.len() > dir.len()
                && trimmed.starts_with(dir.as_str())
                && trimmed.as_bytes()[dir.len()] == b'/')
    })
}

/// Computes environment variable updates needed so host CLI binaries spawned from a Linux
/// AppImage (e.g. `flatpak`, `makemkvcon`, `HandBrakeCLI`, `notify-send`, `xdg-open`, `vainfo`)
/// do not inherit the AppImage's bundled Ubuntu libraries (`LD_LIBRARY_PATH`, GIO/GTK/GSettings
/// paths, etc.) and crash with GLib/GTK symbol lookup errors on newer host distros.
///
/// Returns `(key, None)` to remove a variable or `(key, Some(cleaned))` to replace it.
/// Returns an empty list when not running inside an AppImage.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
pub fn compute_appimage_env_sanitization<I>(vars: I) -> Vec<(String, Option<String>)>
where
    I: IntoIterator<Item = (String, String)>,
{
    let env_list: Vec<(String, String)> = vars.into_iter().collect();

    let mut appdirs: Vec<String> = Vec::new();
    let mut has_appimage = false;
    for (k, v) in &env_list {
        if k == "APPDIR" {
            let norm = v.trim().trim_end_matches('/').to_string();
            if !norm.is_empty() {
                has_appimage = true;
                if let Ok(canon) = std::fs::canonicalize(&norm) {
                    let canon_str = canon.to_string_lossy().trim_end_matches('/').to_string();
                    if !canon_str.is_empty() && canon_str != norm && !appdirs.contains(&canon_str) {
                        appdirs.push(canon_str);
                    }
                }
                if !appdirs.contains(&norm) {
                    appdirs.push(norm);
                }
            }
        } else if k == "APPIMAGE" && !v.trim().is_empty() {
            has_appimage = true;
        } else if k == "LD_LIBRARY_PATH" && v.contains("/.mount_") {
            has_appimage = true;
        }
    }

    if !has_appimage {
        return Vec::new();
    }

    let mut ops: Vec<(String, Option<String>)> = Vec::new();
    for (key, val) in env_list {
        if matches!(key.as_str(), "PWD" | "OLDPWD" | "_") {
            continue;
        }

        if APPIMAGE_DROP_VARS.contains(&key.as_str()) {
            ops.push((key, None));
            continue;
        }

        if key == "LD_PRELOAD" {
            let mut had_appimage = false;
            let kept: Vec<&str> = val
                .split(|c: char| c == ':' || c.is_ascii_whitespace())
                .filter(|seg| {
                    if seg.is_empty() {
                        return false;
                    }
                    if is_appimage_path_entry(seg, &appdirs) {
                        had_appimage = true;
                        false
                    } else {
                        true
                    }
                })
                .collect();
            if had_appimage {
                if kept.is_empty() {
                    ops.push((key, None));
                } else {
                    ops.push((key, Some(kept.join(":"))));
                }
            }
            continue;
        }

        let is_known_list = APPIMAGE_PATH_LIST_VARS.contains(&key.as_str());
        if is_known_list || val.contains(':') {
            let mut had_appimage = false;
            let kept: Vec<&str> = val
                .split(':')
                .filter(|seg| {
                    if seg.is_empty() {
                        return false;
                    }
                    if is_appimage_path_entry(seg, &appdirs) {
                        had_appimage = true;
                        false
                    } else {
                        true
                    }
                })
                .collect();
            if had_appimage {
                if kept.is_empty() {
                    ops.push((key, None));
                } else {
                    ops.push((key, Some(kept.join(":"))));
                }
            }
            continue;
        }

        if is_appimage_path_entry(&val, &appdirs) {
            ops.push((key, None));
        }
    }

    ops
}

/// Applies platform-specific configuration to an external `tokio::process::Command`:
/// - Windows: hides console window flash (`CREATE_NO_WINDOW`).
/// - Linux: strips AppImage-injected library & runtime env vars (`LD_LIBRARY_PATH`, GIO/GTK
///   paths, etc.) so host system binaries run against the host's own shared libraries.
pub fn configure_external_command(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    #[cfg(target_os = "linux")]
    {
        let vars = std::env::vars_os().filter_map(|(k, v)| {
            Some((k.into_string().ok()?, v.into_string().ok()?))
        });
        for (key, value) in compute_appimage_env_sanitization(vars) {
            match value {
                Some(cleaned) => {
                    cmd.env(key, cleaned);
                }
                None => {
                    cmd.env_remove(key);
                }
            }
        }
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = cmd;
    }
}

/// Applies platform-specific configuration to an external `std::process::Command`
/// (Linux AppImage env sanitization; Windows `CREATE_NO_WINDOW`).
pub fn configure_external_std_command(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    #[cfg(target_os = "linux")]
    {
        let vars = std::env::vars_os().filter_map(|(k, v)| {
            Some((k.into_string().ok()?, v.into_string().ok()?))
        });
        for (key, value) in compute_appimage_env_sanitization(vars) {
            match value {
                Some(cleaned) => {
                    cmd.env(key, cleaned);
                }
                None => {
                    cmd.env_remove(key);
                }
            }
        }
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = cmd;
    }
}

/// Classifies free disk space severity for Blu-ray / DVD ripping & encoding workloads.
pub fn classify_disk_severity(available_bytes: u64, is_tmpfs: bool) -> DiskSpaceSeverity {
    if is_tmpfs || available_bytes < CRITICAL_FREE_BYTES {
        DiskSpaceSeverity::Critical
    } else if available_bytes < LOW_FREE_BYTES {
        DiskSpaceSeverity::Low
    } else {
        DiskSpaceSeverity::Ok
    }
}

/// Walks up `path` until an existing ancestor directory is found so non-existent
/// target folders (e.g. before the first rip) still probe the correct underlying volume.
pub fn resolve_existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        if candidate.exists() {
            if candidate.is_dir() {
                return Some(candidate.to_path_buf());
            }
            if let Some(parent) = candidate.parent() {
                return Some(parent.to_path_buf());
            }
        }
        current = candidate.parent();
    }
    None
}

/// Inspects free/total space for both the Temp staging folder and final Output directory.
pub async fn inspect_storage_space(
    temp_dir: Option<&str>,
    output_dir: Option<&str>,
) -> StorageSpaceReport {
    let defaults = detect::detect_default_paths();

    let resolved_temp = temp_dir
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("auto"))
        .unwrap_or(&defaults.temp_dir)
        .to_string();

    let resolved_output = output_dir
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("auto"))
        .unwrap_or(&defaults.output_dir)
        .to_string();

    let (temp, output) = tokio::join!(
        query_disk_space(&resolved_temp),
        query_disk_space(&resolved_output),
    );

    let same_volume = match (&temp, &output) {
        (Some(t), Some(o)) => {
            #[cfg(windows)]
            {
                t.resolved_mount.eq_ignore_ascii_case(&o.resolved_mount)
            }
            #[cfg(not(windows))]
            {
                t.resolved_mount == o.resolved_mount
            }
        }
        _ => false,
    };

    StorageSpaceReport {
        temp,
        output,
        same_volume,
    }
}

/// Queries free/total bytes and mount point for a single filesystem path.
pub async fn query_disk_space(target_path: &str) -> Option<DiskSpaceInfo> {
    let trimmed = target_path.trim();
    if trimmed.is_empty() {
        return None;
    }
    let existing = resolve_existing_ancestor(Path::new(trimmed))?;

    #[cfg(unix)]
    {
        query_disk_space_unix(trimmed, &existing).await
    }

    #[cfg(windows)]
    {
        query_disk_space_windows(trimmed, &existing)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = existing;
        None
    }
}

/// Parses POSIX `df -k -P` output into `(total_bytes, used_bytes, available_bytes, mount_point)`.
/// Handles filesystem names or mount paths containing spaces by anchoring on the `Capacity%` column.
pub fn parse_posix_df_output(output: &str) -> Option<(u64, u64, u64, String)> {
    let data_line = output
        .lines()
        .skip(1)
        .map(str::trim)
        .find(|line| !line.is_empty())?;

    let mut tokens: Vec<(usize, usize, &str)> = Vec::new();
    let mut in_token = false;
    let mut start = 0usize;
    for (idx, ch) in data_line.char_indices() {
        if ch.is_whitespace() {
            if in_token {
                tokens.push((start, idx, &data_line[start..idx]));
                in_token = false;
            }
        } else if !in_token {
            start = idx;
            in_token = true;
        }
    }
    if in_token {
        tokens.push((start, data_line.len(), &data_line[start..]));
    }

    // Locate the capacity column (e.g. `43%` or `-`) preceded by 3 integer block columns.
    for i in 3..tokens.len() {
        let cap_tok = tokens[i].2;
        let is_capacity = (cap_tok.ends_with('%')
            && cap_tok.len() >= 2
            && cap_tok[..cap_tok.len() - 1]
                .chars()
                .all(|c| c.is_ascii_digit()))
            || cap_tok == "-";
        if !is_capacity {
            continue;
        }
        let Ok(total_kb) = tokens[i - 3].2.parse::<u64>() else {
            continue;
        };
        let Ok(used_kb) = tokens[i - 2].2.parse::<u64>() else {
            continue;
        };
        let Ok(avail_kb) = tokens[i - 1].2.parse::<u64>() else {
            continue;
        };
        let mount = data_line[tokens[i].1..].trim().to_string();
        if mount.is_empty() {
            continue;
        }
        return Some((
            total_kb.saturating_mul(1024),
            used_kb.saturating_mul(1024),
            avail_kb.saturating_mul(1024),
            mount,
        ));
    }

    None
}

#[cfg(unix)]
async fn query_disk_space_unix(original_path: &str, existing: &Path) -> Option<DiskSpaceInfo> {
    let mut cmd = Command::new("df");
    cmd.args(["-k", "-P", "--"])
        .arg(existing)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    configure_external_command(&mut cmd);

    let child = cmd.spawn().ok()?;
    let output = timeout(DISK_SPACE_TIMEOUT, child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let (total_bytes, used_bytes, available_bytes, mount) = parse_posix_df_output(&stdout)?;
    let is_tmpfs = detect_mount_is_tmpfs(&mount);
    let severity = classify_disk_severity(available_bytes, is_tmpfs);

    Some(DiskSpaceInfo {
        path: original_path.to_string(),
        resolved_mount: mount,
        available_bytes,
        total_bytes,
        used_bytes,
        is_tmpfs,
        severity,
    })
}

#[cfg(target_os = "linux")]
fn detect_mount_is_tmpfs(mount_point: &str) -> bool {
    let Ok(contents) = std::fs::read_to_string("/proc/mounts") else {
        return false;
    };
    parse_proc_mounts_is_tmpfs(&contents, mount_point)
}

#[cfg(all(unix, not(target_os = "linux")))]
fn detect_mount_is_tmpfs(_mount_point: &str) -> bool {
    false
}

/// Checks `/proc/mounts` text to determine whether the given mount point is RAM-backed (`tmpfs`/`ramfs`).
pub fn parse_proc_mounts_is_tmpfs(proc_mounts: &str, mount_point: &str) -> bool {
    let mut matched_tmpfs = false;
    for line in proc_mounts.lines() {
        let mut parts = line.split_whitespace();
        let Some(_device) = parts.next() else {
            continue;
        };
        let Some(raw_mount) = parts.next() else {
            continue;
        };
        let Some(fs_type) = parts.next() else {
            continue;
        };
        let unescaped_mount = raw_mount.replace("\\040", " ");
        if unescaped_mount == mount_point {
            matched_tmpfs = matches!(fs_type, "tmpfs" | "ramfs");
        }
    }
    matched_tmpfs
}

#[cfg(windows)]
fn query_disk_space_windows(original_path: &str, existing: &Path) -> Option<DiskSpaceInfo> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(
            lpDirectoryName: *const u16,
            lpFreeBytesAvailableToCaller: *mut u64,
            lpTotalNumberOfBytes: *mut u64,
            lpTotalNumberOfFreeBytes: *mut u64,
        ) -> i32;
    }

    let wide: Vec<u16> = OsStr::new(existing)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut avail: u64 = 0;
    let mut total: u64 = 0;
    let mut _total_free: u64 = 0;

    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut avail as *mut u64,
            &mut total as *mut u64,
            &mut _total_free as *mut u64,
        )
    };
    if ok == 0 {
        return None;
    }

    let used = total.saturating_sub(avail);
    let mount = existing
        .components()
        .next()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .unwrap_or_else(|| existing.to_string_lossy().into_owned());
    let severity = classify_disk_severity(avail, false);

    Some(DiskSpaceInfo {
        path: original_path.to_string(),
        resolved_mount: mount,
        available_bytes: avail,
        total_bytes: total,
        used_bytes: used,
        is_tmpfs: false,
        severity,
    })
}

/// Sends a native OS desktop notification (non-blocking, bounded timeout).
pub async fn send_desktop_notification(title: &str, body: &str) -> Result<(), String> {
    let clean_title = title.trim();
    let clean_body = body.trim();
    if clean_title.is_empty() && clean_body.is_empty() {
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(resolved) = detect::locate_optional_tool("notify-send") {
            let mut cmd = Command::new(&resolved.program);
            cmd.args(&resolved.prefix_args)
                .args([
                    "--app-name=RipDaddy",
                    "--icon=media-optical",
                    "--",
                    clean_title,
                    clean_body,
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            configure_external_command(&mut cmd);

            if let Ok(child) = cmd.spawn() {
                if let Ok(Ok(out)) = timeout(NOTIFICATION_TIMEOUT, child.wait_with_output()).await {
                    if out.status.success() {
                        return Ok(());
                    }
                }
            }
        }

        if let Some(resolved) = detect::locate_optional_tool("gdbus") {
            let mut cmd = Command::new(&resolved.program);
            cmd.args(&resolved.prefix_args)
                .args([
                    "call",
                    "--session",
                    "--dest=org.freedesktop.Notifications",
                    "--object-path=/org/freedesktop/Notifications",
                    "--method=org.freedesktop.Notifications.Notify",
                    "RipDaddy",
                    "0",
                    "media-optical",
                    clean_title,
                    clean_body,
                    "[]",
                    "{}",
                    "5000",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            configure_external_command(&mut cmd);

            let child = cmd
                .spawn()
                .map_err(|err| format!("Failed launching gdbus notification: {err}"))?;
            let out = timeout(NOTIFICATION_TIMEOUT, child.wait_with_output())
                .await
                .map_err(|_| "Desktop notification timed out.".to_string())?
                .map_err(|err| format!("Failed waiting for gdbus notification: {err}"))?;
            if out.status.success() {
                return Ok(());
            }
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(if stderr.is_empty() {
                "Notification daemon did not accept the alert.".to_string()
            } else {
                stderr
            });
        }

        Err("Neither notify-send nor gdbus is available on this Linux host.".to_string())
    }

    #[cfg(target_os = "macos")]
    {
        let mut cmd = Command::new("osascript");
        cmd.args([
            "-e",
            "on run argv",
            "-e",
            "display notification (item 2 of argv) with title (item 1 of argv) sound name \"Glass\"",
            "-e",
            "end run",
            "--",
            clean_title,
            clean_body,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

        let child = cmd
            .spawn()
            .map_err(|err| format!("Failed launching osascript: {err}"))?;
        let out = timeout(NOTIFICATION_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| "macOS notification timed out.".to_string())?
            .map_err(|err| format!("Failed waiting for osascript: {err}"))?;
        if out.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Err(if stderr.is_empty() {
                "macOS notification failed.".to_string()
            } else {
                stderr
            })
        }
    }

    #[cfg(target_os = "windows")]
    {
        let script = r#"
$title = [System.Security.SecurityElement]::Escape($env:RD_NOTIFY_TITLE)
$body  = [System.Security.SecurityElement]::Escape($env:RD_NOTIFY_BODY)
try {
    [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null
    [Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] | Out-Null
    $xml = New-Object Windows.Data.Xml.Dom.XmlDocument
    $xml.LoadXml("<toast><visual><binding template='ToastText02'><text id='1'>$title</text><text id='2'>$body</text></binding></visual></toast>")
    $toast = [Windows.UI.Notifications.ToastNotification]::new($xml)
    [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier("RipDaddy").Show($toast)
} catch {
    Add-Type -AssemblyName System.Windows.Forms
    $ni = New-Object System.Windows.Forms.NotifyIcon
    $ni.Icon = [System.Drawing.SystemIcons]::Information
    $ni.BalloonTipTitle = $env:RD_NOTIFY_TITLE
    $ni.BalloonTipText = $env:RD_NOTIFY_BODY
    $ni.Visible = $true
    $ni.ShowBalloonTip(5000)
    Start-Sleep -Milliseconds 1500
    $ni.Dispose()
}
"#;
        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", script])
            .env("RD_NOTIFY_TITLE", clean_title)
            .env("RD_NOTIFY_BODY", clean_body)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        configure_external_command(&mut cmd);

        let child = cmd
            .spawn()
            .map_err(|err| format!("Failed launching PowerShell notification: {err}"))?;
        let out = timeout(NOTIFICATION_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| "Windows notification timed out.".to_string())?
            .map_err(|err| format!("Failed waiting for Windows notification: {err}"))?;
        if out.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Err(if stderr.is_empty() {
                "Windows toast notification failed.".to_string()
            } else {
                stderr
            })
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = (clean_title, clean_body);
        Err("Desktop notifications are not supported on this OS.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_linux_and_macos_posix_df_output_including_spaces_in_mounts() {
        let linux_df = "\
Filesystem     1024-blocks      Used Available Capacity Mounted on
/dev/nvme0n1p3   975429632 412345678 560123456      43% /home
";
        let (total, used, avail, mount) = parse_posix_df_output(linux_df).expect("parse linux df");
        assert_eq!(total, 975_429_632 * 1024);
        assert_eq!(used, 412_345_678 * 1024);
        assert_eq!(avail, 560_123_456 * 1024);
        assert_eq!(mount, "/home");

        let macos_df = "\
Filesystem   1024-blocks       Used  Available Capacity  Mounted on
/dev/disk4s1  3907000000 1000000000 2907000000      26%  /Volumes/Media Archive Drive
";
        let (_, _, avail_mac, mount_mac) = parse_posix_df_output(macos_df).expect("parse mac df");
        assert_eq!(avail_mac, 2_907_000_000 * 1024);
        assert_eq!(mount_mac, "/Volumes/Media Archive Drive");
    }

    #[test]
    fn detects_tmpfs_mounts_from_proc_mounts() {
        let sample = "\
/dev/nvme0n1p3 /home btrfs rw,relatime 0 0
tmpfs /tmp tmpfs rw,nosuid,nodev 0 0
tmpfs /mnt/ram\\040disk tmpfs rw 0 0
";
        assert!(parse_proc_mounts_is_tmpfs(sample, "/tmp"));
        assert!(parse_proc_mounts_is_tmpfs(sample, "/mnt/ram disk"));
        assert!(!parse_proc_mounts_is_tmpfs(sample, "/home"));
    }

    #[test]
    fn classifies_blu_ray_disk_space_thresholds() {
        assert_eq!(
            classify_disk_severity(120 * 1024 * 1024 * 1024, false),
            DiskSpaceSeverity::Ok
        );
        assert_eq!(
            classify_disk_severity(45 * 1024 * 1024 * 1024, false),
            DiskSpaceSeverity::Low
        );
        assert_eq!(
            classify_disk_severity(12 * 1024 * 1024 * 1024, false),
            DiskSpaceSeverity::Critical
        );
        assert_eq!(
            classify_disk_severity(200 * 1024 * 1024 * 1024, true),
            DiskSpaceSeverity::Critical
        );
    }

    #[test]
    fn sanitizes_appimage_env_without_affecting_normal_host_runs() {
        // Non-AppImage environments remain completely untouched.
        let normal_env = vec![
            ("LD_LIBRARY_PATH".to_string(), "/opt/custom/lib".to_string()),
            ("PATH".to_string(), "/usr/local/bin:/usr/bin".to_string()),
            ("HOME".to_string(), "/home/seth".to_string()),
        ];
        assert!(compute_appimage_env_sanitization(normal_env).is_empty());

        // AppImage runtime env strips bundled Ubuntu libs & GTK/GIO vars while preserving user paths.
        let appdir = "/tmp/.mount_RipDadABC123";
        let appimage_env = vec![
            ("APPDIR".to_string(), appdir.to_string()),
            (
                "APPIMAGE".to_string(),
                "/home/seth/Downloads/RipDaddy_0.1.1_amd64.AppImage".to_string(),
            ),
            (
                "LD_LIBRARY_PATH".to_string(),
                format!("{appdir}/usr/lib:{appdir}/usr/lib/x86_64-linux-gnu:"),
            ),
            (
                "PATH".to_string(),
                format!("{appdir}/usr/bin:/usr/local/bin:/usr/bin"),
            ),
            (
                "XDG_DATA_DIRS".to_string(),
                format!("{appdir}/usr/share:/usr/local/share:/usr/share"),
            ),
            (
                "GIO_MODULE_DIR".to_string(),
                format!("{appdir}/usr/lib/gio/modules"),
            ),
            (
                "GSETTINGS_SCHEMA_DIR".to_string(),
                format!("{appdir}/usr/share/glib-2.0/schemas"),
            ),
            (
                "GDK_PIXBUF_MODULE_FILE".to_string(),
                format!("{appdir}/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"),
            ),
            ("HOME".to_string(), "/home/seth".to_string()),
        ];

        let ops: std::collections::HashMap<String, Option<String>> =
            compute_appimage_env_sanitization(appimage_env)
                .into_iter()
                .collect();

        assert_eq!(ops.get("APPDIR"), Some(&None));
        assert_eq!(ops.get("APPIMAGE"), Some(&None));
        assert_eq!(ops.get("LD_LIBRARY_PATH"), Some(&None));
        assert_eq!(
            ops.get("PATH"),
            Some(&Some("/usr/local/bin:/usr/bin".to_string()))
        );
        assert_eq!(
            ops.get("XDG_DATA_DIRS"),
            Some(&Some("/usr/local/share:/usr/share".to_string()))
        );
        assert_eq!(ops.get("GIO_MODULE_DIR"), Some(&None));
        assert_eq!(ops.get("GSETTINGS_SCHEMA_DIR"), Some(&None));
        assert_eq!(ops.get("GDK_PIXBUF_MODULE_FILE"), Some(&None));
        assert!(!ops.contains_key("HOME"));

        // Preserves user-supplied LD_LIBRARY_PATH segments outside the AppImage.
        let mixed_ld = vec![
            ("APPDIR".to_string(), appdir.to_string()),
            (
                "LD_LIBRARY_PATH".to_string(),
                format!("{appdir}/usr/lib:/opt/cuda/lib64"),
            ),
        ];
        let mixed_ops: std::collections::HashMap<String, Option<String>> =
            compute_appimage_env_sanitization(mixed_ld)
                .into_iter()
                .collect();
        assert_eq!(
            mixed_ops.get("LD_LIBRARY_PATH"),
            Some(&Some("/opt/cuda/lib64".to_string()))
        );
    }
}
