#!/usr/bin/env python3
"""Populate Homebrew Cask and WinGet installer SHA256 checksums for RipDaddy releases.

Usage examples:
  # Populate manifests from a directory containing built release installers:
  python3 scripts/update-packaging-manifests.py --version 0.1.0 --from-dir ./release-assets --write-sums ./release-assets/SHA256SUMS.txt

  # Download installers from an existing GitHub Release (draft or published) via `gh`:
  python3 scripts/update-packaging-manifests.py --version 0.1.0 --from-release

  # Populate manifests directly from known SHA256 digests:
  python3 scripts/update-packaging-manifests.py --version 0.1.0 \
      --arm64-dmg-sha256 <64-hex> \
      --x64-dmg-sha256 <64-hex> \
      --win-exe-sha256 <64-hex>

  # Verify no REPLACE_WITH_* placeholders remain:
  python3 scripts/update-packaging-manifests.py --check
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import subprocess
import sys
import tempfile
from typing import Dict, Optional

REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent
DEFAULT_TAURI_CONF = REPO_ROOT / "src-tauri" / "tauri.conf.json"
DEFAULT_CASK_PATH = REPO_ROOT / "packaging" / "homebrew" / "ripdaddy.rb"
DEFAULT_WINGET_PATH = REPO_ROOT / "packaging" / "winget" / "RipDaddy.installer.yaml"
DEFAULT_GITHUB_REPO = "swindsor8/RipDaddy"

SHA256_RE = re.compile(r"^[0-9a-fA-F]{64}$")


def read_default_version(tauri_conf: pathlib.Path = DEFAULT_TAURI_CONF) -> str:
    data = json.loads(tauri_conf.read_text(encoding="utf-8"))
    version = str(data.get("version", "")).strip()
    if not version:
        raise ValueError(f"Missing 'version' in {tauri_conf}")
    return version


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest().lower()


def validate_sha256(value: str, label: str) -> str:
    cleaned = value.strip()
    if not SHA256_RE.match(cleaned):
        raise ValueError(
            f"Invalid SHA256 for {label}: expected 64 hex chars, got {value!r}"
        )
    return cleaned.lower()


def find_asset_file(directory: pathlib.Path, exact_name: str, fallback_pattern: str) -> Optional[pathlib.Path]:
    direct = directory / exact_name
    if direct.is_file():
        return direct
    for candidate in sorted(directory.rglob("*")):
        if candidate.is_file() and candidate.name == exact_name:
            return candidate
    regex = re.compile(fallback_pattern, re.IGNORECASE)
    for candidate in sorted(directory.rglob("*")):
        if candidate.is_file() and regex.match(candidate.name):
            return candidate
    return None


def compute_dir_checksums(directory: pathlib.Path, version: str) -> Dict[str, str]:
    """Computes SHA256 for every file in `directory` and identifies key installer artifacts."""
    all_hashes: Dict[str, str] = {}
    for entry in sorted(directory.rglob("*")):
        if not entry.is_file():
            continue
        if entry.name in {"SHA256SUMS.txt", "ripdaddy.rb", "RipDaddy.installer.yaml"}:
            continue
        all_hashes[entry.name] = sha256_file(entry)

    arm_dmg = find_asset_file(
        directory,
        f"RipDaddy_{version}_aarch64.dmg",
        r"^RipDaddy_.*_(aarch64|arm64)\.dmg$",
    )
    x64_dmg = find_asset_file(
        directory,
        f"RipDaddy_{version}_x64.dmg",
        r"^RipDaddy_.*_(x64|x86_64)\.dmg$",
    )
    win_exe = find_asset_file(
        directory,
        f"RipDaddy_{version}_x64-setup.exe",
        r"^RipDaddy_.*_x64-setup\.exe$",
    )

    result = {"__all__": json.dumps(all_hashes, sort_keys=True)}
    if arm_dmg:
        result["arm64_dmg"] = all_hashes[arm_dmg.name]
        result["arm64_dmg_name"] = arm_dmg.name
    if x64_dmg:
        result["x64_dmg"] = all_hashes[x64_dmg.name]
        result["x64_dmg_name"] = x64_dmg.name
    if win_exe:
        result["win_exe"] = all_hashes[win_exe.name]
        result["win_exe_name"] = win_exe.name
    return result


def download_release_assets_via_gh(repo: str, tag: str, dest_dir: pathlib.Path) -> None:
    """Downloads release assets (supports both draft and published releases) using `gh api`."""
    proc = subprocess.run(
        ["gh", "api", f"repos/{repo}/releases?per_page=30"],
        check=True,
        capture_output=True,
        text=True,
    )
    releases = json.loads(proc.stdout)
    target = next((r for r in releases if r.get("tag_name") == tag), None)
    if target is None:
        raise RuntimeError(
            f"No GitHub release (draft or published) found for tag {tag!r} in {repo}."
        )

    assets = target.get("assets", [])
    if not assets:
        raise RuntimeError(f"Release {tag!r} in {repo} currently has no uploaded assets.")

    dest_dir.mkdir(parents=True, exist_ok=True)
    for asset in assets:
        name = asset["name"]
        if name in {"SHA256SUMS.txt", "ripdaddy.rb", "RipDaddy.installer.yaml"}:
            continue
        asset_id = asset["id"]
        out_path = dest_dir / name
        print(f"Downloading {name} (asset #{asset_id})...", file=sys.stderr)
        with out_path.open("wb") as out_fh:
            subprocess.run(
                [
                    "gh",
                    "api",
                    "-H",
                    "Accept: application/octet-stream",
                    f"repos/{repo}/releases/assets/{asset_id}",
                ],
                check=True,
                stdout=out_fh,
            )


def render_homebrew_cask(content: str, version: str, arm_sha: str, intel_sha: str) -> str:
    arm_lower = validate_sha256(arm_sha, "macOS ARM64 DMG")
    intel_lower = validate_sha256(intel_sha, "macOS Intel x64 DMG")

    updated, n_ver = re.subn(
        r'^(\s*version\s+")[^"]+(")',
        rf"\g<1>{version}\g<2>",
        content,
        count=1,
        flags=re.MULTILINE,
    )
    updated, n_arm = re.subn(
        r'^(\s*sha256\s+arm:\s+")[^"]+(")',
        rf"\g<1>{arm_lower}\g<2>",
        updated,
        count=1,
        flags=re.MULTILINE,
    )
    updated, n_intel = re.subn(
        r'^(\s*intel:\s+")[^"]+(")',
        rf"\g<1>{intel_lower}\g<2>",
        updated,
        count=1,
        flags=re.MULTILINE,
    )

    if n_ver != 1 or n_arm != 1 or n_intel != 1:
        raise ValueError(
            f"Unexpected Homebrew cask structure (matched version={n_ver}, arm={n_arm}, intel={n_intel})"
        )
    return updated


def render_winget_manifest(content: str, version: str, exe_sha: str, repo: str = DEFAULT_GITHUB_REPO) -> str:
    exe_upper = validate_sha256(exe_sha, "Windows x64 NSIS EXE").upper()
    installer_url = (
        f"https://github.com/{repo}/releases/download/v{version}/RipDaddy_{version}_x64-setup.exe"
    )

    updated, n_ver = re.subn(
        r"^(PackageVersion:\s*)\S+",
        rf"\g<1>{version}",
        content,
        count=1,
        flags=re.MULTILINE,
    )
    updated, n_url = re.subn(
        r"^(\s*InstallerUrl:\s*)\S+",
        rf"\g<1>{installer_url}",
        updated,
        count=1,
        flags=re.MULTILINE,
    )
    updated, n_sha = re.subn(
        r"^(\s*InstallerSha256:\s*)\S+",
        rf"\g<1>{exe_upper}",
        updated,
        count=1,
        flags=re.MULTILINE,
    )

    if n_ver != 1 or n_url != 1 or n_sha != 1:
        raise ValueError(
            f"Unexpected WinGet manifest structure (matched version={n_ver}, url={n_url}, sha={n_sha})"
        )
    return updated


def check_no_placeholders(cask_path: pathlib.Path, winget_path: pathlib.Path) -> int:
    errors = []
    for path in (cask_path, winget_path):
        text = path.read_text(encoding="utf-8")
        matches = re.findall(r"REPLACE_WITH_[A-Z0-9_]+", text)
        if matches:
            errors.append(f"{path}: unresolved placeholder(s): {', '.join(matches)}")
    if errors:
        for err in errors:
            print(f"ERROR: {err}", file=sys.stderr)
        return 1
    print("OK: No SHA256 placeholders remain in packaging manifests.")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Populate SHA256 checksums in RipDaddy's Homebrew & WinGet manifests."
    )
    parser.add_argument(
        "--version",
        help="Release version without leading 'v' (defaults to src-tauri/tauri.conf.json).",
    )
    parser.add_argument(
        "--repo",
        default=DEFAULT_GITHUB_REPO,
        help=f"GitHub owner/repo (default: {DEFAULT_GITHUB_REPO}).",
    )
    parser.add_argument(
        "--from-dir",
        type=pathlib.Path,
        help="Directory containing built release installers (.dmg / -setup.exe).",
    )
    parser.add_argument(
        "--from-release",
        action="store_true",
        help="Download release assets from GitHub Release v<version> via `gh` and compute checksums.",
    )
    parser.add_argument("--arm64-dmg-sha256", help="Explicit SHA256 for RipDaddy_<ver>_aarch64.dmg.")
    parser.add_argument("--x64-dmg-sha256", help="Explicit SHA256 for RipDaddy_<ver>_x64.dmg.")
    parser.add_argument("--win-exe-sha256", help="Explicit SHA256 for RipDaddy_<ver>_x64-setup.exe.")
    parser.add_argument(
        "--cask-path",
        type=pathlib.Path,
        default=DEFAULT_CASK_PATH,
        help=f"Path to Homebrew cask file (default: {DEFAULT_CASK_PATH}).",
    )
    parser.add_argument(
        "--winget-path",
        type=pathlib.Path,
        default=DEFAULT_WINGET_PATH,
        help=f"Path to WinGet installer manifest (default: {DEFAULT_WINGET_PATH}).",
    )
    parser.add_argument(
        "--write-sums",
        type=pathlib.Path,
        help="Optional path to write SHA256SUMS.txt covering all assets in --from-dir / --from-release.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print computed checksums without modifying manifest files.",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="Verify that neither manifest contains REPLACE_WITH_* placeholders.",
    )
    args = parser.parse_args()

    if args.check:
        return check_no_placeholders(args.cask_path, args.winget_path)

    version = (args.version or read_default_version()).lstrip("v").strip()
    tag = f"v{version}"

    arm_sha = args.arm64_dmg_sha256
    x64_sha = args.x64_dmg_sha256
    exe_sha = args.win_exe_sha256
    all_hashes: Dict[str, str] = {}

    tmp_ctx = None
    try:
        assets_dir = args.from_dir
        if args.from_release:
            tmp_ctx = tempfile.TemporaryDirectory(prefix="ripdaddy-release-")
            assets_dir = pathlib.Path(tmp_ctx.name)
            download_release_assets_via_gh(args.repo, tag, assets_dir)

        if assets_dir is not None:
            detected = compute_dir_checksums(assets_dir, version)
            all_hashes = json.loads(detected.get("__all__", "{}"))
            arm_sha = arm_sha or detected.get("arm64_dmg")
            x64_sha = x64_sha or detected.get("x64_dmg")
            exe_sha = exe_sha or detected.get("win_exe")

        if args.write_sums and all_hashes:
            lines = [f"{digest}  {name}" for name, digest in sorted(all_hashes.items())]
            args.write_sums.parent.mkdir(parents=True, exist_ok=True)
            args.write_sums.write_text("\n".join(lines) + "\n", encoding="utf-8")
            print(f"Wrote {len(lines)} checksums to {args.write_sums}")

        missing = []
        if not arm_sha:
            missing.append(f"RipDaddy_{version}_aarch64.dmg (--arm64-dmg-sha256)")
        if not x64_sha:
            missing.append(f"RipDaddy_{version}_x64.dmg (--x64-dmg-sha256)")
        if not exe_sha:
            missing.append(f"RipDaddy_{version}_x64-setup.exe (--win-exe-sha256)")
        if missing:
            raise ValueError(
                "Missing required installer SHA256(s): " + ", ".join(missing)
            )

        cask_before = args.cask_path.read_text(encoding="utf-8")
        winget_before = args.winget_path.read_text(encoding="utf-8")

        cask_after = render_homebrew_cask(cask_before, version, arm_sha, x64_sha)
        winget_after = render_winget_manifest(winget_before, version, exe_sha, args.repo)

        print(f"Version:                {version}")
        print(f"macOS ARM64 DMG SHA256: {validate_sha256(arm_sha, 'arm64')}")
        print(f"macOS x64 DMG SHA256:   {validate_sha256(x64_sha, 'x64')}")
        print(f"Windows EXE SHA256:     {validate_sha256(exe_sha, 'win').upper()}")

        if args.dry_run:
            print("(dry-run: no files modified)")
            return 0

        args.cask_path.write_text(cask_after, encoding="utf-8")
        args.winget_path.write_text(winget_after, encoding="utf-8")
        print(f"Updated {args.cask_path}")
        print(f"Updated {args.winget_path}")

        return check_no_placeholders(args.cask_path, args.winget_path)
    finally:
        if tmp_ctx is not None:
            tmp_ctx.cleanup()


if __name__ == "__main__":
    raise SystemExit(main())
