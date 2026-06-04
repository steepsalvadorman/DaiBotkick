# make_icon.ps1 - Convierte icon_source.png a icon.ico (multi-resolucion, todo PNG)
# PNG-in-ICO es el estandar moderno en Windows Vista+/10/11

$OutputEncoding = [System.Text.Encoding]::UTF8
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$Source = Join-Path $PSScriptRoot "icon_source.png"
$Output = Join-Path $PSScriptRoot "icon.ico"

if (-not (Test-Path $Source)) {
    Write-Host "ERROR: No se encontro icon_source.png" -ForegroundColor Red
    exit 1
}

Add-Type -AssemblyName System.Drawing

Write-Host "Convirtiendo $Source a ICO..." -ForegroundColor Cyan

$sizes    = @(16, 32, 48, 64, 128, 256)
$original = [System.Drawing.Bitmap]::new($Source)

$frames = @()
foreach ($size in $sizes) {
    $bmp = New-Object System.Drawing.Bitmap($size, $size,
        [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode  = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.SmoothingMode      = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
    $g.PixelOffsetMode    = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
    $g.Clear([System.Drawing.Color]::Transparent)
    $g.DrawImage($original, 0, 0, $size, $size)
    $g.Dispose()

    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $frames += ,@{ Size = $size; Data = $ms.ToArray() }
    $bmp.Dispose()
    $ms.Dispose()
}
$original.Dispose()

# Construir archivo ICO
$out = New-Object System.IO.MemoryStream
$w   = New-Object System.IO.BinaryWriter($out)

# ICONDIR
$w.Write([uint16]0)
$w.Write([uint16]1)
$w.Write([uint16]$frames.Count)

# Directorio
$offset = 6 + $frames.Count * 16
foreach ($f in $frames) {
    $dim = if ($f.Size -ge 256) { 0 } else { $f.Size }
    $w.Write([byte]$dim)
    $w.Write([byte]$dim)
    $w.Write([byte]0)
    $w.Write([byte]0)
    $w.Write([uint16]1)
    $w.Write([uint16]32)
    $w.Write([uint32]$f.Data.Length)
    $w.Write([uint32]$offset)
    $offset += $f.Data.Length
}

foreach ($f in $frames) { $w.Write($f.Data) }

$w.Flush()
[System.IO.File]::WriteAllBytes($Output, $out.ToArray())
$out.Dispose()
$w.Dispose()

Write-Host "Icono generado: $Output ($([System.IO.File]::ReadAllBytes($Output).Length) bytes)" -ForegroundColor Green
