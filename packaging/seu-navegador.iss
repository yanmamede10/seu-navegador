; Seu Navegador — Windows x64 Installer (Inno Setup 6)
; Per-user install by default (no admin UAC required), optional all-users override,
; zero Microsoft runtime dependencies, no MSIX.

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif

#ifndef DistDir
  #define DistDir "..\dist\seu-navegador-win64"
#endif

#ifndef OutputDir
  #define OutputDir "..\dist"
#endif

[Setup]
AppId={{7B6E09F6-4F7C-4D3A-9C35-5E0AA7E6AD01}
AppName=Seu Navegador
AppVersion={#AppVersion}
AppVerName=Seu Navegador {#AppVersion}
AppPublisher=Yan Mamede
AppPublisherURL=https://github.com/yanmamede10/seu-navegador
AppSupportURL=https://github.com/yanmamede10/seu-navegador/issues
AppUpdatesURL=https://github.com/yanmamede10/seu-navegador/releases
DefaultDirName={autopf}\Seu Navegador
DefaultGroupName=Seu Navegador
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
LicenseFile=..\LICENSE-MIT
SetupIconFile=..\crates\app\assets\icon.ico
UninstallDisplayIcon={app}\seu-navegador.exe
OutputDir={#OutputDir}
OutputBaseFilename=seu-navegador-{#AppVersion}-win64-setup
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ChangesAssociations=yes

[Languages]
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#DistDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Seu Navegador"; Filename: "{app}\seu-navegador.exe"; IconFilename: "{app}\seu-navegador.exe"
Name: "{autodesktop}\Seu Navegador"; Filename: "{app}\seu-navegador.exe"; IconFilename: "{app}\seu-navegador.exe"; Tasks: desktopicon

[Registry]
; Register `.seutheme` file type description & icon under current user (or HKLM if admin)
Root: HKA; Subkey: "Software\Classes\.seutheme"; ValueType: string; ValueName: ""; ValueData: "SeuNavegador.ThemePackage"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\SeuNavegador.ThemePackage"; ValueType: string; ValueName: ""; ValueData: "Pacote de Tema do Seu Navegador (.seutheme)"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\SeuNavegador.ThemePackage\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\seu-navegador.exe,0"

[Run]
Filename: "{app}\seu-navegador.exe"; Description: "{cm:LaunchProgram,Seu Navegador}"; Flags: nowait postinstall skipifsilent
