# Homebrew Cask template for a custom tap (e.g., homebrew-ripdaddy).
# Installing via `brew install --cask ripdaddy` automatically installs upstream
# HandBrakeCLI and MakeMKV alongside RipDaddy without bundling either binary.
# Checksums are auto-populated by `.github/workflows/release.yml` when tagging `v*`
# (or locally via `python3 scripts/update-packaging-manifests.py --from-release`).
cask "ripdaddy" do
  arch arm: "aarch64", intel: "x64"

  version "0.1.0"
  sha256 arm:   "83c96cf965189ef57749b3154526f9837a3b14423a910686203dc5c2b5ad5250",
         intel: "6c6790aec1cd28de74d693dff6d886d27cf056b24c4076e08145ce62458a818b"

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
