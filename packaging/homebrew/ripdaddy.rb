# Homebrew Cask template for a custom tap (e.g., homebrew-ripdaddy).
# Installing via `brew install --cask ripdaddy` automatically installs upstream
# HandBrakeCLI and MakeMKV alongside RipDaddy without bundling either binary.
# Checksums are auto-populated by `.github/workflows/release.yml` when tagging `v*`
# (or locally via `python3 scripts/update-packaging-manifests.py --from-release`).
cask "ripdaddy" do
  arch arm: "aarch64", intel: "x64"

  version "0.1.3"
  sha256 arm:   "cd6c5a9ffaf1574be0f24dc26ccbaa5733886fcd2f04e9e3ac7e287c76a992be",
         intel: "23750fce038cdc1ecbcbcd1146a1f996cf4ecced746a024b783072f19ff3951a"

  url "https://github.com/swindsor8/RipDaddy/releases/download/v#{version}/RipDaddy_#{version}_#{arch}.dmg"
  name "RipDaddy"
  desc "Cross-platform optical disc ripping and encoding orchestrator"
  homepage "https://github.com/swindsor8/RipDaddy"

  depends_on formula: "handbrake"
  depends_on cask: "makemkv"

  app "RipDaddy.app"

  zap trash: [
    "~/Library/Application Support/com.ripdaddy.app",
    "~/Library/Caches/com.ripdaddy.app",
  ]
end
