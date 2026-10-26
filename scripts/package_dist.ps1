# Open Tangerine for Windows — Packaging Script
#
# Packages Open Tangerine into dist/OpenTangerine and creates dist/OpenTangerine.zip.
# If Inno Setup (ISCC) is present on the system, compiles the installer into dist/.

[CmdletBinding()]
param(
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RootDir = Split-Path -Parent $ScriptDir

Write-Host "=== Packaging Open Tangerine for Windows ===" -ForegroundColor Cyan
Write-Host "Project root: $RootDir"

if (-not $SkipBuild) {
    Write-Host "Building release binary..." -ForegroundColor Yellow
    Push-Location $RootDir
    try {
        & cargo build --release
        if ($LASTEXITCODE -ne 0) {
            throw "Cargo release build failed with exit code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
    }
}

$ExePath = Join-Path $RootDir "target\release\open-tangerine.exe"
if (-not (Test-Path $ExePath)) {
    throw "Release binary not found at $ExePath. Run without -SkipBuild."
}

$DistDir = Join-Path $RootDir "dist\OpenTangerine"
if (-not (Test-Path $DistDir)) {
    New-Item -ItemType Directory -Path $DistDir -Force | Out-Null
}

Write-Host "Copying binary to $DistDir..." -ForegroundColor Yellow
Copy-Item $ExePath -Destination (Join-Path $DistDir "open-tangerine.exe") -Force

# Copy PDFium DLL if present in vendor/
$PdfiumVendor = Join-Path $RootDir "vendor\pdfium\pdfium.dll"
if (Test-Path $PdfiumVendor) {
    Write-Host "Bundling PDFium engine..." -ForegroundColor Yellow
    Copy-Item $PdfiumVendor -Destination (Join-Path $DistDir "pdfium.dll") -Force
}

# Verify FFmpeg binaries
$FfmpegDir = Join-Path $DistDir "ffmpeg"
if (Test-Path (Join-Path $FfmpegDir "ffmpeg.exe")) {
    Write-Host "FFmpeg binaries present in $FfmpegDir" -ForegroundColor Green
} else {
    Write-Host "Warning: ffmpeg.exe not found in $FfmpegDir. Conversions requiring FFmpeg will rely on PATH." -ForegroundColor DarkYellow
}

# Ensure license files are present
$LicensesDir = Join-Path $DistDir "licenses"
if (-not (Test-Path $LicensesDir)) {
    New-Item -ItemType Directory -Path $LicensesDir -Force | Out-Null
}

$MitLicense = Join-Path $RootDir "LICENSE-MIT"
if (Test-Path $MitLicense) {
    Copy-Item $MitLicense -Destination (Join-Path $LicensesDir "LICENSE-MIT.txt") -Force
}

$ApacheLicense = Join-Path $RootDir "LICENSE-APACHE"
if (Test-Path $ApacheLicense) {
    Copy-Item $ApacheLicense -Destination (Join-Path $LicensesDir "LICENSE-APACHE.txt") -Force
}

# Create distribution ZIP archive
$DistZip = Join-Path $RootDir "dist\OpenTangerine.zip"
Write-Host "Creating zip package at $DistZip..." -ForegroundColor Yellow
if (Test-Path $DistZip) {
    Remove-Item $DistZip -Force
}
Compress-Archive -Path "$DistDir\*" -DestinationPath $DistZip -CompressionLevel Optimal

$ZipSize = (Get-Item $DistZip).Length / 1MB
Write-Host ("Zip package created successfully: {0:N2} MB" -f $ZipSize) -ForegroundColor Green

# Check for Inno Setup compiler to build the installer if available
$IsccCandidates = @(
    "iscc",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles}\Inno Setup 6\ISCC.exe"
)

$IsccPath = $null
foreach ($cand in $IsccCandidates) {
    if (Get-Command $cand -ErrorAction SilentlyContinue) {
        $IsccPath = $cand
        break
    } elseif (Test-Path $cand) {
        $IsccPath = $cand
        break
    }
}

if ($IsccPath) {
    Write-Host "Compiling installer using $IsccPath..." -ForegroundColor Yellow
    $IssScript = Join-Path $RootDir "installer\open-tangerine.iss"
    & $IsccPath $IssScript
    if ($LASTEXITCODE -eq 0) {
        Write-Host "Installer compiled successfully in dist\" -ForegroundColor Green
    } else {
        Write-Host "Warning: Installer compilation failed with exit code $LASTEXITCODE" -ForegroundColor Red
    }
} else {
    Write-Host "Note: Inno Setup compiler (ISCC) not found on PATH. Installer script is ready at installer\open-tangerine.iss." -ForegroundColor Gray
}

Write-Host "Packaging complete." -ForegroundColor Green
