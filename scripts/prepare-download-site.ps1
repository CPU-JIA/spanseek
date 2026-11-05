param(
    [Parameter(Mandatory)][string]$ReleaseDirectory,
    [Parameter(Mandatory)][string]$Output
)
$ErrorActionPreference = 'Stop'
$siteRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$releaseDirectoryPath = (Resolve-Path -LiteralPath $ReleaseDirectory).Path
$siteOutput = [IO.Path]::GetFullPath($Output)
if (Test-Path -LiteralPath $siteOutput) { throw 'Use a new output directory; existing sites are never overwritten' }
$info = Get-Content -LiteralPath (Join-Path $releaseDirectoryPath 'BUILD-INFO.json') -Raw | ConvertFrom-Json
$version = (& node (Join-Path $PSScriptRoot 'check-version.mjs')).Trim()
if ($LASTEXITCODE -ne 0 -or $info.version -ne $version) { throw 'Source and installer versions differ' }
if ($info.installer -ne "spanseek_${version}_x64-setup.exe" -or $info.sha256 -notmatch '^[0-9a-fA-F]{64}$') {
    throw 'Invalid installer metadata'
}
$installer = Join-Path $releaseDirectoryPath $info.installer
if ((Get-Item -LiteralPath $installer).Length -ne $info.bytes -or
    (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash -ne $info.sha256) {
    throw 'Installer does not match BUILD-INFO.json'
}
$releaseFiles = @('SHA256SUMS.txt')
foreach ($line in Get-Content -LiteralPath (Join-Path $releaseDirectoryPath 'SHA256SUMS.txt')) {
    if ($line -notmatch '^([0-9a-fA-F]{64})  ([A-Za-z0-9][A-Za-z0-9._-]*)$') { throw 'Invalid checksum entry' }
    $expected = $Matches[1]
    $name = $Matches[2]
    if ($name -eq 'SHA256SUMS.txt' -or $releaseFiles -contains $name) { throw 'Duplicate checksum entry' }
    if ((Get-FileHash -LiteralPath (Join-Path $releaseDirectoryPath $name) -Algorithm SHA256).Hash -ne $expected) {
        throw "Release checksum mismatch: $name"
    }
    $releaseFiles += $name
}
if ($releaseFiles -notcontains 'BUILD-INFO.json' -or $releaseFiles -notcontains $info.installer) {
    throw 'Checksum list must cover the metadata and installer'
}
$buildDate = ([DateTimeOffset]$info.built_at).UtcDateTime.ToString('yyyy-MM-dd')
Push-Location -LiteralPath $siteRoot
try {
    # Copy only Git-tracked product docs. Local development records stay local.
    $docs = @(git -c core.quotepath=false ls-files -- docs/download docs/guide)
    if ($LASTEXITCODE -ne 0 -or $docs.Count -eq 0) { throw 'Cannot enumerate product documentation' }
    foreach ($file in $docs) {
        $destination = Join-Path $siteOutput $file
        New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
        Copy-Item -LiteralPath (Join-Path $siteRoot $file) -Destination $destination
    }
    $downloads = Join-Path $siteOutput 'docs/download/downloads'
    New-Item -ItemType Directory -Path $downloads -Force | Out-Null
    foreach ($file in $releaseFiles) {
        Copy-Item -LiteralPath (Join-Path $releaseDirectoryPath $file) -Destination $downloads
    }
    $installPage = Join-Path $siteOutput 'docs/download/install.html'
    $html = [IO.File]::ReadAllText($installPage)
    $values = @{
        '__INSTALLER_SIZE__' = ([double]$info.bytes / 1MB).ToString('F2', [Globalization.CultureInfo]::InvariantCulture) + ' MiB'
        '__INSTALLER_DATE__' = $buildDate
        '__INSTALLER_SHA256__' = $info.sha256.ToUpperInvariant()
    }
    foreach ($token in $values.Keys) {
        if ([regex]::Matches($html, [regex]::Escape($token)).Count -ne 1) { throw "Expected one site token: $token" }
        $html = $html.Replace($token, $values[$token])
    }
    [IO.File]::WriteAllText($installPage, $html, [Text.UTF8Encoding]::new($false))
    Write-Output "Prepared local site: $installPage"
    Write-Output "Installer: $($info.installer); SHA-256: $($info.sha256)"
    Write-Output 'No site or release has been published.'
} finally { Pop-Location }
