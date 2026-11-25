; Установщик для Windows (Inno Setup 6 или 7). Собирается из build-windows.sh:
;   ISCC -DAppVersion=1.3.4 -DAppArch=x64|arm64 -DSourceDir=… -DOutputDir=… -DOutputName=… geo-photo-library.iss
; Установка для текущего пользователя (без прав администратора) в %LOCALAPPDATA%\Programs;
; при запуске от администратора можно выбрать установку для всех.

#ifndef AppVersion
  #error Нужен -DAppVersion
#endif

[Setup]
AppId={{6F0B7E2A-3C1D-4F5E-9A8B-7C6D5E4F3A2B}
AppName=GeoPhotoLibrary
AppVersion={#AppVersion}
AppVerName=GeoPhotoLibrary {#AppVersion}
AppPublisher=EvilX
AppPublisherURL=https://evilx.github.io/geo-photo-library/
AppSupportURL=https://github.com/EvilX/geo-photo-library/issues
AppUpdatesURL=https://github.com/EvilX/geo-photo-library/releases
DefaultDirName={autopf}\GeoPhotoLibrary
DefaultGroupName=GeoPhotoLibrary
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
LicenseFile={#SourceDir}\LICENSE.txt
OutputDir={#OutputDir}
OutputBaseFilename={#OutputName}
SetupIconFile=geo-photo-library.ico
UninstallDisplayIcon={app}\bin\geo-photo-library.exe
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
#if AppArch == "arm64"
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
#else
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
#endif
MinVersion=10.0.17763

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
; Старые файлы удаляются перед обновлением: в новой версии набор DLL может быть другим.
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs ignoreversion

[InstallDelete]
Type: filesandordirs; Name: "{app}\bin"
Type: filesandordirs; Name: "{app}\lib"
Type: filesandordirs; Name: "{app}\share"

[Icons]
Name: "{autoprograms}\GeoPhotoLibrary"; Filename: "{app}\bin\geo-photo-library.exe"
Name: "{autodesktop}\GeoPhotoLibrary"; Filename: "{app}\bin\geo-photo-library.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\bin\geo-photo-library.exe"; Description: "{cm:LaunchProgram,GeoPhotoLibrary}"; Flags: nowait postinstall skipifsilent
