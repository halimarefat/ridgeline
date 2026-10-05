# Ridgeline task runner (Windows PowerShell). macOS / Linux: scripts/rl.sh
#
#   .\scripts\rl.ps1 setup     install UI dependencies
#   .\scripts\rl.ps1 test      Rust workspace tests + UI typecheck and unit tests
#   .\scripts\rl.ps1 demo      build the UI and serve it at http://localhost:1420 with simulated devices
#   .\scripts\rl.ps1 e2e       end-to-end browser walkthrough against the developer server
#   .\scripts\rl.ps1 app       run the desktop app in development mode (Tauri)
#   .\scripts\rl.ps1 package   build unsigned Windows installers (NSIS + MSI)
#   .\scripts\rl.ps1 fit       write a synthetic FIT file to target\fit
param([string]$Command = "help")
$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
$Ui = Join-Path $Root "apps\desktop\ui"

function Invoke-Checked([scriptblock]$Block) {
  & $Block
  if ($LASTEXITCODE -ne 0) { throw "Command failed with exit code $LASTEXITCODE" }
}
function Install-Ui {
  Push-Location $Ui
  try {
    if (Test-Path "package-lock.json") { Invoke-Checked { npm ci --no-audit --no-fund } } else { Invoke-Checked { npm install --no-audit --no-fund } }
  } finally { Pop-Location }
}
function Build-Ui {
  if (-not (Test-Path (Join-Path $Ui "node_modules"))) { Install-Ui }
  Push-Location $Ui
  try { Invoke-Checked { npm run build } } finally { Pop-Location }
}

switch ($Command) {
  "setup" { Install-Ui }
  "test" {
    Push-Location $Root; try { Invoke-Checked { cargo test --workspace } } finally { Pop-Location }
    if (-not (Test-Path (Join-Path $Ui "node_modules"))) { Install-Ui }
    Push-Location $Ui; try { Invoke-Checked { npm run typecheck }; Invoke-Checked { npm test } } finally { Pop-Location }
  }
  "demo" {
    Build-Ui
    Write-Host "Open http://localhost:1420 (data in $Root\.ridgeline-data)"
    Push-Location $Root
    try { Invoke-Checked { cargo run --release -p rl-devserver -- --port 1420 --data .ridgeline-data --ui apps/desktop/ui/dist } } finally { Pop-Location }
  }
  "e2e" {
    Build-Ui
    Push-Location $Root; try { Invoke-Checked { cargo build --release -p rl-devserver } } finally { Pop-Location }
    $Data = Join-Path ([System.IO.Path]::GetTempPath()) ("ridgeline-e2e-" + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $Data | Out-Null
    $Srv = Start-Process -FilePath (Join-Path $Root "target\release\rl-devserver.exe") -ArgumentList "--port", "1421", "--data", $Data, "--ui", (Join-Path $Ui "dist") -PassThru -WindowStyle Hidden
    try {
      Start-Sleep -Seconds 1
      Push-Location $Ui
      if (-not (Test-Path "node_modules\playwright")) { Invoke-Checked { npm install --no-save --no-audit --no-fund playwright@1.56.0 } }
      Invoke-Checked { node e2e/demo-flow.mjs http://localhost:1421 }
    } finally { Pop-Location; Stop-Process -Id $Srv.Id -ErrorAction SilentlyContinue }
  }
  "app" {
    Build-Ui
    Push-Location (Join-Path $Root "apps\desktop")
    try { Invoke-Checked { .\ui\node_modules\.bin\tauri.cmd dev } } finally { Pop-Location }
  }
  "package" {
    Build-Ui
    Push-Location (Join-Path $Root "apps\desktop")
    try { Invoke-Checked { .\ui\node_modules\.bin\tauri.cmd build --bundles nsis,msi } } finally { Pop-Location }
  }
  "fit" {
    Push-Location $Root
    try { Invoke-Checked { cargo run -q -p rl-storage --example fit_fixture -- target/fit } } finally { Pop-Location }
  }
  default { Get-Content $PSCommandPath | Select-Object -Skip 1 -First 9 | ForEach-Object { $_ -replace '^# ?', '' } }
}
