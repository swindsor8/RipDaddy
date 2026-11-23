# OS-aware CLI dependency bootstrapper for RipDaddy (Windows).
# Invokes Windows winget to install user-owned HandBrakeCLI and MakeMKV without bundling binaries.
$ErrorActionPreference = "Stop"

Write-Host "[RipDaddy] Checking Windows external CLI dependencies..."

if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
    Write-Error "winget (App Installer) was not found. Install it from Microsoft Store or download HandBrakeCLI and MakeMKV manually."
    exit 1
}

if (-not (Get-Command HandBrakeCLI -ErrorAction SilentlyContinue)) {
    Write-Host "[RipDaddy] Installing HandBrakeCLI via winget..."
    winget install --id HandBrake.HandBrake.CLI --exact --accept-source-agreements --accept-package-agreements
} else {
    Write-Host "[RipDaddy] HandBrakeCLI already installed."
}

$makemkvCandidates = @(
    "$env:ProgramFiles (x86)\MakeMKV\makemkvcon64.exe",
    "$env:ProgramFiles\MakeMKV\makemkvcon64.exe"
)
$hasMakemkv = (Get-Command makemkvcon64.exe -ErrorAction SilentlyContinue) -or
              (Get-Command makemkvcon.exe -ErrorAction SilentlyContinue) -or
              ($makemkvCandidates | Where-Object { Test-Path $_ })

if (-not $hasMakemkv) {
    Write-Host "[RipDaddy] Installing MakeMKV via winget..."
    winget install --id GuinpinSoft.MakeMKV --exact --accept-source-agreements --accept-package-agreements
} else {
    Write-Host "[RipDaddy] MakeMKV already installed."
}

Write-Host "[RipDaddy] CLI dependency setup complete."
