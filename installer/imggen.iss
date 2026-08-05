; ImgGen Windows 安装程序
; 使用 Inno Setup 6 编译。

#define MyAppName "ImgGen"
#ifndef MyAppVersion
  #define MyAppVersion "0.1.1"
#endif
#define MyAppPublisher "皊零"
#define MyAppURL "https://github.com/ling552/imgGenMCP"
#define MyAppExeName "imggen.exe"

[Setup]
AppId={{C4D5A6E7-0F3A-4C85-9B9E-1A6B3C8D2026}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}/releases
DefaultDirName={localappdata}\Programs\ImgGen
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=output
OutputBaseFilename=ImgGen-windows-x86_64-v{#MyAppVersion}-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
Uninstallable=yes
UninstallDisplayName={#MyAppName}
UninstallFilesDir={localappdata}\ImgGen\Uninstall

[Tasks]
Name: "desktopicon"; Description: "创建桌面快捷方式"; GroupDescription: "附加快捷方式："; Flags: unchecked

[Files]
Source: "..\target\release\imggen.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\assets\imggen-icon.svg"; DestDir: "{app}\assets"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\ImgGen"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\{#MyAppExeName}"; WorkingDir: "{app}"
Name: "{autodesktop}\ImgGen"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\{#MyAppExeName}"; WorkingDir: "{app}"; Tasks: desktopicon
Name: "{group}\MCP 配置说明"; Filename: "{app}\README.md"; IconFilename: "{app}\{#MyAppExeName}"

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "启动 ImgGen"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; 仅删除安装器创建的快捷方式和程序文件，不删除 {app}\data 中的用户配置、API Key、历史记录或图片。
Type: filesandordirs; Name: "{app}\assets"
Type: files; Name: "{app}\README.md"
Type: files; Name: "{app}\LICENSE"
Type: files; Name: "{app}\{#MyAppExeName}"
