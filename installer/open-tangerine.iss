; Open Tangerine for Windows — Inno Setup Script
;
; Defines a clean 64-bit Windows installer with:
; - Program files installation into {autopf}\Open Tangerine
; - Start Menu & Desktop shortcuts
; - Optional autostart on Windows sign-in
; - Bundled FFmpeg utilities & PDFium engine
; - Clean uninstaller cleanup

#define MyAppName "Open Tangerine"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "Open Tangerine Contributors"
#define MyAppURL "https://github.com/open-tangerine/open-tangerine"
#define MyAppExeName "open-tangerine.exe"

[Setup]
; Basic Application Info
AppId={{8B59275F-2E0D-48E6-92F6-29F741C04AE9}}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}

; Destination Directories
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes

; Output Configuration
OutputDir=..\dist
OutputBaseFilename=OpenTangerine-Setup-{#MyAppVersion}
Compression=lzma2/ultra64
SolidCompression=yes

; Architecture (64-bit Windows)
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

; UI and Appearance
WizardStyle=modern
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
SetupIconFile=app.ico
UninstallDisplayIcon={app}\{#MyAppExeName}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "autostart"; Description: "Start Open Tangerine automatically when Windows starts"; GroupDescription: "Startup options:"

[Files]
; Main Executable
Source: "..\dist\OpenTangerine\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion

; PDF Engine
Source: "..\dist\OpenTangerine\pdfium.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist

; FFmpeg Media Tools
Source: "..\dist\OpenTangerine\ffmpeg\*"; DestDir: "{app}\ffmpeg"; Flags: ignoreversion recursesubdirs createallsubdirs skipifsourcedoesntexist

; Licenses & Source Offer Notices
Source: "..\dist\OpenTangerine\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs skipifsourcedoesntexist
Source: "..\dist\OpenTangerine\SOURCE-OFFER.txt"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{group}\{cm:UninstallProgram,{#MyAppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Registry]
; Run on Windows startup (optional task)
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "OpenTangerine"; ValueData: """{app}\{#MyAppExeName}"""; Flags: uninsdeletevalue; Tasks: autostart

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent
