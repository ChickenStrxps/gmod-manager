$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location -LiteralPath $root
cargo build --release
if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
$destination = Join-Path $root 'dist\GModManager-Windows.zip'
New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
Compress-Archive -LiteralPath @(
    (Join-Path $root 'target\release\gmod-manager.exe'),
    (Join-Path $root 'presets'),
    (Join-Path $root 'README.md')
) -DestinationPath $destination -Force
Write-Output $destination
