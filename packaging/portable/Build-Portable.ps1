# Builds a portable Notchy: one ZIP your friends unzip and run, no installer, no admin.
#
#   powershell -ExecutionPolicy Bypass -File packaging\portable\Build-Portable.ps1
#
# Produces dist\Notchy-<version>-portable.zip with Notchy.exe, the licences (GPL-3.0
# requires them to travel with the program) and a short read-me.

param(
    # Skip `cargo build --release` when target\release\Notchy.exe is already current.
    [switch]$NoBuild
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location $root

$version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "(.+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value

if (-not $NoBuild) {
    $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo build --release failed.' }
}

$exe = Join-Path $root 'target\release\Notchy.exe'
if (-not (Test-Path $exe)) { throw "Release build not found: $exe" }

$name = "Notchy-$version-portable"
$dist = Join-Path $root 'dist'
$stage = Join-Path $dist $name
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $stage
New-Item -ItemType Directory -Force -Path (Join-Path $stage 'licenses') | Out-Null

Copy-Item $exe (Join-Path $stage 'Notchy.exe')
foreach ($file in 'LICENSE', 'THIRD_PARTY_NOTICES.md', 'UPSTREAM_NOTICE.md') {
    Copy-Item (Join-Path $root $file) (Join-Path $stage 'licenses' $file)
}
Copy-Item (Join-Path $root 'resources\licenses\*') (Join-Path $stage 'licenses') -Recurse
Copy-Item (Join-Path $PSScriptRoot 'READ-ME-FIRST.txt') (Join-Path $stage 'READ-ME-FIRST.txt')

$zip = Join-Path $dist "$name.zip"
Remove-Item -Force -ErrorAction SilentlyContinue $zip
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip

$size = [math]::Round((Get-Item $zip).Length / 1MB, 1)
$hash = (Get-FileHash $zip -Algorithm SHA256).Hash
Write-Host "Built $zip ($size MB)"
Write-Host "SHA-256: $hash"
