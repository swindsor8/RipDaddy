# Homebrew Cask template for a custom tap (e.g., homebrew-ripdaddy).
# Installing via `brew install --cask ripdaddy` automatically installs upstream
# HandBrakeCLI and MakeMKV alongside RipDaddy without bundling either binary.
cask "ripdaddy" do
  arch arm: "aarch64", intel: "x64"

  version "0.1.0"
  sha256 arm:   "REPLACE_WITH_ARM64_DMG_SHA256",
         intel: "REPLACE_WITH_X64_DMG_SHA256"

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
