
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
SetupIconFile={#AppRoot}\assets\icons\Vermouth.ico
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
// Remove that entry only if it points to THIS installation.
// AppData is intentionally retained unless the user explicitly opts in.

var
  RemoveUserData: Boolean;

procedure RemoveApplicationData(const Folder: String);
begin
  if DirExists(Folder) then
  begin
    if not DelTree(Folder, True, True, True) then
    begin
      MsgBox(
        'Vermouth could not completely delete this folder:' + #13#10 +
        Folder + #13#10#13#10 +
        'Close any applications using it, then remove the remaining ' +
        'files manually if desired.',
        mbError, MB_OK);
    end;
  end;
end;

procedure RemoveStartupRegistration();
var
  ExistingCommand: String;
  InstalledCommand: String;
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

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
  begin
    // Executed after the standard uninstall confirmation.
    // "No" retains personal data but does NOT cancel the uninstallation.
    RemoveUserData :=
      MsgBox(
        'Also remove Vermouth''s local application data?' + #13#10#13#10 +
        'Yes: Delete Vermouth settings, saved camera configurations, ' +
        'and caches stored in Vermouth-owned AppData folders.' + #13#10 +
        'No: Keep those files for a future installation.' + #13#10#13#10 +
        'Recordings or exports saved elsewhere will not be deleted.',
        mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES;

    RemoveStartupRegistration();
  end
  else if CurUninstallStep = usPostUninstall then
  begin
    if RemoveUserData then
    begin
      // Roaming AppData, current Vermouth storage location.
      RemoveApplicationData(ExpandConstant('{userappdata}\Vermouth'));

      // Local AppData for Vermouth-specific caches/settings, if present.
      RemoveApplicationData(ExpandConstant('{localappdata}\Vermouth'));
    end;
  end;
end;
