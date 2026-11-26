//! Cross-platform locator, OS package-manager auto-setup runner, and version probe for
//! external user-installed binaries.
//!
//! RipDaddy does NOT bundle, download, or distribute `makemkvcon` or `HandBrakeCLI`.
//! Instead, it discovers existing installations on `PATH`, standard OS directories,
//! Linux Flatpaks (`fr.handbrake.ghb`, `com.makemkv.MakeMKV`), and (when developing
//! inside a Linux `toolbox`/`distrobox` container) delegates to host binaries via
//! `flatpak-spawn --host`.

use serde::{Deserialize, Serialize};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

/// Timeout applied to external binary version checks (generous enough for cold Flatpak start).
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(8);

/// Maximum time allowed for an OS package-manager install (`winget`/`brew`/`flatpak`).
const PACKAGE_INSTALL_TIMEOUT: Duration = Duration::from_secs(300);

const MAKEMKV_OFFICIAL_URL: &str = "https://www.makemkv.com/download/";
const HANDBRAKE_OFFICIAL_URL: &str = "https://handbrake.fr/downloads2.php";

/// Identifies which external CLI dependency is being probed or set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BinaryKind {
    MakeMkv,
    HandBrake,
}

impl BinaryKind {
    /// Human-readable canonical binary name.
    pub fn display_name(self) -> &'static str {
        match self {
            BinaryKind::MakeMkv => "makemkvcon",
            BinaryKind::HandBrake => "HandBrakeCLI",
        }
    }

    /// Candidate executable filenames searched on `PATH` for the current platform.
    pub fn executable_names(self) -> &'static [&'static str] {
        #[cfg(target_os = "windows")]
        {
            match self {
                BinaryKind::MakeMkv => &["makemkvcon64.exe", "makemkvcon.exe"],
                BinaryKind::HandBrake => &["HandBrakeCLI.exe"],
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            match self {
                BinaryKind::MakeMkv => &["makemkvcon"],
                BinaryKind::HandBrake => &["HandBrakeCLI"],
            }
        }
    }
}

/// How an external binary path was resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BinarySource {
    CustomPath,
    SystemPath,
    StandardInstallDir,
    Flatpak,
    NotFound,
}

/// Native package managers detected on the user's host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PackageManagerKind {
    Winget,
    Brew,
    Dnf,
    Apt,
    Pacman,
    Flatpak,
}

/// Resolved command invocation (program + fixed prefix args) used to launch the CLI.
/// Supports direct binaries, Linux Flatpak (`flatpak run --command=...`), and
/// toolbox host delegation (`flatpak-spawn --host ...`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedCommand {
    pub program: String,
    pub prefix_args: Vec<String>,
}

impl ResolvedCommand {
    /// Returns a copy of this command with extra Flatpak `--filesystem=<dir>` mounts
    /// injected before the Flatpak app ID when running under `flatpak run`.
    /// No-op for native binaries.
    pub fn with_flatpak_paths(&self, dirs: &[&Path]) -> Self {
        let is_flatpak = self.program.ends_with("flatpak")
            || self.prefix_args.iter().any(|a| a == "flatpak");
        if !is_flatpak {
            return self.clone();
        }
        let Some(run_idx) = self.prefix_args.iter().position(|a| a == "run") else {
            return self.clone();
        };
        let mut next_args = Vec::with_capacity(self.prefix_args.len() + dirs.len());
        next_args.extend_from_slice(&self.prefix_args[..=run_idx]);
        for dir in dirs {
            if let Some(dir_str) = dir.to_str() {
                if !dir_str.is_empty() {
                    let flag = format!("--filesystem={dir_str}");
                    if !next_args.contains(&flag) {
                        next_args.push(flag);
                    }
                }
            }
        }
        next_args.extend_from_slice(&self.prefix_args[run_idx + 1..]);
        Self {
            program: self.program.clone(),
            prefix_args: next_args,
        }
    }
}

/// Platform-tailored guidance for installing a missing external CLI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallHint {
    pub package_manager: Option<PackageManagerKind>,
    pub command: String,
    pub official_url: String,
    pub can_auto_install: bool,
    pub note: String,
}

/// Auto-detected default staging (temp MKV) and final output directories for this OS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultPaths {
    pub temp_dir: String,
    pub output_dir: String,
}

/// Detection and version-probe status for a single external binary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinaryStatus {
    pub name: String,
    pub found: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub source: BinarySource,
    pub error: Option<String>,
    pub command: Option<ResolvedCommand>,
    pub install_hint: InstallHint,
}

/// Optional user-specified executable path overrides passed from frontend settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinaryPathsConfig {
    pub makemkv_path: Option<String>,
    pub handbrake_path: Option<String>,
}

/// Aggregated detection result returned by the `check_binaries` Tauri command.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinariesCheckResult {
    pub makemkv: BinaryStatus,
    pub handbrake: BinaryStatus,
    pub platform: String,
    pub available_package_managers: Vec<PackageManagerKind>,
    pub default_paths: DefaultPaths,
}

/// Outcome of a single binary setup attempt via the host's native package manager.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinaryInstallOutcome {
    pub binary: BinaryKind,
    pub attempted: bool,
    pub succeeded: bool,
    pub message: String,
}

/// Combined result returned after auto-setting up missing CLIs and re-probing paths.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoSetupResult {
    pub outcomes: Vec<BinaryInstallOutcome>,
    pub binaries: BinariesCheckResult,
}

