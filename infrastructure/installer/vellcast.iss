; Instalador Windows do Vellcast (Inno Setup 6). ADR-045.
; Compilado por scripts/windows/Build-Package.ps1 (workflows release.yml e smoke.yml):
;   ISCC.exe /DAppVersion=0.1.0 /DSourceDir=<pasta do pacote> /DOutputDir=<saída> vellcast.iss
; Instala por usuário (sem administrador). Instalação/desinstalação silenciosa:
;   Vellcast-Setup-x64.exe /VERYSILENT /SUPPRESSMSGBOXES /CURRENTUSER [/DIR=...]
;   unins000.exe /VERYSILENT /SUPPRESSMSGBOXES

#ifndef AppVersion
  #define AppVersion "0.0.0-dev"
#endif
#ifndef SourceDir
  #define SourceDir "..\..\dist\Vellcast-MVP-windows-x64"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\dist"
#endif

[Setup]
AppId={{6F4D3C1E-2B7A-4E8F-9C51-5A0D7E3B9F21}
AppName=Vellcast
AppVersion={#AppVersion}
AppVerName=Vellcast {#AppVersion}
AppPublisher=Next Estúdios
AppPublisherURL=https://nextestudios.com
AppSupportURL=https://github.com/nextestudios/Vellcast
DefaultDirName={autopf}\Vellcast
DefaultGroupName=Vellcast
DisableProgramGroupPage=yes
; Por usuário: não exige administrador; o usuário pode escolher instalar para todos.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Windows 10 2004 (build 19041): mínimo para captura por janela e áudio por processo.
MinVersion=10.0.19041
OutputDir={#OutputDir}
OutputBaseFilename=Vellcast-Setup-x64
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
LicenseFile={#SourceDir}\LICENSE.txt
UninstallDisplayIcon={app}\vellcast-desktop.exe
UninstallDisplayName=Vellcast
; Fecha o app e o servidor local antes de atualizar ou desinstalar.
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
brazilianportuguese.AutoStart=Iniciar o Vellcast com o Windows
english.AutoStart=Start Vellcast with Windows
brazilianportuguese.RemoveUserData=Remover também as configurações e salas salvas do Vellcast?
english.RemoveUserData=Also remove Vellcast settings and saved rooms?

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
; Desligado por padrão (spec §3.6 e §13: nada roda em segundo plano sem escolha do usuário).
Name: "autostart"; Description: "{cm:AutoStart}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\vellcast-desktop.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\vellcast-relay.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\player\*"; DestDir: "{app}\player"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#SourceDir}\LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LEIA-ME.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\iniciar-relay.bat"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Vellcast"; Filename: "{app}\vellcast-desktop.exe"
Name: "{group}\{cm:UninstallProgram,Vellcast}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Vellcast"; Filename: "{app}\vellcast-desktop.exe"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Vellcast"; ValueData: """{app}\vellcast-desktop.exe"""; Flags: uninsdeletevalue; Tasks: autostart

[Run]
Filename: "{app}\vellcast-desktop.exe"; Description: "{cm:LaunchProgram,Vellcast}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Encerra um servidor local iniciado pelo app, se ainda estiver rodando.
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM vellcast-relay.exe"; Flags: runhidden; RunOnceId: "StopLocalRelay"

[UninstallDelete]
Type: filesandordirs; Name: "{app}\player"

[Code]
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  DataDir: string;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    { Configurações, salas e segredo do servidor local (directories::ProjectDirs). }
    DataDir := ExpandConstant('{userappdata}\Next Estudios\Vellcast');
    if DirExists(DataDir) and (not UninstallSilent) then
      if MsgBox(ExpandConstant('{cm:RemoveUserData}'), mbConfirmation, MB_YESNO) = IDYES then
        DelTree(DataDir, True, True, True);
  end;
end;
