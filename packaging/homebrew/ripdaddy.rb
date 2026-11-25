# Homebrew Cask template for a custom tap (e.g., homebrew-ripdaddy).
# Installing via `brew install --cask ripdaddy` automatically installs upstream
# HandBrakeCLI and MakeMKV alongside RipDaddy without bundling either binary.
# Checksums are auto-populated by `.github/workflows/release.yml` when tagging `v*`
# (or locally via `python3 scripts/update-packaging-manifests.py --from-release`).
cask "ripdaddy" do
  arch arm: "aarch64", intel: "x64"

  version "0.1.1"
  sha256 arm:   "6cb332b540b1a75cb4f63cf699a8c1b33935488276f8550a2a65c4081368f68c",
         intel: "4097d95dc44adc65be0c769ec0aed8974ab5addb5f183dd42742dac154850231"

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
