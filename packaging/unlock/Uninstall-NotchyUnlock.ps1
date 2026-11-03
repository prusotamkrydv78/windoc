# Removes the "Unlock with phone" tile and its saved password. Run as administrator.
$ErrorActionPreference = 'SilentlyContinue'
$clsid = '{6E3F8C2A-4B1D-4F7A-9C55-2D8E1A7B9F31}'
Remove-Item "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\Credential Providers\$clsid" -Recurse -Force
Remove-Item "Registry::HKEY_CLASSES_ROOT\CLSID\$clsid" -Recurse -Force
Remove-Item 'HKLM:\SOFTWARE\Notchy\Unlock' -Recurse -Force
Remove-NetFirewallRule -DisplayName 'Notchy phone unlock'
# LogonUI may still hold the DLL until the next restart; delete it then if this fails.
Remove-Item (Join-Path $env:ProgramFiles 'Notchy\notchy_unlock.dll') -Force
Write-Host 'Notchy phone unlock removed. Restart to unload it completely.'
