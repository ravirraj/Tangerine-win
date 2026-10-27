param (
    [string]$SourcePath = "C:\Users\thorfinn\.gemini\antigravity\brain\46b47e8c-cd3b-41d9-b75b-75c013faf3a2\.user_uploaded\media_1793091516984_778164bb.jpg"
)

Add-Type -AssemblyName System.Drawing

$assetsDir = Join-Path $PSScriptRoot "..\assets"
$installerDir = Join-Path $PSScriptRoot "..\installer"

if (-not (Test-Path $assetsDir)) {
    New-Item -ItemType Directory -Path $assetsDir | Out-Null
}
if (-not (Test-Path $installerDir)) {
    New-Item -ItemType Directory -Path $installerDir | Out-Null
}

$srcImage = [System.Drawing.Image]::FromFile($SourcePath)

# 1. Save master PNG (1024x1024)
$png1024 = Join-Path $assetsDir "icon.png"
$srcImage.Save($png1024, [System.Drawing.Imaging.ImageFormat]::Png)
Write-Host "Saved: $png1024"

# Sizes for multi-resolution Windows ICO
$sizes = @(16, 24, 32, 48, 64, 128, 256)
$pngBytesList = @()

foreach ($sz in $sizes) {
    $bmp = New-Object System.Drawing.Bitmap $sz, $sz
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
    
    $g.DrawImage($srcImage, 0, 0, $sz, $sz)
    $g.Dispose()

    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()

    $pngBytesList += ,@($sz, $ms.ToArray())
    $ms.Dispose()
}

$srcImage.Dispose()

# 2. Build multi-resolution ICO file
# ICO header (6 bytes): Reserved (2), Type (2=1 for ICO), Count (2)
# Directory entries (16 bytes each): Width (1), Height (1), ColorCount (1), Reserved (1), Planes (2), BitCount (2), BytesInRes (4), ImageOffset (4)
# Followed by image data

function Write-IcoFile([string]$outputPath, $entries) {
    $fs = [System.IO.File]::Create($outputPath)
    $bw = New-Object System.IO.BinaryWriter($fs)

    # ICONDIR
    $bw.Write([uint16]0) # idReserved
    $bw.Write([uint16]1) # idType (1 = icon)
    $bw.Write([uint16]$entries.Count) # idCount

    $offset = 6 + ($entries.Count * 16)

    # ICONDIRENTRY list
    foreach ($entry in $entries) {
        $sz = $entry[0]
        $bytes = $entry[1]
        
        $w = if ($sz -ge 256) { 0 } else { [byte]$sz }
        $h = if ($sz -ge 256) { 0 } else { [byte]$sz }

        $bw.Write([byte]$w)
        $bw.Write([byte]$h)
        $bw.Write([byte]0)   # bColorCount
        $bw.Write([byte]0)   # bReserved
        $bw.Write([uint16]1) # wPlanes
        $bw.Write([uint16]32) # wBitCount
        $bw.Write([uint32]$bytes.Length) # dwBytesInRes
        $bw.Write([uint32]$offset)       # dwImageOffset

        $offset += $bytes.Length
    }

    # Image data
    foreach ($entry in $entries) {
        $bytes = $entry[1]
        $bw.Write($bytes)
    }

    $bw.Flush()
    $bw.Close()
    $fs.Close()
    Write-Host "Created ICO: $outputPath"
}

$assetsIco = Join-Path $assetsDir "icon.ico"
Write-IcoFile $assetsIco $pngBytesList

$installerIco = Join-Path $installerDir "app.ico"
Copy-Item $assetsIco $installerIco -Force
Write-Host "Copied to: $installerIco"
