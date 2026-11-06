param(
    [Parameter(Mandatory)][string]$Directory,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-f]{40}$')][string]$ExpectedCommit,
    [Parameter(Mandatory)][string]$ExpectedVersion
)
$ErrorActionPreference = 'Stop'
$release = (Resolve-Path -LiteralPath $Directory).Path
$info = Get-Content -LiteralPath (Join-Path $release 'BUILD-INFO.json') -Raw | ConvertFrom-Json
if ($info.commit -cne $ExpectedCommit -or $info.version -cne $ExpectedVersion -or
    $info.installer -cne "spanseek_${ExpectedVersion}_x64-setup.exe") {
    throw 'Installer metadata does not match the tagged source'
}
$expectedFiles = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
[void]$expectedFiles.Add('SHA256SUMS.txt')
foreach ($line in Get-Content -LiteralPath (Join-Path $release 'SHA256SUMS.txt')) {
    if ($line -cnotmatch '^([0-9a-f]{64})  ([A-Za-z0-9][A-Za-z0-9._-]*)$') { throw 'Invalid release checksum entry' }
    $digest = $Matches[1]
    $name = $Matches[2]
    if (-not $expectedFiles.Add($name)) { throw 'Duplicate checksum entry' }
    $path = Join-Path $release $name
    if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $digest) {
        throw "Release checksum mismatch: $name"
    }
}
foreach ($required in @($info.installer, 'BUILD-INFO.json', 'build-manifest.json', 'source-files.json',
    'LICENSE', 'RELEASE-NOTES.md', 'THIRD-PARTY-NOTICES.md', 'THIRD-PARTY-LICENSES.txt',
    'THIRD-PARTY-COMPONENTS.json', 'THIRD-PARTY-SOURCES.zip', 'THIRD-PARTY-SOURCE-INFO.txt')) {
    if (-not $expectedFiles.Contains($required)) { throw "Required release file missing: $required" }
}
$unexpected = @(Get-ChildItem -LiteralPath $release -Force | Where-Object {
    $_.PSIsContainer -or -not $expectedFiles.Contains($_.Name)
})
if ($unexpected.Count) { throw 'Unexpected files in the release directory' }
$installer = Join-Path $release $info.installer
if ((Get-Item -LiteralPath $installer).Length -ne $info.bytes -or
    (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash -ne $info.sha256 -or
    (Get-AuthenticodeSignature -LiteralPath $installer).Status.ToString() -cne $info.signature) {
    throw 'Installer size, hash or signature status differs from build metadata'
}
$manifest = Get-Content -LiteralPath (Join-Path $release 'build-manifest.json') -Raw | ConvertFrom-Json
if ($manifest.git_revision -cne $ExpectedCommit -or
    (Get-FileHash -LiteralPath (Join-Path $release 'source-files.json') -Algorithm SHA256).Hash -ne $manifest.source_inventory_sha256) {
    throw 'Source inventory does not match the checked build'
}
$licenses = Get-Content -LiteralPath (Join-Path $release 'THIRD-PARTY-COMPONENTS.json') -Raw | ConvertFrom-Json
if (@($licenses.missing).Count -ne 0) { throw 'Release has missing license material' }
Write-Output "Verified $($expectedFiles.Count) release files for $ExpectedVersion ($ExpectedCommit), signature $($info.signature)"
