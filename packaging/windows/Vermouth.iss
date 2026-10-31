; Vermouth Windows installer (Inno Setup 6.3+).
; Variables are supplied by packaging/windows/build.ps1.
#ifndef AppVersion
  #error AppVersion must be provided by build.ps1.
#endif
#ifndef AppPublisher
  #error AppPublisher must be provided by build.ps1.
#endif
#ifndef AppBinary
  #error AppBinary must be provided by build.ps1.
#endif
#ifndef AppRoot
  #error AppRoot must be provided by build.ps1.
#endif

#define AppName "Vermouth"
#define AppExeName "Vermouth.exe"

[Setup]
; Keep this identifier stable in future releases for clean upgrades.
AppId=LucasErrNotFound.Vermouth
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
#ifdef AppRepositoryUrl
AppPublisherURL={#AppRepositoryUrl}
AppSupportURL={#AppRepositoryUrl}/issues
AppUpdatesURL={#AppRepositoryUrl}/releases
#endif
DefaultDirName={localappdata}\Programs\{#AppName}
DefaultGroupName={#AppName}
DisableDirPage=no
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
#ifdef AppLicense
LicenseFile={#AppLicense}
#endif
OutputBaseFilename=Vermouth-{#AppVersion}-windows-x64-Setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
SetupIconFile={#AppRoot}\assets\icons\vermouth.ico
UninstallDisplayIcon={app}\{#AppExeName}
CloseApplications=yes
RestartApplications=no
VersionInfoVersion={#AppVersion}.0
VersionInfoCompany={#AppPublisher}
VersionInfoDescription=Vermouth Installer
VersionInfoProductName={#AppName}
VersionInfoProductVersion={#AppVersion}.0

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Files]
Source: "{#AppBinary}"; DestDir: "{app}"; DestName: "{#AppExeName}"; Flags: ignoreversion
#ifdef AppReadme
Source: "{#AppReadme}"; DestDir: "{app}"; DestName: "README.md"; Flags: ignoreversion
#endif
#ifdef AppLicense
Source: "{#AppLicense}"; DestDir: "{app}"; DestName: "LICENSE"; Flags: ignoreversion
#endif

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExeName}"; WorkingDir: "{app}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExeName}"; WorkingDir: "{app}"; Description: "Launch {#AppName}"; Flags: nowait postinstall skipifsilent unchecked

[Code]
// The application registers its own current-user Run entry when enabled.
// Remove that entry on uninstall only if it points to THIS installation.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  ExistingCommand: String;
  InstalledCommand: String;
begin
  if CurUninstallStep = usUninstall then
  begin
    InstalledCommand := '"' + ExpandConstant('{app}\{#AppExeName}') + '"';
    if RegQueryStringValue(HKCU64,
      'Software\Microsoft\Windows\CurrentVersion\Run',
      'Vermouth', ExistingCommand) then
    begin
      if CompareText(ExistingCommand, InstalledCommand) = 0 then
        RegDeleteValue(HKCU64,
          'Software\Microsoft\Windows\CurrentVersion\Run', 'Vermouth');
    end;
  end;
end;