/// Checks both `makemkvcon` and `HandBrakeCLI` concurrently, plus host package managers
/// and default directories.
pub async fn check_binaries(config: BinaryPathsConfig) -> BinariesCheckResult {
    let available_package_managers = detect_package_managers();
    let default_paths = detect_default_paths();

    let (makemkv, handbrake) = tokio::join!(
        check_single_binary(
            BinaryKind::MakeMkv,
            config.makemkv_path,
            &available_package_managers,
        ),
        check_single_binary(
            BinaryKind::HandBrake,
            config.handbrake_path,
            &available_package_managers,
        ),
    );

    BinariesCheckResult {
        makemkv,
        handbrake,
        platform: env::consts::OS.to_string(),
        available_package_managers,
        default_paths,
    }
}

/// Locates a single external binary (native or Flatpak) and verifies it via version probe.
pub async fn check_single_binary(
    kind: BinaryKind,
    custom_path: Option<String>,
    available_managers: &[PackageManagerKind],
) -> BinaryStatus {
    let name = kind.display_name().to_string();
    let install_hint = build_install_hint(kind, env::consts::OS, available_managers);
    let trimmed_custom = custom_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let candidates = match locate_binary_candidates(kind, trimmed_custom) {
        Ok(list) => list,
        Err(err) => {
            return BinaryStatus {
                name,
                found: false,
                path: trimmed_custom.map(str::to_string),
                version: None,
                source: if trimmed_custom.is_some() {
                    BinarySource::CustomPath
                } else {
                    BinarySource::NotFound
                },
                error: Some(err),
                command: None,
                install_hint,
            };
        }
    };

    let mut first_failure: Option<(String, ResolvedCommand, BinarySource, String)> = None;

    for (display_path, resolved_cmd, source) in candidates {
        match probe_version(kind, &resolved_cmd).await {
            Ok(version) => {
                return BinaryStatus {
                    name,
                    found: true,
                    path: Some(display_path),
                    version: Some(version),
                    source,
                    error: None,
                    command: Some(resolved_cmd),
                    install_hint,
                };
            }
            Err(err) => {
                // If running inside a Linux toolbox/distrobox container and a host binary
                // failed to load host shared libraries (e.g., newer host FFmpeg), retry
                // transparently via `flatpak-spawn --host`.
                #[cfg(target_os = "linux")]
                if source != BinarySource::Flatpak {
                    if let Some(host_cmd) = wrap_for_container_host(&resolved_cmd) {
                        if let Ok(version) = probe_version(kind, &host_cmd).await {
                            return BinaryStatus {
                                name,
                                found: true,
                                path: Some(display_path),
                                version: Some(version),
                                source,
                                error: None,
                                command: Some(host_cmd),
                                install_hint,
                            };
                        }
                    }
                }

                if first_failure.is_none() {
                    first_failure = Some((display_path, resolved_cmd, source, err));
                }
            }
        }
    }

    if let Some((display_path, resolved_cmd, source, err)) = first_failure {
        BinaryStatus {
            name,
            found: false,
            path: Some(display_path),
            version: None,
            source,
            error: Some(err),
            command: Some(resolved_cmd),
            install_hint,
        }
    } else {
        BinaryStatus {
            name,
            found: false,
            path: None,
            version: None,
            source: BinarySource::NotFound,
            error: Some(format!("{} was not found.", kind.display_name())),
            command: None,
            install_hint,
        }
    }
}

/// Detects sensible disk-backed default staging (temp MKV) and final output directories.
///
/// Avoids `/tmp` on Linux because many distros mount `/tmp` as RAM-backed `tmpfs`,
/// which can exhaust memory on 30–80 GB Blu-ray rips and is isolated inside Flatpak.
pub fn detect_default_paths() -> DefaultPaths {
    let home = home_dir();
    let output_dir = if let Some(xdg_videos) = env::var_os("XDG_VIDEOS_DIR") {
        PathBuf::from(xdg_videos).join("RipDaddy")
    } else if let Some(ref home_path) = home {
        home_path.join("Videos").join("RipDaddy")
    } else {
        env::temp_dir().join("ripdaddy-output")
    };

    let temp_dir = if let Some(ref home_path) = home {
        #[cfg(target_os = "macos")]
        {
            home_path.join("Library/Caches/RipDaddy/staging")
        }
        #[cfg(target_os = "windows")]
        {
            env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| home_path.clone())
                .join("RipDaddy")
                .join("staging")
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home_path.join(".cache"))
                .join("ripdaddy")
                .join("staging")
        }
    } else {
        env::temp_dir().join("ripdaddy")
    };

    DefaultPaths {
        temp_dir: temp_dir.to_string_lossy().into_owned(),
        output_dir: output_dir.to_string_lossy().into_owned(),
    }
}

