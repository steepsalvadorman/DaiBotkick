; ─────────────────────────────────────────────────────────────────────────────
; DaiBot — Instalador para Windows
; Requiere Inno Setup 6: https://jrsoftware.org/isdl.php
; ─────────────────────────────────────────────────────────────────────────────

#define AppName    "DaiBot"
#define AppVersion "1.0"
#define AppExe     "daibot.exe"

; ── Credenciales de la app OAuth de Kick (kick.com/settings/developer) ────────
; Crea UNA app con redirect URL: http://localhost:3001/callback
; y pon tus credenciales aquí antes de compilar el instalador.
#define KickClientId     "01K30C6XJXKREZF75EW23ENYBT"
#define KickClientSecret "b6397ebb9bc13dae47b1e819da780e56e267c24633a365ced4ace4e9e6c71e1e"

[Setup]
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=DaiBot
AppPublisherURL=https://kick.com
DefaultDirName={localappdata}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
OutputDir=output
OutputBaseFilename=DaiBot
#if FileExists("icon.ico")
SetupIconFile=icon.ico
#endif
; Sin UAC — se instala en %LOCALAPPDATA% del usuario
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=
; El instalador cierra daibot.exe solo via taskkill en InitializeSetup
; (no usar CloseApplications para evitar el dialog de "proceso en uso")
CloseApplications=no
; Compresión máxima
Compression=lzma2/ultra64
SolidCompression=yes
; Mínimo Windows 10
MinVersion=10.0
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "spanish"; MessagesFile: "compiler:Languages\Spanish.isl"

[Tasks]
Name: "autostart"; Description: "Iniciar DaiBot automáticamente con Windows"; GroupDescription: "Opciones:"
Name: "desktopicon"; Description: "Crear acceso directo en el Escritorio"; GroupDescription: "Accesos directos:"

[Files]
; Ejecutable principal
Source: "..\backend\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion

; Overlay para OBS (pixel art 1920x1080 con chat, cola de videos, TTS, alertas)
Source: "..\overlay\*"; DestDir: "{app}\overlay"; Flags: ignoreversion recursesubdirs createallsubdirs

; Datos persistentes
Source: "..\data\*"; DestDir: "{app}\data"; Flags: ignoreversion recursesubdirs createallsubdirs skipifsourcedoesntexist

; Plantilla de configuración
Source: "..\.env.example"; DestDir: "{app}"; Flags: ignoreversion

; Referencia de comandos del chat
Source: "..\comandos.html"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\comandos.png";  DestDir: "{app}"; Flags: ignoreversion

; Icono de la aplicación (para accesos directos)
#if FileExists("icon.ico")
Source: "icon.ico"; DestDir: "{app}"; DestName: "daibot.ico"; Flags: ignoreversion
#endif

; Python portable + edge-tts (ya incluido, el usuario no instala nada)
Source: "python_portable\*"; DestDir: "{app}\python"; Flags: ignoreversion recursesubdirs createallsubdirs skipifsourcedoesntexist

[Dirs]
Name: "{app}\data"
Name: "{app}\data\tts_cache"

[Icons]
; Lanzar el bot
Name: "{group}\{#AppName}";         Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"; IconFilename: "{app}\daibot.ico"; Comment: "Inicia DaiBot"
; Configurar OAuth (primer uso)
Name: "{group}\Configurar OAuth";   Filename: "{app}\{#AppExe}"; Parameters: "--login"; WorkingDir: "{app}"; IconFilename: "{app}\daibot.ico"; Comment: "Autentica DaiBot con tu cuenta de Kick"
; Comandos del chat
Name: "{group}\Comandos del Chat";  Filename: "{app}\comandos.html"; Comment: "Lista de comandos disponibles"
; Desinstalar
Name: "{group}\Desinstalar {#AppName}"; Filename: "{uninstallexe}"

Name: "{userdesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"; IconFilename: "{app}\daibot.ico"; Tasks: desktopicon

[Registry]
; Autostart opcional
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "{#AppName}"; ValueData: """{app}\{#AppExe}"""; Flags: uninsdeletevalue; Tasks: autostart

[Code]
// ─── Reemplaza o añade una clave KEY="value" en el .env ──────────────────────

procedure SetEnvKey(EnvPath, Key, Value: string);
var
  Lines: TStringList;
  Prefix, Line: string;
  i: Integer;
  Found: Boolean;
