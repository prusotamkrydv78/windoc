; Notchy installer (Inno Setup 6). Per-user, no admin, no code-signing certificate needed.
; Built by packaging\installer\Build-NotchyInstaller.ps1, which passes the defines below.

#ifndef SourceDirectory
  #error "SourceDirectory must be provided."
#endif
#ifndef OutputDirectory
  #error "OutputDirectory must be provided."
#endif
#ifndef Version
  #error "Version must be provided."
#endif

#define AppName "Notchy"
; A new id: Notchy installs beside any old WinIsland install instead of replacing it.
#define AppId "{{6C2F3B1E-8E4A-4F7B-9C1D-2A5E7B9D4F31}"

[Setup]
AppId={#AppId}
AppName={#AppName}
AppVersion={#Version}
AppVerName={#AppName} {#Version}
AppPublisher=Notchy
VersionInfoVersion={#Version}
DefaultDirName={localappdata}\Programs\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDirectory}
OutputBaseFilename=Notchy-{#Version}-Setup
SetupIconFile={#SourceDirectory}\icon.ico
UninstallDisplayIcon={app}\Notchy.exe
UninstallDisplayName={#AppName}
LicenseFile={#SourceDirectory}\licenses\LICENSE
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; Quits a running Notchy before replacing it, and starts it again afterwards if asked.
CloseApplications=force
RestartApplications=no

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "{#SourceDirectory}\Notchy.exe"; DestDir: "{app}"; Flags: ignoreversion restartreplace
Source: "{#SourceDirectory}\READ-ME-FIRST.txt"; DestDir: "{app}"; Flags: ignoreversion isreadme
Source: "{#SourceDirectory}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\Notchy.exe"
Name: "{autoprograms}\{#AppName} (safe mode)"; Filename: "{app}\Notchy.exe"; Parameters: "--safe-mode"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\Notchy.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\Notchy.exe"; Description: "Launch {#AppName}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Quit Notchy before its files are removed. Settings in %USERPROFILE%\.winisland are kept.
Filename: "{sys}\taskkill.exe"; Parameters: "/IM Notchy.exe /F"; Flags: runhidden; RunOnceId: "StopNotchy"
