# RipDaddy

RipDaddy is a fast, lightweight, cross-platform desktop orchestration wrapper built with **Tauri v2 (Rust)**, **Svelte 5 (TypeScript, Runes)**, and **Tailwind CSS v4**.

It supports three flexible workflows around **external, user-installed** CLI binaries:
- **Rip Only:** `makemkvcon -r` directly to lossless MKV output
- **Encode Only:** `HandBrakeCLI --json` on existing local video files with native 16:9 preview
- **Rip + Encode:** Decoupled two-stage pipeline (`makemkvcon` -> temp MKV -> `HandBrakeCLI` -> temp file cleanup)

### Highlights
- **Customizable Quality Matrix (480p / 720p / 1080p / 4K × Local / Remote):** Pre-tuned profiles for home LAN direct play (10-bit x265, audio passthrough, MKV) vs. remote streaming (compact RF, AAC, web-optimized MP4), editable per slot with live `HandBrakeCLI` flag preview.
- **Live IMDb / TVDB / TMDB / TVMaze Titling:** Built-in metadata search (OMDb, TVDB v4, TMDB v3/v4 + external IDs, and keyless TVMaze), optical disc label cleanup, poster preview, and automatic Plex / Jellyfin / Plain output naming.
- **OS-Aware Auto-Detection & Setup:** Discovers `makemkvcon` and `HandBrakeCLI` across `PATH`, standard Windows/macOS/Linux install paths, Linux Flatpaks (`fr.handbrake.ghb`, `com.makemkv.MakeMKV`), and Linux `toolbox` host delegation (`flatpak-spawn --host`).
- **Process Safety:** Cleanly terminates all spawned `makemkvcon` and `HandBrakeCLI` child processes on job cancellation or application exit.

## Legal & Binary Policy

RipDaddy **does not** bundle, download, distribute, or include any decryption/circumvention code or third-party binaries. Users independently install MakeMKV (`makemkvcon`) and HandBrake (`HandBrakeCLI`), or use RipDaddy's optional OS package-manager helper (`winget` on Windows, `brew` on macOS/Linux, `flatpak` on Linux).

## OS CLI Bootstrap Scripts

- **macOS / Linux:** `./scripts/setup-deps.sh`
- **Windows (PowerShell):** `.\scripts\setup-deps.ps1`

## Development

```bash
# Install frontend dependencies
npm install

# Run frontend TypeScript & Svelte checks
npm run check

# Build frontend bundle & verify Rust backend + unit tests
npm run build
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml

# Launch the Tauri desktop app in dev mode
npm run tauri dev
```

## Architecture

- `src-tauri/src/detect.rs` — Cross-platform binary locator (`PATH`, standard dirs, Linux Flatpak, `toolbox` host fallback), default folder discovery, OS package-manager auto-setup, and version probes.
- `src-tauri/src/makemkv.rs` — Data structures and parser stubs for `makemkvcon -r` (`DRV`, `TINFO`, `SINFO`, `PRGV`).
- `src-tauri/src/handbrake.rs` — 8-slot Resolution × Delivery quality matrix, `HandBrakeCLI` flag builder, and `--json` progress / preset parser stubs.
- `src-tauri/src/metadata.rs` — Live `reqwest` HTTP search across OMDb (IMDb), TVDB v4, TMDB, and keyless TVMaze, plus disc-label cleanup and Plex/Jellyfin/Plain path formatting.
- `src-tauri/src/pipeline.rs` — Mode-aware two-stage queue state machine (`RipOnly` / `EncodeOnly` / `RipAndEncode`) and `ProcessRegistry` for orphan-free child termination.
- `src/App.svelte`, `src/lib/SettingsDropdown.svelte`, `src/lib/PreviewDrawer.svelte`, `src/lib/QueueDock.svelte` — Full-width Clay-inspired desktop UI with categorized Settings dropdown, live Preview drawer, and collapsible Bottom Queue dock.

## License

MIT — see [LICENSE](./LICENSE).
