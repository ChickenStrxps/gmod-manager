# Renders the product intro for the version in Cargo.toml.
#
#   .\promo\render.ps1                 full run: build, voice, capture, render
#   .\promo\render.ps1 -SkipCapture    reuse the last recording (script/style tweaks)
#
# Needs Python (edge-tts, numpy, pywin32), ffmpeg and a Remotion install. Remotion
# is borrowed from ..\AnimeEditSaaS\node_modules through a junction; point
# GMM_REMOTION_MODULES at another node_modules to use a different one.
# The capture runs in the background, off screen: you can keep using the PC.
param([switch]$SkipCapture)

$ErrorActionPreference = 'Stop'
$promo = $PSScriptRoot
$root = Split-Path $promo

function Run([string]$what, [scriptblock]$command) {
    Write-Host "== $what" -ForegroundColor Cyan
    & $command
    if ($LASTEXITCODE -ne 0) { throw "$what failed ($LASTEXITCODE)" }
}

Push-Location $root
try {
    $version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
    Run 'Narration' { python promo/tools/voice.py }
    if (-not $SkipCapture) {
        Run 'Promo build' { cargo build --release --features promo --target-dir target/promo }
        Run 'Capture' { python promo/tools/capture.py }
    }
    $seconds = python promo/tools/manifest.py
    if ($LASTEXITCODE -ne 0) { throw 'Manifest failed' }
    Run 'Music' { python promo/tools/music.py promo/public/music.wav ([double]$seconds + 1) }

    $modules = Join-Path $promo 'node_modules'
    if (-not (Test-Path $modules)) {
        $source = if ($env:GMM_REMOTION_MODULES) { $env:GMM_REMOTION_MODULES } else { Join-Path $root '..\AnimeEditSaaS\node_modules' }
        if (-not (Test-Path (Join-Path $source 'remotion'))) { throw "No Remotion install at $source" }
        New-Item -ItemType Junction -Path $modules -Target (Resolve-Path $source) | Out-Null
    }

    $out = Join-Path $promo "out\gmod-manager-intro-v$version.mp4"
    Push-Location $promo
    try {
        Run 'Render' { npx remotion render src/index.ts ProductIntro $out }
    } finally { Pop-Location }

    # README preview: the live-search scene as a small looping GIF, plus the video itself.
    $assets = Join-Path $root 'assets'
    $discover = (Get-Content (Join-Path $promo 'src\manifest.generated.json') -Raw | ConvertFrom-Json).scenes |
        Where-Object id -eq 'discover'
    $start = $discover.from / 50 + 0.4
    $length = [Math]::Min(8, $discover.duration / 50 - 0.9)
    Run 'Preview GIF' {
        ffmpeg -v error -y -ss $start -t $length -i $out -vf "fps=12,scale=720:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=96:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle" (Join-Path $assets 'intro-preview.gif')
    }
    Copy-Item $out (Join-Path $assets 'gmod-manager-intro.mp4')
    Write-Host "Done: $out" -ForegroundColor Green
} finally { Pop-Location }
