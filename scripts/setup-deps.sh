#!/usr/bin/env bash
# OS-aware CLI dependency bootstrapper for RipDaddy (macOS & Linux).
# Invokes the user's native system package manager without bundling binaries.
set -euo pipefail

OS="$(uname -s)"
echo "[RipDaddy] Detecting OS and external CLI dependencies ($OS)..."

has_cmd() {
  command -v "$1" >/dev/null 2>&1
}

if [[ "$OS" == "Darwin" ]]; then
  if ! has_cmd brew; then
    echo "Homebrew is required for automated macOS setup: https://brew.sh"
    exit 1
  fi
  if ! has_cmd HandBrakeCLI; then
    echo "[RipDaddy] Installing HandBrakeCLI via Homebrew..."
    brew install handbrake
  else
    echo "[RipDaddy] HandBrakeCLI already installed: $(command -v HandBrakeCLI)"
  fi
  if [[ ! -x "/Applications/MakeMKV.app/Contents/MacOS/makemkvcon" ]] && ! has_cmd makemkvcon; then
    echo "[RipDaddy] Installing MakeMKV via Homebrew Cask..."
    brew install --cask makemkv
  else
    echo "[RipDaddy] makemkvcon already installed."
  fi
elif [[ "$OS" == "Linux" ]]; then
  if ! has_cmd HandBrakeCLI && ! flatpak info fr.handbrake.ghb >/dev/null 2>&1; then
    if has_cmd flatpak; then
      echo "[RipDaddy] Installing HandBrake (includes HandBrakeCLI) via Flathub..."
      flatpak install -y flathub fr.handbrake.ghb
    elif has_cmd brew; then
      echo "[RipDaddy] Installing HandBrakeCLI via Homebrew..."
      brew install handbrake
    elif has_cmd apt-get; then
      echo "[RipDaddy] Installing handbrake-cli via apt..."
      sudo apt-get update && sudo apt-get install -y handbrake-cli
    elif has_cmd dnf; then
      echo "[RipDaddy] Installing HandBrake-cli via dnf..."
      sudo dnf install -y HandBrake-cli
    elif has_cmd pacman; then
      echo "[RipDaddy] Installing handbrake-cli via pacman..."
      sudo pacman -S --needed --noconfirm handbrake-cli
    else
      echo "[RipDaddy] Could not detect a supported package manager for HandBrakeCLI."
      echo "Download from: https://handbrake.fr/downloads2.php"
    fi
  else
    echo "[RipDaddy] HandBrakeCLI already installed."
  fi

  if ! has_cmd makemkvcon && [[ ! -x "$HOME/.local/bin/makemkvcon" ]] && ! flatpak info com.makemkv.MakeMKV >/dev/null 2>&1; then
    if has_cmd flatpak; then
      echo "[RipDaddy] Installing MakeMKV (includes makemkvcon) via Flathub..."
      flatpak install -y flathub com.makemkv.MakeMKV
    else
      echo "[RipDaddy] Install MakeMKV for Linux from https://www.makemkv.com/download/ (RipDaddy will auto-detect it)."
    fi
  else
    echo "[RipDaddy] makemkvcon already installed."
  fi
else
  echo "[RipDaddy] Unsupported POSIX OS: $OS"
  exit 1
fi

echo "[RipDaddy] CLI dependency check complete."
