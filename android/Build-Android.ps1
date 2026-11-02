# Builds the Notchy phone app and, with -Install, puts it on a phone plugged in over USB.
#
#   powershell -ExecutionPolicy Bypass -File android\Build-Android.ps1 [-Install]
#
# Run android\Setup-AndroidSdk.ps1 once first. For -Install, turn on Developer options and
# USB debugging on the phone and accept the "Allow USB debugging?" prompt.

param([switch]$Install)

$ErrorActionPreference = 'Stop'
$gradle = Join-Path $env:LOCALAPPDATA 'Gradle\gradle-8.14.3\bin\gradle.bat'
if (-not (Test-Path $gradle)) { throw 'Gradle not found. Run android\Setup-AndroidSdk.ps1 first.' }

Push-Location $PSScriptRoot
try {
    & $gradle --no-daemon assembleDebug
    if ($LASTEXITCODE -ne 0) { throw 'Build failed.' }
    $apk = Join-Path $PSScriptRoot 'app\build\outputs\apk\debug\app-debug.apk'
    $dist = Join-Path $PSScriptRoot '..\dist'
    New-Item -ItemType Directory -Force -Path $dist | Out-Null
    Copy-Item $apk (Join-Path $dist 'Notchy-phone.apk')
    Write-Host "Built dist\Notchy-phone.apk ($([math]::Round((Get-Item $apk).Length / 1KB)) KB)"

    if ($Install) {
        $adb = Join-Path $env:LOCALAPPDATA 'Android\Sdk\platform-tools\adb.exe'
        & $adb install -r $apk
        if ($LASTEXITCODE -ne 0) { throw 'adb install failed. Is the phone plugged in with USB debugging on?' }
        & $adb shell am start -n com.notchy.app/.MainActivity | Out-Null
        Write-Host 'Installed and opened on the phone.'
    }
} finally {
    Pop-Location
}
