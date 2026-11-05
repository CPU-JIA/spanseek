param(
    [string]$SourceRoot = '',
    [string]$LanguageSource = '',
    [string]$Destination = ''
)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$vendor = Join-Path $root 'third-party\tesseract'
$manifestPath = Join-Path $vendor 'runtime-manifest.json'
if (-not (Test-Path -LiteralPath $manifestPath)) { throw "Tesseract manifest missing: $manifestPath" }
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
if (-not $SourceRoot) {
    $SourceRoot = Join-Path $root ".local-archive/dependencies/tesseract-source-$($manifest.version)"
    $installedOcr = Join-Path $env:ProgramFiles 'Tesseract-OCR/tesseract.exe'
    $useInstalledOcr = $false
    if (Test-Path -LiteralPath $installedOcr) {
        $enginePin = $manifest.runtime | Where-Object { $_.path -eq 'tesseract.exe' }
        if ((Get-FileHash -LiteralPath $installedOcr -Algorithm SHA256).Hash -eq $enginePin.sha256) {
            $SourceRoot = Split-Path -Parent $installedOcr
            $useInstalledOcr = $true
        }
    }
    if (-not $useInstalledOcr) {
        & (Join-Path $PSScriptRoot 'fetch-tesseract.ps1') -Destination $SourceRoot
    }
}
if (-not $Destination) { $Destination = Join-Path $root ".local-archive/dependencies/tesseract-$($manifest.version)" }
$target = [IO.Path]::GetFullPath($Destination)
$source = [IO.Path]::GetFullPath($SourceRoot)
$languageRoot = if ($LanguageSource) {
    [IO.Path]::GetFullPath($LanguageSource)
} else {
    Join-Path $root '.local-archive/dependencies/tesseract/tessdata'
}
if (-not (Test-Path -LiteralPath (Join-Path $source 'tesseract.exe'))) {
    throw "Tesseract 5.4.0 source tree missing: $source. Install the pinned UB Mannheim package or pass -SourceRoot."
}
if (-not $LanguageSource) {
    $missingLanguages = @($manifest.tessdata.files | Where-Object {
        -not (Test-Path -LiteralPath (Join-Path $source ('tessdata/' + $_.path))) -and
        -not (Test-Path -LiteralPath (Join-Path $languageRoot $_.path))
    })
    if ($missingLanguages.Count -gt 0) {
        & (Join-Path $PSScriptRoot 'fetch-tesseract.ps1') -LanguagesOnly -LanguageDestination $languageRoot
    }
}
New-Item -ItemType Directory -Path $target -Force | Out-Null
$expected = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
$copied = [Collections.Generic.List[object]]::new()
function Copy-PinnedFile([string]$from, [string]$relative, [long]$bytes, [string]$sha256) {
    if (-not (Test-Path -LiteralPath $from)) { throw "Missing bundled file: $from" }
    $item = Get-Item -LiteralPath $from
    if ($item.Length -ne $bytes) { throw "Size mismatch for $from" }
    $actual = (Get-FileHash -LiteralPath $from -Algorithm SHA256).Hash
    if ($actual -ne $sha256) { throw "SHA-256 mismatch for $from" }
    $to = Join-Path $target $relative
    New-Item -ItemType Directory -Path (Split-Path -Parent $to) -Force | Out-Null
    Copy-Item -LiteralPath $from -Destination $to -Force
    if ((Get-FileHash -LiteralPath $to -Algorithm SHA256).Hash -ne $sha256) {
        throw "Copied file failed verification: $to"
    }
    $null = $expected.Add($relative)
    $copied.Add([ordered]@{ path = $relative; bytes = $bytes; sha256 = $sha256 })
}
foreach ($file in $manifest.runtime) {
    Copy-PinnedFile (Join-Path $source $file.path) $file.path $file.bytes $file.sha256
}
foreach ($file in $manifest.tessdata.files) {
    $candidates = @(
        (Join-Path $source ('tessdata/' + $file.path)),
        (Join-Path $languageRoot $file.path)
    )
    $from = $candidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if (-not $from) { throw "Tesseract language data missing: $($file.path)" }
    Copy-PinnedFile $from ('tessdata/' + $file.path) $file.bytes $file.sha256
}
foreach ($file in $manifest.docs) {
    Copy-PinnedFile (Join-Path $vendor $file.path) ('licenses/tesseract/' + $file.path) $file.bytes $file.sha256
}
Copy-PinnedFile $manifestPath 'manifest.json' (Get-Item -LiteralPath $manifestPath).Length (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash
[void]$expected.Add('bundle-validation.json')
# Refuse to bundle stale or unexpected files from an older attempt.
$actual = Get-ChildItem -LiteralPath $target -File -Recurse | ForEach-Object {
    [IO.Path]::GetRelativePath($target, $_.FullName).Replace('\', '/')
}
$unexpected = @($actual | Where-Object { -not $expected.Contains($_) })
if ($unexpected.Count -gt 0) { throw "Unexpected files in Tesseract bundle: $($unexpected -join ', ')" }
$engineVersion = (& (Join-Path $target 'tesseract.exe') --version 2>&1 | Select-Object -First 1)
$languages = (& (Join-Path $target 'tesseract.exe') --list-langs --tessdata-dir (Join-Path $target 'tessdata') 2>&1) -join "`n"
if ([string]$engineVersion -notmatch '^tesseract v5\.4\.0\.20240606$' -or $languages -notmatch '(?m)^chi_sim$' -or $languages -notmatch '(?m)^eng$') {
    throw "Bundled Tesseract failed its isolated smoke test: $engineVersion`n$languages"
}
$summary = [ordered]@{
    name = $manifest.name
    version = $manifest.version
    engine = $engineVersion
    destination = $target
    files = $copied
    languages = @('chi_sim', 'eng')
    isolatedSmokeTest = $true
}
$summary | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $target 'bundle-validation.json') -Encoding utf8NoBOM
Write-Output ("Tesseract bundle ready: {0} files, {1}, chi_sim+eng" -f $copied.Count, $engineVersion)
