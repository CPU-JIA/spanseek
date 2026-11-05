param([string]$Output = '')
$ErrorActionPreference = 'Stop'
$releaseRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $Output) { $Output = Join-Path $releaseRoot '.local-archive/release' }
$releaseOutput = [IO.Path]::GetFullPath($Output)
$releaseVersion = (& node (Join-Path $PSScriptRoot 'check-version.mjs')).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Version validation failed' }
Push-Location -LiteralPath $releaseRoot
try {
    $metadata = cargo metadata --no-deps --locked --format-version 1 | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed' }
    $installerName = "spanseek_${releaseVersion}_x64-setup.exe"
    $installerSource = Join-Path $metadata.target_directory "release/bundle/nsis/$installerName"
    if (-not (Test-Path -LiteralPath $installerSource)) { throw "Installer missing: $installerName" }
    New-Item -ItemType Directory -Path $releaseOutput -Force | Out-Null
    & node (Join-Path $PSScriptRoot 'build-notices.mjs')
    if ($LASTEXITCODE -ne 0) { throw 'Third-party notice generation failed' }
    & python (Join-Path $PSScriptRoot 'release-inventory.py') --target-dir $metadata.target_directory --installer-name $installerName --output $releaseOutput
    if ($LASTEXITCODE -ne 0) { throw 'Release inventory failed' }
    Copy-Item -LiteralPath $installerSource -Destination (Join-Path $releaseOutput $installerName) -Force
    Copy-Item -LiteralPath (Join-Path $releaseRoot 'THIRD-PARTY-NOTICES.md') -Destination $releaseOutput -Force
    Copy-Item -LiteralPath (Join-Path $releaseRoot 'LICENSE') -Destination $releaseOutput -Force
    Copy-Item -LiteralPath (Join-Path $releaseRoot 'CHANGELOG.md') -Destination (Join-Path $releaseOutput 'RELEASE-NOTES.md') -Force
    $signature = (Get-AuthenticodeSignature -LiteralPath $installerSource).Status.ToString()
    $revision = git rev-parse HEAD
    if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve build commit' }
    [ordered]@{
        version = $releaseVersion
        commit = $revision
        installer = $installerName
        bytes = (Get-Item -LiteralPath $installerSource).Length
        sha256 = (Get-FileHash -LiteralPath $installerSource -Algorithm SHA256).Hash
        signature = $signature
        built_at = (Get-Date).ToUniversalTime().ToString('o')
        platform = 'windows-x64'
        release_state = 'draft'
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $releaseOutput 'BUILD-INFO.json') -Encoding utf8NoBOM
    $sumLines = Get-ChildItem -LiteralPath $releaseOutput -File | Where-Object { $_.Name -ne 'SHA256SUMS.txt' } | Sort-Object Name | ForEach-Object {
        '{0}  {1}' -f (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant(), $_.Name
    }
    $sumLines | Set-Content -LiteralPath (Join-Path $releaseOutput 'SHA256SUMS.txt') -Encoding utf8NoBOM
    Write-Output "Release artifacts prepared: $installerName ($signature)"
} finally { Pop-Location }
