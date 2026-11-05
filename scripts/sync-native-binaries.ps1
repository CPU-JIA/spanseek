param(
    [ValidateSet('debug','release')][string]$Profile = 'release',
    [string[]]$Names = @('spanseek','pkms-worker','pkms-inference')
)
$ErrorActionPreference = 'Stop'
$nativeRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$metadata = cargo metadata --no-deps --format-version 1 --locked | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or -not $metadata.target_directory) { throw 'Cargo target directory could not be resolved' }
$nativeTargetRoot = [IO.Path]::GetFullPath([string]$metadata.target_directory)
foreach ($nativeName in $Names) {
    if ($nativeName -notin @('spanseek','pkms-worker','pkms-inference')) { throw 'Unsupported native binary name' }
    $nativePublished = Join-Path $nativeTargetRoot "$Profile/$nativeName.exe"
    $nativeArtifact = Join-Path $nativeTargetRoot ("$Profile/deps/" + $nativeName.Replace('-','_') + '.exe')
    # On the verified Windows toolchain Cargo can leave an older hyphenated alias
    # beside its freshly linked underscored executable. Never bundle that stale alias.
    if (Test-Path -LiteralPath $nativeArtifact) {
        if (-not (Test-Path -LiteralPath $nativePublished) -or
            (Get-FileHash -LiteralPath $nativeArtifact -Algorithm SHA256).Hash -ne
            (Get-FileHash -LiteralPath $nativePublished -Algorithm SHA256).Hash) {
            Copy-Item -LiteralPath $nativeArtifact -Destination $nativePublished -Force
        }
    }
    if (-not (Test-Path -LiteralPath $nativePublished)) { throw "Native binary unavailable: $nativeName" }
    [pscustomobject]@{ Name=$nativeName; SHA256=(Get-FileHash -LiteralPath $nativePublished -Algorithm SHA256).Hash }
}
