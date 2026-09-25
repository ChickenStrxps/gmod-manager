# Publishes the version in Cargo.toml as a GitHub release. The app's updater downloads it from there.
# Usage: bump `version` in Cargo.toml, commit, then run .\release.ps1
param([string]$Notes = '')
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location -LiteralPath $root

$repo = (Select-String -Path 'src\update.rs' -Pattern 'pub const REPO: &str = "(.+)";').Matches[0].Groups[1].Value
$version = (Select-String -Path 'Cargo.toml' -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$tag = "v$version"

if (git status --porcelain) { throw 'Commit your changes before releasing.' }
if (git tag --list $tag) { throw "$tag already exists. Bump the version in Cargo.toml first." }

cargo test --locked
if ($LASTEXITCODE -ne 0) { throw 'Tests failed.' }
& .\build.ps1 | Out-Null

$exe = Join-Path $root 'target\release\gmod-manager.exe'
$zip = Join-Path $root 'dist\GModManager-Windows.zip'
$sha = Join-Path $root 'target\release\gmod-manager.exe.sha256'
$hash = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLower()
[IO.File]::WriteAllText($sha, "$hash  gmod-manager.exe`n")

# Reuse the GitHub login Git already has for the repo owner.
$owner = $repo.Split('/')[0]
$credential = "protocol=https`nhost=github.com`nusername=$owner`n`n" | git credential fill
$token = ($credential | Where-Object { $_ -like 'password=*' }) -replace '^password=', ''
if (-not $token) { throw "No GitHub login for $owner. Run: git credential-manager github login --username $owner" }
$headers = @{ Authorization = "Bearer $token"; Accept = 'application/vnd.github+json'; 'User-Agent' = 'gmm-release' }

git tag $tag
git push origin HEAD
git push origin $tag

$body = @{ tag_name = $tag; name = "GMod Manager $version"; body = $Notes; generate_release_notes = [string]::IsNullOrEmpty($Notes) } | ConvertTo-Json
$release = Invoke-RestMethod -Method Post -Uri "https://api.github.com/repos/$repo/releases" -Headers $headers -Body $body -ContentType 'application/json'
foreach ($file in @($exe, $sha, $zip)) {
    $name = Split-Path -Leaf $file
    $uri = "https://uploads.github.com/repos/$repo/releases/$($release.id)/assets?name=$name"
    Invoke-RestMethod -Method Post -Uri $uri -Headers $headers -InFile $file -ContentType 'application/octet-stream' | Out-Null
    Write-Output "Uploaded $name"
}
Write-Output $release.html_url
