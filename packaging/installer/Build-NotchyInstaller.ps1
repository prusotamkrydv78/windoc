# Builds the Notchy installer: dist\Notchy-<version>-Setup.exe.
#
#   powershell -ExecutionPolicy Bypass -File packaging\installer\Build-NotchyInstaller.ps1
#
# Needs Inno Setup 6 (free: https://jrsoftware.org/isdl.php). No code-signing certificate:
# Windows SmartScreen may warn the first time, as with the portable ZIP.

param(
    # Skip `cargo build --release` when target\release\Notchy.exe is already current.
    [switch]$NoBuild
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location $root

function Get-InnoCompiler {
    $command = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($null -ne $command) { return $command.Source }
    foreach ($path in @(
            (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
            (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe'),
            (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'))) {
        if (Test-Path $path) { return $path }
    }
    throw 'Inno Setup 6 was not found. Install it from https://jrsoftware.org/isdl.php (or: winget install JRSoftware.InnoSetup), then run this again.'
}

$version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "(.+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
$compiler = Get-InnoCompiler

if (-not $NoBuild) {
    $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo build --release failed.' }
}
$exe = Join-Path $root 'target\release\Notchy.exe'
if (-not (Test-Path $exe)) { throw "Release build not found: $exe" }

# Stage what the installer ships.
$dist = Join-Path $root 'dist'
$stage = Join-Path $dist 'installer-stage'
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $stage
New-Item -ItemType Directory -Force -Path (Join-Path $stage 'licenses') | Out-Null
Copy-Item $exe (Join-Path $stage 'Notchy.exe')
Copy-Item (Join-Path $root 'resources\icon-dark.ico') (Join-Path $stage 'icon.ico')
Copy-Item (Join-Path $root 'packaging\portable\READ-ME-FIRST.txt') (Join-Path $stage 'READ-ME-FIRST.txt')
foreach ($file in 'LICENSE', 'THIRD_PARTY_NOTICES.md', 'UPSTREAM_NOTICE.md') {
    Copy-Item (Join-Path $root $file) (Join-Path (Join-Path $stage 'licenses') $file)
}
Copy-Item (Join-Path $root 'resources\licenses\*') (Join-Path $stage 'licenses') -Recurse

& $compiler `
    "/DSourceDirectory=$stage" `
    "/DOutputDirectory=$dist" `
    "/DVersion=$version" `
    (Join-Path $PSScriptRoot 'Notchy.iss')
if ($LASTEXITCODE -ne 0) { throw 'Inno Setup failed to build the installer.' }

$setup = Join-Path $dist "Notchy-$version-Setup.exe"
$size = [math]::Round((Get-Item $setup).Length / 1MB, 1)
Write-Host "Built $setup ($size MB)"
Write-Host "SHA-256: $((Get-FileHash $setup -Algorithm SHA256).Hash)"
