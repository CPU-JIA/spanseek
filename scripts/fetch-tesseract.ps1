param(
    [string]$Destination = '',
    [string]$LanguageDestination = '',
    [string]$BuildRoot = '',
    [switch]$LanguagesOnly
)
$ErrorActionPreference = 'Stop'
$fetchRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$fetchManifest = Get-Content -LiteralPath (Join-Path $fetchRoot 'third-party/tesseract/runtime-manifest.json') -Raw | ConvertFrom-Json
if (-not $Destination) { $Destination = Join-Path $fetchRoot ".local-archive/dependencies/tesseract-source-$($fetchManifest.version)" }
if (-not $LanguageDestination) { $LanguageDestination = Join-Path $Destination 'tessdata' }
$fetchTarget = [IO.Path]::GetFullPath($Destination)
$fetchLanguages = [IO.Path]::GetFullPath($LanguageDestination)

function Get-PinnedDownload([string]$Url, [string]$File, [string]$ExpectedHash) {
    if (-not (Test-Path -LiteralPath $File)) {
        New-Item -ItemType Directory -Path (Split-Path -Parent $File) -Force | Out-Null
        $downloadPending = "$File.download"
        Invoke-WebRequest -Uri $Url -OutFile $downloadPending
        if ((Get-FileHash -LiteralPath $downloadPending -Algorithm SHA256).Hash -ne $ExpectedHash) {
            throw "Downloaded file checksum mismatch: $File"
        }
        Move-Item -LiteralPath $downloadPending -Destination $File
    }
    if ((Get-FileHash -LiteralPath $File -Algorithm SHA256).Hash -ne $ExpectedHash) {
        throw "Cached file checksum mismatch: $File"
    }
}

if (-not $LanguagesOnly) {
    & (Join-Path $PSScriptRoot 'build-tesseract.ps1') -BuildRoot $BuildRoot -Destination $fetchTarget
}
foreach ($languageFile in $fetchManifest.tessdata.files) {
    $languageUrl = "https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/$($fetchManifest.tessdata.commit)/$($languageFile.path)"
    Get-PinnedDownload $languageUrl (Join-Path $fetchLanguages $languageFile.path) $languageFile.sha256
}
Write-Output "Pinned OCR resources verified: $($fetchManifest.version), chi_sim + eng"
