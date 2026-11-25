# Homebrew Cask template for a custom tap (e.g., homebrew-ripdaddy).
# Installing via `brew install --cask ripdaddy` automatically installs upstream
# HandBrakeCLI and MakeMKV alongside RipDaddy without bundling either binary.
# Checksums are auto-populated by `.github/workflows/release.yml` when tagging `v*`
# (or locally via `python3 scripts/update-packaging-manifests.py --from-release`).
cask "ripdaddy" do
  arch arm: "aarch64", intel: "x64"

  version "0.1.2"
  sha256 arm:   "11da778587259d2b1d48bb64bcbb872fec9c4cb1f74973f77c0e05e9fcb119af",
         intel: "c4334fbbe0cfe3cb7b6b2d0a0b97caf9d0e040b83ddce63659dfc992df6ed7e3"

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
