# build.ps1 - Compila el bot y genera DaiBot.exe (instalador completo)
# Ejecutar desde la carpeta installer/: .\build.ps1

$OutputEncoding = [System.Text.Encoding]::UTF8
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$ErrorActionPreference = "Stop"
$Root = Split-Path $PSScriptRoot -Parent

# 1. Compilar el bot en modo release
Write-Host "Compilando DaiBot..." -ForegroundColor Cyan
Push-Location "$Root\backend"
# cargo escribe a stderr normalmente; suspender Stop para no fallar por eso
$ErrorActionPreference = "Continue"
cargo build --release
$cargoExit = $LASTEXITCODE
$ErrorActionPreference = "Stop"
if ($cargoExit -ne 0) { Write-Error "cargo build --release fallo"; exit 1 }
Pop-Location
Write-Host "OK: $Root\backend\target\release\daibot.exe" -ForegroundColor Green

# 2. Generar icon.ico desde icon_source.png (si existe)
$IconSource = "$PSScriptRoot\icon_source.png"
$IconOutput = "$PSScriptRoot\icon.ico"
if (Test-Path $IconSource) {
    Write-Host "Generando icono..." -ForegroundColor Cyan
    & "$PSScriptRoot\make_icon.ps1"
    if ($LASTEXITCODE -ne 0) { Write-Error "make_icon.ps1 fallo"; exit 1 }
} elseif (-not (Test-Path $IconOutput)) {
    Write-Host "AVISO: no se encontro icon_source.png, se usara icono por defecto." -ForegroundColor Yellow
}

# 3. Python portable + edge-tts (se incluye en el instalador, sin que el usuario instale nada)
$PyDir   = "$PSScriptRoot\python_portable"
$EdgeTts = "$PyDir\Scripts\edge-tts.exe"

if (-not (Test-Path $EdgeTts)) {
    Write-Host "Preparando Python portable con edge-tts..." -ForegroundColor Cyan

    # Limpiar si habia un intento previo incompleto
    if (Test-Path $PyDir) { Remove-Item $PyDir -Recurse -Force }

    # Descargar Python 3.12 embeddable (x64, ~12 MB, sin instalador)
    $PyVersion = "3.12.9"
    $PyZip     = "$env:TEMP\python-embed-amd64.zip"
    if (-not (Test-Path $PyZip)) {
        Write-Host "  Descargando Python $PyVersion embeddable..." -ForegroundColor Gray
        Invoke-WebRequest -Uri "https://www.python.org/ftp/python/$PyVersion/python-$PyVersion-embed-amd64.zip" `
                          -OutFile $PyZip -UseBasicParsing
    }
    Expand-Archive $PyZip -DestinationPath $PyDir

    # Habilitar site-packages (requerido para pip e instalacion de paquetes)
    $pthFile = Get-ChildItem $PyDir -Filter "python*._pth" | Select-Object -First 1
    if ($pthFile) {
        (Get-Content $pthFile.FullName -Raw) -replace '#import site', 'import site' |
            Set-Content $pthFile.FullName -Encoding UTF8
    }

    # Instalar pip
    Write-Host "  Instalando pip..." -ForegroundColor Gray
    $getPip = "$env:TEMP\get-pip.py"
    if (-not (Test-Path $getPip)) {
        Invoke-WebRequest -Uri "https://bootstrap.pypa.io/get-pip.py" -OutFile $getPip -UseBasicParsing
    }
    & "$PyDir\python.exe" $getPip --no-warn-script-location --quiet
    if ($LASTEXITCODE -ne 0) { Write-Error "Fallo al instalar pip"; exit 1 }

    # Instalar edge-tts y sus dependencias
    Write-Host "  Instalando edge-tts..." -ForegroundColor Gray
    & "$PyDir\python.exe" -m pip install edge-tts --no-warn-script-location --quiet
    if ($LASTEXITCODE -ne 0) { Write-Error "Fallo al instalar edge-tts"; exit 1 }

    Write-Host "Python portable listo: $PyDir" -ForegroundColor Green
} else {
    Write-Host "Python portable ya existe, omitiendo descarga." -ForegroundColor Gray
}

# 4. Buscar Inno Setup
$IsccPaths = @(
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles}\Inno Setup 6\ISCC.exe"
)
$Iscc = $IsccPaths | Where-Object { Test-Path $_ } | Select-Object -First 1

if (-not $Iscc) {
    Write-Host ""
    Write-Host "ERROR: Inno Setup 6 no encontrado." -ForegroundColor Red
    Write-Host "Descargalo en: https://jrsoftware.org/isdl.php" -ForegroundColor Yellow
    exit 1
}

# 5. Compilar el instalador
Write-Host "Generando instalador..." -ForegroundColor Cyan
Push-Location $PSScriptRoot
& $Iscc "DaiBot.iss"
if ($LASTEXITCODE -ne 0) { Write-Error "ISCC.exe fallo"; exit 1 }
Pop-Location

Write-Host ""
Write-Host "Instalador listo: $PSScriptRoot\output\DaiBot.exe" -ForegroundColor Green
