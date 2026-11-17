; Marah Launcher's Windows installer, with Inno Setup 6 (https://jrsoftware.org/isinfo.php), from the
; Windows folder scripts/package.sh made. Run from the repository root:
;   iscc /DAppVersion=0.1.0 /DFolder=dist\marah-launcher-windows-x86_64 /DOutputDir=dist packaging\windows\marah-launcher.iss
; It installs for the current user only, into their local programs folder, so neither installing
; nor updating needs an administrator. It adds a Start menu entry, an optional desktop shortcut,
; and an uninstaller in Windows' installed apps. The uninstaller removes the program folder only:
; settings, plugins' data and emulators stay in the data folder.
;
; A marker file, installed-by-setup, tells Marah Launcher it was installed this way: its update then runs
; the next release's installer silently (/VERYSILENT), and the [Run] entry starts the new version.

#ifndef AppVersion
  #error Pass the version: /DAppVersion=0.1.0
#endif
#ifndef Folder
  #define Folder "dist\marah-launcher-windows-x86_64"
#endif
#ifndef OutputDir
  #define OutputDir "dist"
#endif

[Setup]
; Never change AppId: Windows tells an update from a second install by it.
AppId={{E61BFC70-9A4B-4444-951B-9B8FFB796BDF}
AppName=Marah Launcher
AppVersion={#AppVersion}
AppVerName=Marah Launcher {#AppVersion}
AppPublisher=Marah Launcher
AppPublisherURL=https://github.com/MarahHQ/marah-launcher
AppSupportURL=https://github.com/MarahHQ/marah-launcher/issues
AppUpdatesURL=https://github.com/MarahHQ/marah-launcher/releases
DefaultDirName={localappdata}\Programs\Marah Launcher
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=marah-launcher-windows-x86_64-setup
SetupIconFile=..\..\assets\marah-launcher.ico
UninstallDisplayIcon={app}\marah-launcher.exe
UninstallDisplayName=Marah Launcher
LicenseFile=..\..\LICENSE
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; Marah Launcher closes before its files are replaced, and the update starts it again ([Run]).
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\..\{#Folder}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Dirs]
; Third-party plugins and themes, found next to marah-launcher.exe.
Name: "{app}\extensions"

[Icons]
Name: "{autoprograms}\Marah Launcher"; Filename: "{app}\marah-launcher.exe"
Name: "{autodesktop}\Marah Launcher"; Filename: "{app}\marah-launcher.exe"; Tasks: desktopicon

[Run]
; Offered after an install; after a silent update it runs by itself, starting the new version.
Filename: "{app}\marah-launcher.exe"; Description: "{cm:LaunchProgram,Marah Launcher}"; Flags: nowait postinstall

[UninstallDelete]
Type: files; Name: "{app}\installed-by-setup"
Type: dirifempty; Name: "{app}\extensions"
Type: dirifempty; Name: "{app}"

[Code]
procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    SaveStringToFile(ExpandConstant('{app}\installed-by-setup'), 'Windows installer', False);
end;