/// Detects available native package managers on the host without invoking them.
pub fn detect_package_managers() -> Vec<PackageManagerKind> {
    let mut found = Vec::new();

    #[cfg(target_os = "windows")]
    {
        if find_on_path(&["winget.exe", "winget"]).is_some() {
            found.push(PackageManagerKind::Winget);
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        if find_brew_binary().is_some() {
            found.push(PackageManagerKind::Brew);
        }
    }

    #[cfg(target_os = "linux")]
    {
        if has_linux_flatpak() {
            found.push(PackageManagerKind::Flatpak);
        }
        if find_on_path(&["dnf"]).is_some() {
            found.push(PackageManagerKind::Dnf);
        }
        if find_on_path(&["apt-get", "apt"]).is_some() {
            found.push(PackageManagerKind::Apt);
        }
        if find_on_path(&["pacman"]).is_some() {
            found.push(PackageManagerKind::Pacman);
        }
    }

    found
}

/// Produces platform-specific CLI setup guidance based on the detected OS and package managers.
pub fn build_install_hint(
    kind: BinaryKind,
    platform: &str,
    available_managers: &[PackageManagerKind],
) -> InstallHint {
    let has = |pm: PackageManagerKind| available_managers.contains(&pm);

    match (kind, platform) {
        (BinaryKind::HandBrake, "windows") => {
            if has(PackageManagerKind::Winget) {
                InstallHint {
                    package_manager: Some(PackageManagerKind::Winget),
                    command: "winget install --id HandBrake.HandBrake.CLI --exact".to_string(),
                    official_url: HANDBRAKE_OFFICIAL_URL.to_string(),
                    can_auto_install: true,
                    note: "Auto-installable via Windows winget; RipDaddy re-detects HandBrakeCLI.exe automatically.".to_string(),
                }
            } else {
                InstallHint {
                    package_manager: None,
                    command: "winget install --id HandBrake.HandBrake.CLI --exact".to_string(),
                    official_url: HANDBRAKE_OFFICIAL_URL.to_string(),
                    can_auto_install: false,
                    note: "Install App Installer (winget) or download HandBrakeCLI from handbrake.fr.".to_string(),
                }
            }
        }
        (BinaryKind::MakeMkv, "windows") => {
            if has(PackageManagerKind::Winget) {
                InstallHint {
                    package_manager: Some(PackageManagerKind::Winget),
                    command: "winget install --id GuinpinSoft.MakeMKV --exact".to_string(),
                    official_url: MAKEMKV_OFFICIAL_URL.to_string(),
                    can_auto_install: true,
                    note: "Auto-installable via Windows winget; RipDaddy re-detects makemkvcon64.exe automatically.".to_string(),
                }
            } else {
                InstallHint {
                    package_manager: None,
                    command: "winget install --id GuinpinSoft.MakeMKV --exact".to_string(),
                    official_url: MAKEMKV_OFFICIAL_URL.to_string(),
                    can_auto_install: false,
                    note: "Install MakeMKV from makemkv.com or via winget; RipDaddy auto-detects it on re-check.".to_string(),
                }
            }
        }
        (BinaryKind::HandBrake, "macos") => {
            if has(PackageManagerKind::Brew) {
                InstallHint {
                    package_manager: Some(PackageManagerKind::Brew),
                    command: "brew install handbrake".to_string(),
                    official_url: HANDBRAKE_OFFICIAL_URL.to_string(),
                    can_auto_install: true,
                    note: "Auto-installable via Homebrew; RipDaddy re-detects HandBrakeCLI automatically.".to_string(),
                }
            } else {
                InstallHint {
                    package_manager: None,
                    command: "brew install handbrake".to_string(),
                    official_url: HANDBRAKE_OFFICIAL_URL.to_string(),
                    can_auto_install: false,
                    note: "Install Homebrew or download HandBrakeCLI from handbrake.fr.".to_string(),
                }
            }
        }
        (BinaryKind::MakeMkv, "macos") => {
            if has(PackageManagerKind::Brew) {
                InstallHint {
                    package_manager: Some(PackageManagerKind::Brew),
                    command: "brew install --cask makemkv".to_string(),
                    official_url: MAKEMKV_OFFICIAL_URL.to_string(),
                    can_auto_install: true,
                    note: "Auto-installable via Homebrew Cask; RipDaddy detects /Applications/MakeMKV.app automatically.".to_string(),
                }
            } else {
                InstallHint {
                    package_manager: None,
                    command: "brew install --cask makemkv".to_string(),
                    official_url: MAKEMKV_OFFICIAL_URL.to_string(),
                    can_auto_install: false,
                    note: "Install MakeMKV into /Applications from makemkv.com or install Homebrew.".to_string(),
                }
            }
        }
        (BinaryKind::HandBrake, "linux") => {
            if has(PackageManagerKind::Flatpak) {
                InstallHint {
                    package_manager: Some(PackageManagerKind::Flatpak),
                    command: "flatpak install -y flathub fr.handbrake.ghb".to_string(),
                    official_url: HANDBRAKE_OFFICIAL_URL.to_string(),
                    can_auto_install: true,
                    note: "Flatpak detected — RipDaddy can install HandBrake (includes HandBrakeCLI) via Flathub and auto-detect it.".to_string(),
                }
            } else if has(PackageManagerKind::Brew) {
                InstallHint {
                    package_manager: Some(PackageManagerKind::Brew),
                    command: "brew install handbrake".to_string(),
                    official_url: HANDBRAKE_OFFICIAL_URL.to_string(),
                    can_auto_install: true,
                    note: "Homebrew detected — RipDaddy can run `brew install handbrake` and auto-detect HandBrakeCLI.".to_string(),
                }
            } else if has(PackageManagerKind::Apt) {
                InstallHint {
                    package_manager: Some(PackageManagerKind::Apt),
                    command: "sudo apt update && sudo apt install -y handbrake-cli".to_string(),
                    official_url: HANDBRAKE_OFFICIAL_URL.to_string(),
                    can_auto_install: false,
                    note: "Run this in your terminal (requires sudo), then click Re-check.".to_string(),
                }
            } else if has(PackageManagerKind::Dnf) {
                InstallHint {
                    package_manager: Some(PackageManagerKind::Dnf),
                    command: "sudo dnf install -y HandBrake-cli".to_string(),
                    official_url: HANDBRAKE_OFFICIAL_URL.to_string(),
                    can_auto_install: false,
                    note: "Install HandBrake-cli via RPM Fusion, Flatpak, or Homebrew, then click Re-check.".to_string(),
                }
            } else if has(PackageManagerKind::Pacman) {
                InstallHint {
                    package_manager: Some(PackageManagerKind::Pacman),
                    command: "sudo pacman -S --needed handbrake-cli".to_string(),
                    official_url: HANDBRAKE_OFFICIAL_URL.to_string(),
                    can_auto_install: false,
                    note: "Run this in your terminal (requires sudo), then click Re-check.".to_string(),
                }
            } else {
                InstallHint {
                    package_manager: None,
                    command: "flatpak install -y flathub fr.handbrake.ghb".to_string(),
                    official_url: HANDBRAKE_OFFICIAL_URL.to_string(),
                    can_auto_install: false,
                    note: "Install HandBrakeCLI via Flatpak, Homebrew, or your distro package manager.".to_string(),
                }
            }
        }
        (BinaryKind::MakeMkv, "linux") => {
            if has(PackageManagerKind::Flatpak) {
                InstallHint {
                    package_manager: Some(PackageManagerKind::Flatpak),
                    command: "flatpak install -y flathub com.makemkv.MakeMKV".to_string(),
                    official_url: MAKEMKV_OFFICIAL_URL.to_string(),
                    can_auto_install: true,
                    note: "Flatpak detected — RipDaddy can install MakeMKV (includes makemkvcon) via Flathub or use a local makemkvcon binary.".to_string(),
                }
            } else {
                InstallHint {
                    package_manager: None,
                    command: "See https://www.makemkv.com/download/ for Linux install instructions"
                        .to_string(),
                    official_url: MAKEMKV_OFFICIAL_URL.to_string(),
                    can_auto_install: false,
                    note: "Install makemkvcon locally (e.g. ~/.local/bin or /usr/bin) and RipDaddy will auto-detect it.".to_string(),
                }
            }
        }
        (BinaryKind::MakeMkv, _) => InstallHint {
            package_manager: None,
            command: "See https://www.makemkv.com/download/ for install instructions".to_string(),
            official_url: MAKEMKV_OFFICIAL_URL.to_string(),
            can_auto_install: false,
            note: "Install makemkvcon locally and RipDaddy will auto-detect it.".to_string(),
        },
        (BinaryKind::HandBrake, _) => InstallHint {
            package_manager: None,
            command: "Install HandBrakeCLI from https://handbrake.fr/downloads2.php".to_string(),
            official_url: HANDBRAKE_OFFICIAL_URL.to_string(),
            can_auto_install: false,
            note: "Install HandBrakeCLI and ensure it is on PATH or select its path manually.".to_string(),
        },
    }
}

/// Invokes the host OS's native non-interactive package manager (`winget` on Windows,
/// `brew` on macOS/Linux, or `flatpak` on Linux) to install a missing CLI.
pub async fn install_binary_via_package_manager(kind: BinaryKind) -> Result<String, String> {
    #[cfg(target_os = "linux")]
    if in_luma_capsule() {
        return Err(LUMA_CAPSULE_GUIDANCE.to_string());
    }

    let resolved = resolve_package_manager_install_invocation(kind)?;

    let mut cmd = Command::new(&resolved.program);
    cmd.args(&resolved.prefix_args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    crate::system::configure_external_command(&mut cmd);

    let child = cmd
        .spawn()
        .map_err(|err| format!("Failed to launch {}: {}", resolved.program, err))?;

    let output = timeout(PACKAGE_INSTALL_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| {
            format!(
                "Package manager timed out after {}s while installing {}.",
                PACKAGE_INSTALL_TIMEOUT.as_secs(),
                kind.display_name()
            )
        })?
        .map_err(|err| format!("Failed waiting for package manager: {err}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if output.status.success() {
        Ok(if stdout.is_empty() {
            format!("{} installed successfully.", kind.display_name())
        } else {
            stdout
        })
    } else {
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        Err(format!(
            "Package manager exited with {:?}: {}",
            output.status.code(),
            detail
        ))
    }
}

/// Checks current binaries and automatically invokes the host OS's package manager for
/// any missing CLI marked `can_auto_install`, then re-runs `check_binaries`.
pub async fn auto_setup_missing_binaries(config: BinaryPathsConfig) -> AutoSetupResult {
    let initial = check_binaries(config.clone()).await;
    let mut outcomes = Vec::new();

    for (kind, status) in [
        (BinaryKind::MakeMkv, &initial.makemkv),
        (BinaryKind::HandBrake, &initial.handbrake),
    ] {
        if status.found {
            outcomes.push(BinaryInstallOutcome {
                binary: kind,
                attempted: false,
                succeeded: true,
                message: format!(
                    "{} already installed ({})",
                    status.name,
                    status.version.as_deref().unwrap_or("detected")
                ),
            });
            continue;
        }

        if !status.install_hint.can_auto_install {
            outcomes.push(BinaryInstallOutcome {
                binary: kind,
                attempted: false,
                succeeded: false,
                message: status.install_hint.note.clone(),
            });
            continue;
        }

        match install_binary_via_package_manager(kind).await {
            Ok(msg) => outcomes.push(BinaryInstallOutcome {
                binary: kind,
                attempted: true,
                succeeded: true,
                message: msg,
            }),
            Err(err) => outcomes.push(BinaryInstallOutcome {
                binary: kind,
                attempted: true,
                succeeded: false,
                message: err,
            }),
        }
    }

    let binaries = check_binaries(config).await;
    AutoSetupResult { outcomes, binaries }
}

fn resolve_package_manager_install_invocation(
    kind: BinaryKind,
) -> Result<ResolvedCommand, String> {
    #[cfg(target_os = "windows")]
    {
        let winget = find_on_path(&["winget.exe", "winget"])
            .ok_or_else(|| "Windows winget was not found on PATH.".to_string())?;
        let pkg_id = match kind {
            BinaryKind::HandBrake => "HandBrake.HandBrake.CLI",
            BinaryKind::MakeMkv => "GuinpinSoft.MakeMKV",
        };
        Ok(ResolvedCommand {
            program: winget.to_string_lossy().into_owned(),
            prefix_args: vec![
                "install".to_string(),
                "--id".to_string(),
                pkg_id.to_string(),
                "--exact".to_string(),
                "--accept-source-agreements".to_string(),
                "--accept-package-agreements".to_string(),
            ],
        })
    }

    #[cfg(target_os = "macos")]
    {
        let brew = find_brew_binary()
            .ok_or_else(|| "Homebrew (`brew`) was not found on this Mac.".to_string())?;
        let args = match kind {
            BinaryKind::HandBrake => vec!["install".to_string(), "handbrake".to_string()],
            BinaryKind::MakeMkv => vec![
                "install".to_string(),
                "--cask".to_string(),
                "makemkv".to_string(),
            ],
        };
        Ok(ResolvedCommand {
            program: brew.to_string_lossy().into_owned(),
            prefix_args: args,
        })
    }

    #[cfg(target_os = "linux")]
    {
        if has_linux_flatpak() {
            let app_id = match kind {
                BinaryKind::HandBrake => "fr.handbrake.ghb",
                BinaryKind::MakeMkv => "com.makemkv.MakeMKV",
            };
            return Ok(build_linux_flatpak_command(&[
                "install", "-y", "flathub", app_id,
            ]));
        }

        if kind == BinaryKind::HandBrake {
            if let Some(brew) = find_brew_binary() {
                return Ok(ResolvedCommand {
                    program: brew.to_string_lossy().into_owned(),
                    prefix_args: vec!["install".to_string(), "handbrake".to_string()],
                });
            }
        }

        Err(format!(
            "No non-interactive package manager (Flatpak/Homebrew) available for {}.",
            kind.display_name()
        ))
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = kind;
        Err("Automatic CLI installation is not supported on this operating system.".to_string())
    }
}

#[cfg(target_os = "linux")]
const LUMA_CAPSULE_GUIDANCE: &str = "RipDaddy is running inside a Luma OS capsule sandbox, which hides host CLIs (~/.local/bin, /var/lib/flatpak), optical drives (/dev/sr0), and /run/systemd/resolve DNS. Run the RipDaddy .AppImage directly on your host (chmod +x RipDaddy_*.AppImage && ./RipDaddy_*.AppImage) instead of importing it into Luma's capsule.";

#[cfg(target_os = "linux")]
fn in_luma_capsule() -> bool {
    env::var_os("LUMA_CAPSULE_DESKTOP_ID")
        .is_some_and(|v| !v.is_empty())
}

/// Collects all candidate native executables (across `PATH` and standard install dirs)
/// plus any installed Linux Flatpak fallback, ordered by priority and deduplicated.
fn locate_binary_candidates(
    kind: BinaryKind,
    custom_path: Option<&str>,
) -> Result<Vec<(String, ResolvedCommand, BinarySource)>, String> {
    if let Some(raw) = custom_path {
        let candidate = PathBuf::from(raw);
        if is_executable_file(&candidate) {
            let path_str = candidate.to_string_lossy().into_owned();
            let cmd = ResolvedCommand {
                program: path_str.clone(),
                prefix_args: Vec::new(),
            };
            return Ok(vec![(path_str, cmd, BinarySource::CustomPath)]);
        }
        return Err(format!(
            "Configured {} path does not exist or is not executable: {}",
            kind.display_name(),
            raw
        ));
    }

    let mut results: Vec<(String, ResolvedCommand, BinarySource)> = Vec::new();
    let mut seen_paths: Vec<PathBuf> = Vec::new();

    let mut push_native = |path: PathBuf, source: BinarySource| {
        let dedup_key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if seen_paths.contains(&dedup_key) {
            return;
        }
        seen_paths.push(dedup_key);
        let path_str = path.to_string_lossy().into_owned();
        let cmd = ResolvedCommand {
            program: path_str.clone(),
            prefix_args: Vec::new(),
        };
        results.push((path_str, cmd, source));
    };

    for found in find_all_on_path(kind.executable_names()) {
        push_native(found, BinarySource::SystemPath);
    }

    for candidate in standard_install_candidates(kind) {
        if is_executable_file(&candidate) {
            push_native(candidate, BinarySource::StandardInstallDir);
        }
    }

    #[cfg(target_os = "linux")]
    if let Some((display_label, cmd)) = locate_flatpak_binary(kind) {
        results.push((display_label, cmd, BinarySource::Flatpak));
    }

    if !results.is_empty() {
        return Ok(results);
    }

    #[cfg(target_os = "linux")]
    if in_luma_capsule() {
        return Err(format!(
            "{} could not be seen from inside the Luma capsule. {}",
            kind.display_name(),
            LUMA_CAPSULE_GUIDANCE
        ));
    }

    Err(format!(
        "{} was not found in PATH, standard installation directories, or Flatpak.",
        kind.display_name()
    ))
}

/// Resolves a native executable path by checking:
/// 1. Explicit user-selected custom path (if provided)
/// 2. Directories on the system `PATH`
/// 3. Well-known platform-specific installation directories
pub fn locate_binary(
    kind: BinaryKind,
    custom_path: Option<&str>,
) -> Result<(PathBuf, BinarySource), String> {
    if let Some(raw) = custom_path {
        let candidate = PathBuf::from(raw);
        if is_executable_file(&candidate) {
            return Ok((candidate, BinarySource::CustomPath));
        }
        return Err(format!(
            "Configured {} path does not exist or is not executable: {}",
            kind.display_name(),
            raw
        ));
    }

    if let Some(found) = find_on_path(kind.executable_names()) {
        return Ok((found, BinarySource::SystemPath));
    }

    for candidate in standard_install_candidates(kind) {
        if is_executable_file(&candidate) {
            return Ok((candidate, BinarySource::StandardInstallDir));
        }
    }

    Err(format!(
        "{} was not found in PATH or standard installation directories.",
        kind.display_name()
    ))
}

/// Locates an optional companion CLI tool such as `ffmpeg` or `ffprobe` across
/// `PATH`, common OS install locations, and (inside Linux `toolbox`/`distrobox`
/// containers) the host filesystem via `flatpak-spawn --host`.
pub fn locate_optional_tool(base_name: &str) -> Option<ResolvedCommand> {
    #[cfg(target_os = "windows")]
    let exe_names = [format!("{base_name}.exe")];
    #[cfg(not(target_os = "windows"))]
    let exe_names = [base_name.to_string()];

    let name_refs: Vec<&str> = exe_names.iter().map(String::as_str).collect();
    if let Some(found) = find_on_path(&name_refs) {
        return Some(ResolvedCommand {
            program: found.to_string_lossy().into_owned(),
            prefix_args: Vec::new(),
        });
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    #[cfg(target_os = "windows")]
    {
        let exe = &exe_names[0];
        if let Some(local) = env::var_os("LOCALAPPDATA") {
            let local_path = PathBuf::from(&local);
            candidates.push(local_path.join("Microsoft\\WinGet\\Links").join(exe));
            candidates.push(local_path.join("Programs\\ffmpeg\\bin").join(exe));
        }
        if let Some(profile) = env::var_os("USERPROFILE") {
            candidates.push(PathBuf::from(profile).join("scoop\\shims").join(exe));
        }
        if let Some(pf) = env::var_os("ProgramFiles") {
            candidates.push(PathBuf::from(pf).join("ffmpeg\\bin").join(exe));
        }
        candidates.push(PathBuf::from("C:\\ProgramData\\chocolatey\\bin").join(exe));
        candidates.push(PathBuf::from("C:\\ffmpeg\\bin").join(exe));
    }

    #[cfg(target_os = "macos")]
    {
        for dir in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
            candidates.push(PathBuf::from(dir).join(base_name));
        }
        if let Some(home) = home_dir() {
            candidates.push(home.join(".local/bin").join(base_name));
        }
    }

    #[cfg(target_os = "linux")]
    {
        for dir in [
            "/usr/bin",
            "/usr/local/bin",
            "/home/linuxbrew/.linuxbrew/bin",
        ] {
            candidates.push(PathBuf::from(dir).join(base_name));
        }
        if let Some(home) = home_dir() {
            candidates.push(home.join(".local/bin").join(base_name));
        }
    }

    for candidate in candidates {
        if is_executable_file(&candidate) {
            return Some(ResolvedCommand {
                program: candidate.to_string_lossy().into_owned(),
                prefix_args: Vec::new(),
            });
        }
    }

    // Inside Linux toolbox/distrobox containers, delegate to host binary if installed there.
    #[cfg(target_os = "linux")]
    if in_linux_container() {
        if let Some(spawn_bin) = find_on_path(&["flatpak-spawn"]) {
            for host_path in [
                format!("/usr/bin/{base_name}"),
                format!("/usr/local/bin/{base_name}"),
            ] {
                let run_host_probe = PathBuf::from(format!("/run/host{host_path}"));
                if is_executable_file(&run_host_probe) {
                    return Some(ResolvedCommand {
                        program: spawn_bin.to_string_lossy().into_owned(),
                        prefix_args: vec!["--host".to_string(), host_path],
                    });
                }
            }
        }
    }

    None
}

#[cfg(target_os = "linux")]
fn in_linux_container() -> bool {
    Path::new("/run/.toolboxenv").exists() || Path::new("/run/.containerenv").exists()
}

#[cfg(target_os = "linux")]
fn has_linux_flatpak() -> bool {
    if find_on_path(&["flatpak"]).is_some() {
        return true;
    }
    in_linux_container()
        && find_on_path(&["flatpak-spawn"]).is_some()
        && Path::new("/run/host/usr/bin/flatpak").exists()
}

#[cfg(target_os = "linux")]
fn build_linux_flatpak_command(flatpak_args: &[&str]) -> ResolvedCommand {
    if in_linux_container() {
        if let Some(spawn_bin) = find_on_path(&["flatpak-spawn"]) {
            let mut args = vec!["--host".to_string(), "flatpak".to_string()];
            args.extend(flatpak_args.iter().map(|s| (*s).to_string()));
            return ResolvedCommand {
                program: spawn_bin.to_string_lossy().into_owned(),
                prefix_args: args,
            };
        }
    }

    let flatpak_program = find_on_path(&["flatpak"])
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "flatpak".to_string());
    ResolvedCommand {
        program: flatpak_program,
        prefix_args: flatpak_args.iter().map(|s| (*s).to_string()).collect(),
    }
}

#[cfg(target_os = "linux")]
fn wrap_for_container_host(cmd: &ResolvedCommand) -> Option<ResolvedCommand> {
    if !in_linux_container() {
        return None;
    }
    let spawn_bin = find_on_path(&["flatpak-spawn"])?;
    let mut args = vec!["--host".to_string(), cmd.program.clone()];
    args.extend(cmd.prefix_args.iter().cloned());
    Some(ResolvedCommand {
        program: spawn_bin.to_string_lossy().into_owned(),
        prefix_args: args,
    })
}

#[cfg(target_os = "linux")]
fn locate_flatpak_binary(kind: BinaryKind) -> Option<(String, ResolvedCommand)> {
    if !has_linux_flatpak() {
        return None;
    }

    let (app_id, internal_bin) = match kind {
        BinaryKind::HandBrake => ("fr.handbrake.ghb", "HandBrakeCLI"),
        BinaryKind::MakeMkv => ("com.makemkv.MakeMKV", "makemkvcon"),
    };

    let mut roots = vec![
        PathBuf::from("/var/lib/flatpak/app"),
        PathBuf::from("/run/host/var/lib/flatpak/app"),
    ];
    if let Some(home) = home_dir() {
        roots.push(home.join(".local/share/flatpak/app"));
    }

    let installed = roots.iter().any(|root| {
        let bin_path = root
            .join(app_id)
            .join("current/active/files/bin")
            .join(internal_bin);
        is_executable_file(&bin_path)
    });

    if !installed {
        return None;
    }

    let command_flag = format!("--command={internal_bin}");
    let resolved = build_linux_flatpak_command(&["run", &command_flag, app_id]);
    let display = format!("flatpak://{app_id} ({internal_bin})");
    Some((display, resolved))
}

/// Returns well-known platform-specific installation paths for `makemkvcon` and `HandBrakeCLI`.
pub fn standard_install_candidates(kind: BinaryKind) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    #[cfg(target_os = "windows")]
    {
        let program_files =
            env::var_os("ProgramFiles").unwrap_or_else(|| "C:\\Program Files".into());
        let program_files_x86 =
            env::var_os("ProgramFiles(x86)").unwrap_or_else(|| "C:\\Program Files (x86)".into());
        let local_app_data = env::var_os("LOCALAPPDATA");

        match kind {
            BinaryKind::MakeMkv => {
                candidates.push(PathBuf::from(&program_files_x86).join("MakeMKV\\makemkvcon64.exe"));
                candidates.push(PathBuf::from(&program_files_x86).join("MakeMKV\\makemkvcon.exe"));
                candidates.push(PathBuf::from(&program_files).join("MakeMKV\\makemkvcon64.exe"));
                candidates.push(PathBuf::from(&program_files).join("MakeMKV\\makemkvcon.exe"));
            }
            BinaryKind::HandBrake => {
                candidates.push(PathBuf::from(&program_files).join("HandBrake\\HandBrakeCLI.exe"));
                candidates
                    .push(PathBuf::from(&program_files_x86).join("HandBrake\\HandBrakeCLI.exe"));
                if let Some(local) = local_app_data {
                    candidates
                        .push(PathBuf::from(&local).join("Programs\\HandBrake\\HandBrakeCLI.exe"));
                    candidates.push(
                        PathBuf::from(&local)
                            .join("Microsoft\\WinGet\\Links\\HandBrakeCLI.exe"),
                    );
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let home = home_dir();
        match kind {
            BinaryKind::MakeMkv => {
                candidates.push(PathBuf::from(
                    "/Applications/MakeMKV.app/Contents/MacOS/makemkvcon",
                ));
                if let Some(home_dir) = home {
                    candidates
                        .push(home_dir.join("Applications/MakeMKV.app/Contents/MacOS/makemkvcon"));
                }
                candidates.push(PathBuf::from("/opt/homebrew/bin/makemkvcon"));
                candidates.push(PathBuf::from("/usr/local/bin/makemkvcon"));
            }
            BinaryKind::HandBrake => {
                candidates.push(PathBuf::from("/opt/homebrew/bin/HandBrakeCLI"));
                candidates.push(PathBuf::from("/usr/local/bin/HandBrakeCLI"));
                candidates.push(PathBuf::from("/Applications/HandBrakeCLI"));
                if let Some(home_dir) = home {
                    candidates.push(home_dir.join(".local/bin/HandBrakeCLI"));
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let home = home_dir();
        match kind {
            BinaryKind::MakeMkv => {
                if let Some(home_dir) = home {
                    candidates.push(home_dir.join(".local/bin/makemkvcon"));
                }
                candidates.push(PathBuf::from("/usr/bin/makemkvcon"));
                candidates.push(PathBuf::from("/usr/local/bin/makemkvcon"));
                candidates.push(PathBuf::from("/opt/makemkv/bin/makemkvcon"));
                candidates.push(PathBuf::from("/snap/bin/makemkv.makemkvcon"));
                candidates.push(PathBuf::from("/home/linuxbrew/.linuxbrew/bin/makemkvcon"));
            }
            BinaryKind::HandBrake => {
                if let Some(home_dir) = home {
                    candidates.push(home_dir.join(".local/bin/HandBrakeCLI"));
                }
                candidates.push(PathBuf::from("/usr/bin/HandBrakeCLI"));
                candidates.push(PathBuf::from("/usr/local/bin/HandBrakeCLI"));
                candidates.push(PathBuf::from("/home/linuxbrew/.linuxbrew/bin/HandBrakeCLI"));
            }
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = kind;
    }

    candidates
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        env::var_os("USERPROFILE").map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        env::var_os("HOME").map(PathBuf::from)
    }
}

fn find_on_path(names: &[&str]) -> Option<PathBuf> {
    find_all_on_path(names).into_iter().next()
}

fn find_all_on_path(names: &[&str]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Some(path_var) = env::var_os("PATH") else {
        return found;
    };
    for dir in env::split_paths(&path_var) {
        for exe_name in names {
            let candidate = dir.join(exe_name);
            if is_executable_file(&candidate) && !found.contains(&candidate) {
                found.push(candidate);
            }
        }
    }
    found
}

#[cfg(not(target_os = "windows"))]
fn find_brew_binary() -> Option<PathBuf> {
    if let Some(found) = find_on_path(&["brew"]) {
        return Some(found);
    }
    for candidate in [
        "/opt/homebrew/bin/brew",
        "/usr/local/bin/brew",
        "/home/linuxbrew/.linuxbrew/bin/brew",
        "/usr/bin/brew",
    ] {
        let path = PathBuf::from(candidate);
        if is_executable_file(&path) {
            return Some(path);
        }
    }
    None
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Executes a bounded, non-invasive version check against the resolved command.
async fn probe_version(kind: BinaryKind, resolved: &ResolvedCommand) -> Result<String, String> {
    let mut cmd = Command::new(&resolved.program);
    cmd.args(&resolved.prefix_args);
    match kind {
        BinaryKind::MakeMkv => {
            cmd.args(["-r", "--noscan", "--cache=1", "info", "disc:999"]);
        }
        BinaryKind::HandBrake => {
            cmd.arg("--version");
        }
    }

    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    crate::system::configure_external_command(&mut cmd);

    let child = cmd.spawn().map_err(|err| {
        format!(
            "Failed to launch {} via {}: {}",
            kind.display_name(),
            resolved.program,
            err
        )
    })?;

    let output = timeout(VERSION_PROBE_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| {
            format!(
                "Timed out after {}s while checking {} version",
                VERSION_PROBE_TIMEOUT.as_secs(),
                kind.display_name()
            )
        })?
        .map_err(|err| format!("Failed reading {} output: {}", kind.display_name(), err))?;

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let parsed = match kind {
        BinaryKind::MakeMkv => parse_makemkv_version(&combined),
        BinaryKind::HandBrake => parse_handbrake_version(&combined),
    };

    if let Some(version) = parsed {
        return Ok(version);
    }

    if output.status.success() {
        Ok("Detected (version unrecognized)".to_string())
    } else {
        let snippet = combined
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("");
        if snippet.is_empty() {
            Err(format!(
                "{} exited with status {:?} without reporting a recognizable version.",
                kind.display_name(),
                output.status.code()
            ))
        } else {
            Err(format!(
                "{} failed (exit {:?}): {}",
                kind.display_name(),
                output.status.code(),
                snippet
            ))
        }
    }
}

/// Extracts the version token from `makemkvcon -r` output, e.g. `v1.18.1` or `v2.0.0`.
pub fn parse_makemkv_version(output: &str) -> Option<String> {
    for line in output.lines() {
        if let Some(idx) = line.find("MakeMKV v") {
            let rest = &line[idx + "MakeMKV ".len()..];
            let token = rest
                .split(|c: char| c.is_whitespace() || c == '"' || c == ',')
                .next()
                .unwrap_or("")
                .trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }
    None
}

/// Extracts the version string from `HandBrakeCLI --version` output, e.g. `1.11.2`.
pub fn parse_handbrake_version(output: &str) -> Option<String> {
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("HandBrake ") {
            let version = rest.trim();
            if !version.is_empty() {
                return Some(version.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_makemkv_robot_version() {
        let sample = r#"MSG:1005,0,1,"MakeMKV v2.0.0 linux(x64-release) started","%1 started","MakeMKV v2.0.0 linux(x64-release)""#;
        assert_eq!(parse_makemkv_version(sample), Some("v2.0.0".to_string()));
    }

    #[test]
    fn parses_handbrake_cli_version() {
        let sample = "HandBrake 1.11.2\n";
        assert_eq!(parse_handbrake_version(sample), Some("1.11.2".to_string()));
    }

    #[test]
    fn rejects_nonexistent_custom_binary_path() {
        let result = locate_binary(
            BinaryKind::MakeMkv,
            Some("/nonexistent/ripdaddy/path/makemkvcon"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn install_hints_prefer_flatpak_on_linux_when_available() {
        let linux_hb = build_install_hint(
            BinaryKind::HandBrake,
            "linux",
            &[PackageManagerKind::Flatpak, PackageManagerKind::Dnf],
        );
        assert!(linux_hb.can_auto_install);
        assert_eq!(linux_hb.command, "flatpak install -y flathub fr.handbrake.ghb");

        let linux_mkv = build_install_hint(
            BinaryKind::MakeMkv,
            "linux",
            &[PackageManagerKind::Flatpak],
        );
        assert!(linux_mkv.can_auto_install);
        assert_eq!(linux_mkv.command, "flatpak install -y flathub com.makemkv.MakeMKV");
    }

    #[test]
    fn injects_flatpak_filesystem_mounts_for_native_and_container_commands() {
        let native = ResolvedCommand {
            program: "/usr/bin/HandBrakeCLI".to_string(),
            prefix_args: Vec::new(),
        };
        assert_eq!(
            native.with_flatpak_paths(&[Path::new("/media/rips")]),
            native
        );

        let container_flatpak = ResolvedCommand {
            program: "/usr/bin/flatpak-spawn".to_string(),
            prefix_args: vec![
                "--host".to_string(),
                "flatpak".to_string(),
                "run".to_string(),
                "--command=HandBrakeCLI".to_string(),
                "fr.handbrake.ghb".to_string(),
            ],
        };
        let mounted = container_flatpak
            .with_flatpak_paths(&[Path::new("/mnt/staging"), Path::new("/mnt/staging")]);
        assert_eq!(
            mounted.prefix_args,
            vec![
                "--host",
                "flatpak",
                "run",
                "--filesystem=/mnt/staging",
                "--command=HandBrakeCLI",
                "fr.handbrake.ghb",
            ]
        );
    }
}
