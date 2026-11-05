param(
    [string]$Destination = '',
    [string]$LanguageDestination = '',
    [string]$SevenZipPath = '',
    [string]$DownloadDirectory = '',
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
    $missingRuntime = @($fetchManifest.runtime | Where-Object { -not (Test-Path -LiteralPath (Join-Path $fetchTarget $_.path)) })
    if ($missingRuntime.Count -gt 0) {
        if (-not $DownloadDirectory) { $DownloadDirectory = Join-Path $fetchRoot '.local-archive/dependencies/downloads' }
        $fetchArchive = Join-Path $DownloadDirectory "tesseract-$($fetchManifest.version).exe"
        Get-PinnedDownload $fetchManifest.installer.url $fetchArchive $fetchManifest.installer.sha256
        if (-not $SevenZipPath) {
            $zipCommand = Get-Command 7z -ErrorAction SilentlyContinue
            if ($zipCommand) { $SevenZipPath = $zipCommand.Source }
            else { $SevenZipPath = Join-Path $env:ProgramFiles '7-Zip/7z.exe' }
        }
        if (-not (Test-Path -LiteralPath $SevenZipPath)) {
            throw '7-Zip is required to extract the pinned OCR runtime without installing it. Install 7-Zip or pass -SevenZipPath. GitHub windows-2025 includes it.'
        }
        New-Item -ItemType Directory -Path $fetchTarget -Force | Out-Null
        # Extract only manifest-listed runtime files. Never execute the installer.
        $runtimeNames = @($fetchManifest.runtime | ForEach-Object { [string]$_.path })
        & $SevenZipPath e -y -r "-o$fetchTarget" $fetchArchive @runtimeNames | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'Pinned Tesseract extraction failed' }
    }
    foreach ($runtimeFile in $fetchManifest.runtime) {
        $runtimePath = Join-Path $fetchTarget $runtimeFile.path
        if (-not (Test-Path -LiteralPath $runtimePath) -or
            (Get-Item -LiteralPath $runtimePath).Length -ne $runtimeFile.bytes -or
            (Get-FileHash -LiteralPath $runtimePath -Algorithm SHA256).Hash -ne $runtimeFile.sha256) {
            throw "Extracted OCR runtime mismatch: $($runtimeFile.path)"
        }
    }
}
foreach ($languageFile in $fetchManifest.tessdata.files) {
    $languageUrl = "https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/$($fetchManifest.tessdata.commit)/$($languageFile.path)"
    Get-PinnedDownload $languageUrl (Join-Path $fetchLanguages $languageFile.path) $languageFile.sha256
}
Write-Output "Pinned OCR resources verified: $($fetchManifest.version), chi_sim + eng"
