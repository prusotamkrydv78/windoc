# One-time, small Android setup: no Android Studio, no emulator (about 2-3 GB).
#
#   powershell -ExecutionPolicy Bypass -File android\Setup-AndroidSdk.ps1
#
# Needs Java 17 (winget install Microsoft.OpenJDK.17). Installs the SDK command-line tools,
# platform-tools (adb), Android 16 (API 36) and build-tools into %LOCALAPPDATA%\Android\Sdk,
# plus Gradle into %LOCALAPPDATA%\Gradle, then writes android\local.properties.

$ErrorActionPreference = 'Stop'
# Windows PowerShell 5.1 downloads many times slower while drawing its progress bar.
$ProgressPreference = 'SilentlyContinue'
$sdk = Join-Path $env:LOCALAPPDATA 'Android\Sdk'
$tools = Join-Path $sdk 'cmdline-tools\latest'
$gradleVersion = '8.14.3'
$gradleHome = Join-Path $env:LOCALAPPDATA "Gradle\gradle-$gradleVersion"
$temp = Join-Path $env:TEMP 'notchy-android-setup'
New-Item -ItemType Directory -Force -Path $temp | Out-Null

if (-not (Get-Command java -ErrorAction SilentlyContinue)) {
    throw 'Java was not found. Install it with: winget install Microsoft.OpenJDK.17'
}

if (-not (Test-Path (Join-Path $tools 'bin\sdkmanager.bat'))) {
    Write-Host 'Downloading Android command-line tools...'
    $zip = Join-Path $temp 'cmdline-tools.zip'
    Invoke-WebRequest 'https://dl.google.com/android/repository/commandlinetools-win-13114758_latest.zip' -OutFile $zip
    $unpack = Join-Path $temp 'cmdline-tools'
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $unpack
    Expand-Archive $zip -DestinationPath $unpack
    New-Item -ItemType Directory -Force -Path (Split-Path $tools) | Out-Null
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $tools
    Move-Item (Join-Path $unpack 'cmdline-tools') $tools
}

$sdkmanager = Join-Path $tools 'bin\sdkmanager.bat'
Write-Host 'Accepting licences and installing Android 16 packages...'
# sdkmanager asks once per licence; PowerShell pipes don't reach its prompt, so feed a file
# of "y" lines through cmd.
$answers = Join-Path $temp 'yes.txt'
Set-Content -Encoding ASCII $answers (@('y') * 40)
cmd /c "`"$sdkmanager`" --sdk_root=`"$sdk`" --licenses < `"$answers`"" | Out-Null
cmd /c "`"$sdkmanager`" --sdk_root=`"$sdk`" platform-tools platforms;android-36 build-tools;36.0.0 < `"$answers`""
# sdkmanager exits 0 even when it skips packages, so check what landed.
foreach ($path in 'platform-tools\adb.exe', 'platforms\android-36\android.jar', 'build-tools\36.0.0') {
    if (-not (Test-Path (Join-Path $sdk $path))) { throw "Android package missing after install: $path" }
}

if (-not (Test-Path (Join-Path $gradleHome 'bin\gradle.bat'))) {
    Write-Host "Downloading Gradle $gradleVersion..."
    $zip = Join-Path $temp 'gradle.zip'
    Invoke-WebRequest "https://services.gradle.org/distributions/gradle-$gradleVersion-bin.zip" -OutFile $zip
    Expand-Archive $zip -DestinationPath (Join-Path $env:LOCALAPPDATA 'Gradle') -Force
}

$local = Join-Path $PSScriptRoot 'local.properties'
"sdk.dir=$($sdk -replace '\\', '\\')" | Set-Content -Encoding ASCII $local
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $temp

Write-Host ''
Write-Host "Done. SDK: $sdk"
Write-Host 'Next: powershell -ExecutionPolicy Bypass -File android\Build-Android.ps1'
