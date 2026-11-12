; Inno Setup 6 script for the KeeperFX Map Editor Windows installer.
;
; Compile from the repository root (Inno Setup 6.3 or newer):
;
;   iscc /DAppVersion=0.1.2 /DSourceDir=<staged folder> /DOutputDir=<output folder> ^
;        /DOutputBase=kfx-editor-v0.1.2-windows-x86_64-setup packaging\windows\installer.iss
;
; Every define is optional; the defaults are below. Relative paths given on the command line
; are resolved against this script's folder, not the current directory, so pass absolute paths.
; SourceDir must hold kfx-editor.exe, kfx-editor-cli.exe, README.md, CHANGELOG.md, LICENSE,
; THIRD_PARTY.md and docs\. IconFile defaults to packaging\icons\kfx-editor.ico.
;
; Setup switches (all documented in the Inno Setup help, "Setup Command Line Parameters"):
;   /SILENT              no wizard, progress window only
;   /VERYSILENT          no windows at all
;   /SUPPRESSMSGBOXES    answer message boxes with their default (with /SILENT or /VERYSILENT)
;   /NORESTART           never reboot
;   /DIR="x:\path"       install folder (a full path)
;   /CURRENTUSER         per-user install (the default, no administrator rights needed)
;   /ALLUSERS            install for all users into Program Files (asks for elevation)
;   /TASKS="list"        tasks to select: desktopicon, slbassoc (comma separated)
;   /LOG="file"          write a setup log to this file
; Switches read by this script itself (used by the editor's self-updater):
;   /RELAUNCH            start the editor when setup has finished, even when silent
;   /OPEN="path"         with /RELAUNCH, the file the editor opens on start
; Uninstall silently with: "<install folder>\unins000.exe" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

; The repository root, from this script's location (packaging\windows).
#define RepoRoot AddBackslash(SourcePath) + "..\.."

#ifndef SourceDir
  #define SourceDir RepoRoot + "\stage\kfx-editor-v" + AppVersion + "-windows-x86_64"
#endif
#ifndef OutputDir
  #define OutputDir RepoRoot + "\out"
#endif
#ifndef OutputBase
  #define OutputBase "kfx-editor-v" + AppVersion + "-windows-x86_64-setup"
#endif
#ifndef IconFile
  #define IconFile RepoRoot + "\packaging\icons\kfx-editor.ico"
#endif

#if !FileExists(SourceDir + "\kfx-editor.exe")
  #error kfx-editor.exe not found in SourceDir; pass /DSourceDir=<staged folder>
#endif
#if !FileExists(IconFile)
  #error Icon not found; add packaging\icons\kfx-editor.ico or pass /DIconFile=<file>
#endif

#define AppName "KeeperFX Map Editor"
#define AppExe "kfx-editor.exe"
#define RepoUrl "https://github.com/eddoww/keeperfx-editor"
#define ProgId "KeeperFX.MapEditor.slb"

[Setup]
; AppId identifies the installation in the registry. It must never change: upgrading in place,
; reusing the previous folder, install mode and tasks, and the uninstall entry all depend on it.
AppId={{8025DB88-FBE2-4DAB-847E-90656C8AE937}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=KeeperFX Map Editor project
AppPublisherURL={#RepoUrl}
AppSupportURL={#RepoUrl}/issues
AppUpdatesURL={#RepoUrl}/releases
LicenseFile={#RepoRoot}\LICENSE
SetupIconFile={#IconFile}
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}

; Per-user by default, so no administrator prompt; the user (dialog) or /ALLUSERS can choose an
; all-users install instead. {autopf} is %LOCALAPPDATA%\Programs per user and Program Files for
; all users. UsePreviousPrivileges (default yes) makes upgrades keep the mode used before.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog commandline
DefaultDirName={autopf}\{#AppName}
UsePreviousAppDir=yes
DisableProgramGroupPage=yes

ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0

; Close a running editor through the Restart Manager before replacing its files. The editor is
; started again by the [Run] entries below, not by the Restart Manager.
CloseApplications=yes
CloseApplicationsFilter=*.exe
RestartApplications=no

; Refresh Explorer after the optional .slb registration (and after uninstall).
ChangesAssociations=yes

OutputDir={#OutputDir}
OutputBaseFilename={#OutputBase}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "slbassoc"; Description: "Add ""Open with {#AppName}"" for .slb map files"; GroupDescription: "File types:"; Flags: unchecked

[InstallDelete]
; docs\ is entirely ours: clear it so pages removed in a newer version do not linger.
Type: filesandordirs; Name: "{app}\docs"

[Files]
Source: "{#SourceDir}\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\kfx-editor-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\CHANGELOG.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\THIRD_PARTY.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\docs\*"; DestDir: "{app}\docs"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#IconFile}"; DestDir: "{app}"; DestName: "kfx-editor.ico"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
; .slb is a generic extension, so the editor only adds itself to "Open with" (OpenWithProgids,
; an empty string value named after the ProgID) and never takes the extension's default.
; HKA is HKCU\Software\Classes per user and HKLM\Software\Classes for all users.
Root: HKA; Subkey: "Software\Classes\.slb"; Flags: uninsdeletekeyifempty; Tasks: slbassoc
Root: HKA; Subkey: "Software\Classes\.slb\OpenWithProgids"; Flags: uninsdeletekeyifempty; Tasks: slbassoc
Root: HKA; Subkey: "Software\Classes\.slb\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue; Tasks: slbassoc
Root: HKA; Subkey: "Software\Classes\{#ProgId}"; ValueType: string; ValueName: ""; ValueData: "KeeperFX map"; Flags: uninsdeletekey; Tasks: slbassoc
Root: HKA; Subkey: "Software\Classes\{#ProgId}\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\kfx-editor.ico"; Tasks: slbassoc
Root: HKA; Subkey: "Software\Classes\{#ProgId}\shell\open"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "{#AppName}"; Tasks: slbassoc
Root: HKA; Subkey: "Software\Classes\{#ProgId}\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: slbassoc

[UninstallDelete]
; Marker written in [Code] below, and leftovers of the portable self-updater.
Type: files; Name: "{app}\setup-install"
Type: filesandordirs; Name: "{app}\.kfx-editor-previous"
Type: filesandordirs; Name: "{app}\.kfx-update-staging"
Type: files; Name: "{app}\*.old"

[Run]
; "Launch now" checkbox on the last page of an interactive install.
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent; Check: not RelaunchRequested
; Silent self-update: start the editor again, optionally with the file that was open.
; runasoriginaluser keeps the editor unelevated when setup elevated itself (all-users install).
Filename: "{app}\{#AppExe}"; Parameters: "{code:RelaunchParams}"; Flags: nowait runasoriginaluser; Check: RelaunchRequested

[Code]
{ True when setup was started with /RELAUNCH (by the editor's self-updater). }
function RelaunchRequested: Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), '/RELAUNCH') = 0 then
    begin
      Result := True;
      Exit;
    end;
end;

{ The editor's arguments for the relaunch: the /OPEN= path, quoted, or nothing. }
function RelaunchParams(Param: String): String;
var
  Path: String;
begin
  Path := ExpandConstant('{param:OPEN|}');
  if Path <> '' then
    Result := '"' + Path + '"'
  else
    Result := '';
end;

{ The marker tells the editor it was installed by this setup, so it updates by running the newer
  setup instead of replacing its own files. }
procedure CurStepChanged(CurStep: TSetupStep);
var
  Marker: String;
begin
  if CurStep = ssPostInstall then
  begin
    Marker := ExpandConstant('{app}\setup-install');
    if not SaveStringToFile(Marker, 'Installed by the KeeperFX Map Editor setup; the editor updates itself by running the newer setup.' + #13#10, False) then
      Log('Could not write ' + Marker);
  end;
end;
