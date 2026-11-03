# Adds the "Unlock with phone" tile to the Windows lock and sign-in screens.
#
#   1. Pair your phone in Notchy -> Phone first.
#   2. cargo build --release -p notchy-unlock
#   3. Right-click PowerShell -> Run as administrator, then:
#      powershell -ExecutionPolicy Bypass -File packaging\unlock\Install-NotchyUnlock.ps1
#
# Your password and PIN tiles keep working; this only adds a tile. Remove it any time with
# Uninstall-NotchyUnlock.ps1. The password is encrypted with Windows DPAPI (machine scope)
# and stored in a registry key only SYSTEM and Administrators can read.

$ErrorActionPreference = 'Stop'
$clsid = '{6E3F8C2A-4B1D-4F7A-9C55-2D8E1A7B9F31}'

$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) { throw 'Run this from an administrator PowerShell.' }

$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$dll = Join-Path $root 'target\release\notchy_unlock.dll'
if (-not (Test-Path $dll)) { throw "Build it first: cargo build --release -p notchy-unlock ($dll not found)" }

$link = Join-Path $env:USERPROFILE '.winisland\phone_link.json'
if (-not (Test-Path $link)) { throw 'Pair your phone in Notchy -> Phone first (phone_link.json not found).' }
$key = (Get-Content $link -Raw | ConvertFrom-Json).key
if ($key.Length -ne 64) { throw 'The phone pairing key looks wrong. Pair again in Notchy -> Phone.' }

$user = $env:USERNAME
$domain = $env:USERDOMAIN
Write-Host "Account: $domain\$user"
Write-Host 'Enter your Windows password (not the PIN). For a Microsoft account, its password.'
$secure = Read-Host -AsSecureString 'Password'
$plain = [Runtime.InteropServices.Marshal]::PtrToStringUni([Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure))

Add-Type -AssemblyName System.DirectoryServices.AccountManagement
try {
    $context = New-Object System.DirectoryServices.AccountManagement.PrincipalContext('Machine')
    if (-not $context.ValidateCredentials($user, $plain)) {
        Write-Warning 'Windows did not accept that password for a local check. Microsoft accounts sometimes fail this check; continuing.'
    }
} catch { Write-Warning "Could not check the password: $_" }

Add-Type -AssemblyName System.Security
$bytes = [Text.Encoding]::Unicode.GetBytes($plain + [char]0)
$protected = [Security.Cryptography.ProtectedData]::Protect($bytes, $null, 'LocalMachine')
$plain = $null

# The DLL.
$target = Join-Path $env:ProgramFiles 'Notchy'
New-Item -ItemType Directory -Force -Path $target | Out-Null
Copy-Item $dll (Join-Path $target 'notchy_unlock.dll') -Force

# Settings, readable by SYSTEM and Administrators only.
$settings = 'HKLM:\SOFTWARE\Notchy\Unlock'
New-Item -Path $settings -Force | Out-Null
Set-ItemProperty $settings -Name User -Value $user
Set-ItemProperty $settings -Name Domain -Value $domain
Set-ItemProperty $settings -Name Key -Value $key
Set-ItemProperty $settings -Name PcName -Value $env:COMPUTERNAME
Set-ItemProperty $settings -Name Password -Value $protected -Type Binary
$acl = New-Object System.Security.AccessControl.RegistrySecurity
$acl.SetAccessRuleProtection($true, $false)
foreach ($who in 'NT AUTHORITY\SYSTEM', 'BUILTIN\Administrators') {
    $acl.AddAccessRule((New-Object System.Security.AccessControl.RegistryAccessRule($who, 'FullControl', 'ContainerInherit', 'None', 'Allow')))
}
Set-Acl -Path $settings -AclObject $acl

# COM registration and the credential provider entry.
$com = "Registry::HKEY_CLASSES_ROOT\CLSID\$clsid"
New-Item -Path "$com\InprocServer32" -Force | Out-Null
Set-ItemProperty $com -Name '(default)' -Value 'Notchy phone unlock'
Set-ItemProperty "$com\InprocServer32" -Name '(default)' -Value (Join-Path $target 'notchy_unlock.dll')
Set-ItemProperty "$com\InprocServer32" -Name ThreadingModel -Value 'Apartment'
$provider = "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\Credential Providers\$clsid"
New-Item -Path $provider -Force | Out-Null
Set-ItemProperty $provider -Name '(default)' -Value 'Notchy phone unlock'

# Let the phone's answer in on private networks.
if (-not (Get-NetFirewallRule -DisplayName 'Notchy phone unlock' -ErrorAction SilentlyContinue)) {
    New-NetFirewallRule -DisplayName 'Notchy phone unlock' -Direction Inbound -Protocol UDP -LocalPort 47823 -Profile Private -Action Allow | Out-Null
}

Write-Host ''
Write-Host 'Done. Lock the PC (Win+L), pick "Unlock with phone", and approve on your phone.'
Write-Host 'Your password and PIN still work. To remove: Uninstall-NotchyUnlock.ps1'