begin
  Lines := TStringList.Create;
  try
    if FileExists(EnvPath) then
      Lines.LoadFromFile(EnvPath);
    Prefix := Key + '=';
    Line   := Key + '="' + Value + '"';
    Found  := False;
    for i := 0 to Lines.Count - 1 do begin
      if Pos(Prefix, Lines[i]) = 1 then begin
        Lines[i] := Line;
        Found := True;
        Break;
      end;
    end;
    if not Found then Lines.Add(Line);
    Lines.SaveToFile(EnvPath);
  finally
    Lines.Free;
  end;
end;

// ─── Mensaje antes de instalar ────────────────────────────────────────────────

function InitializeSetup(): Boolean;
var
  ResultCode: Integer;
begin
  // Cerrar daibot si esta corriendo (actualizacion silenciosa)
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM {#AppExe}',
       '', SW_HIDE, ewWaitUntilTerminated, ResultCode);

  // Aviso sobre SmartScreen
  MsgBox(
    'NOTA: Windows puede mostrar una advertencia de seguridad.' + #13#10#13#10 +
    'Si aparece la pantalla azul "Windows protegió su PC":' + #13#10 +
    '  1. Haz clic en "Más información"' + #13#10 +
    '  2. Luego haz clic en "Ejecutar de todas formas"' + #13#10#13#10 +
    'Esto ocurre porque el instalador es nuevo y Windows' + #13#10 +
    'aún no lo reconoce. No es un virus.',
    mbInformation, MB_OK);

  MsgBox(
    'Bienvenido a DaiBot' + #13#10#13#10 +
    'Esto es lo que se va a instalar en tu computadora:' + #13#10#13#10 +
    '  [+] El bot de Kick' + #13#10 +
    '      Se conecta a tu canal y gestiona el chat,' + #13#10 +
    '      comandos, alertas y sorteos.' + #13#10#13#10 +
    '  [+] El overlay para OBS' + #13#10 +
    '      Pantalla con el chat en vivo, cola de videos' + #13#10 +
    '      y alertas en tiempo real.' + #13#10#13#10 +
    '  [+] Texto a voz (TTS)' + #13#10 +
    '      Ya viene incluido, no necesitas instalar' + #13#10 +
    '      nada extra.' + #13#10#13#10 +
    'Al terminar, el bot abrira tu navegador para que' + #13#10 +
    'inicies sesion con tu cuenta de Kick.' + #13#10 +
    'Solo una vez y listo, sin pasos tecnicos.',
    mbInformation, MB_OK);
  Result := True;
end;

// ─── Pasos post-instalación ───────────────────────────────────────────────────

procedure CurStepChanged(CurStep: TSetupStep);
var
  AppDir, EnvDst, EnvSrc: string;
begin
  if CurStep <> ssPostInstall then Exit;

  AppDir := ExpandConstant('{app}');
  EnvSrc := AppDir + '\.env.example';
  EnvDst := AppDir + '\.env';

  // ── 1. Crear .env desde la plantilla (si no existe ya) ───────────────────
  if not FileExists(EnvDst) then
    if not CopyFile(EnvSrc, EnvDst, False) then
      MsgBox('No se pudo crear el archivo de configuracion.', mbError, MB_OK);

  // ── 2. Escribir credenciales y rutas en .env ─────────────────────────────
  SetEnvKey(EnvDst, 'KICK_CLIENT_ID',     '{#KickClientId}');
  SetEnvKey(EnvDst, 'KICK_CLIENT_SECRET', '{#KickClientSecret}');
  SetEnvKey(EnvDst, 'OVERLAY_DIR',   'overlay');
  SetEnvKey(EnvDst, 'QUEUE_FILE',    'data/queue.json');
  SetEnvKey(EnvDst, 'TTS_CACHE_DIR', 'data/tts_cache');

  // ── 3. Instrucciones finales ───────────────────────────────────────────────
  MsgBox('Instalacion completada.' + #13#10#13#10 +

         'PRIMER USO (solo 2 pasos):' + #13#10 +
         '  1. Haz doble clic en el icono DaiBot del Escritorio' + #13#10 +
         '  2. Se abrira el navegador - inicia sesion con tu' + #13#10 +
         '     cuenta de Kick y acepta los permisos.' + #13#10 +
         '     El bot se configura solo.' + #13#10#13#10 +

         'OVERLAY EN OBS:' + #13#10 +
         '  Fuente de Navegador (Browser Source):' + #13#10 +
         '  URL: http://localhost:3000/pixel.html' + #13#10 +
         '  Ancho: 1920  Alto: 1080',
         mbInformation, MB_OK);
end;
