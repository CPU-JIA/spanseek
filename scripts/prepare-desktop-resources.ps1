param([switch]$Release, [switch]$Dev)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$metadata = cargo metadata --no-deps --format-version 1 --locked | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or -not $metadata.target_directory) { throw 'Cargo target directory could not be resolved' }
$resourceTargetRoot = [IO.Path]::GetFullPath([string]$metadata.target_directory)
Push-Location -LiteralPath $root
try {
    & node (Join-Path $PSScriptRoot 'build-rich-export.mjs')
    if ($LASTEXITCODE -ne 0) { throw 'Offline rich-format export resource build failed' }
    & pwsh -NoProfile -File (Join-Path $PSScriptRoot 'fetch-pdfium.ps1')
    if ($LASTEXITCODE -ne 0) { throw 'PDFium resource preparation failed' }
    & pwsh -NoProfile -File (Join-Path $PSScriptRoot 'prepare-tesseract.ps1')
    if ($LASTEXITCODE -ne 0) { throw 'Tesseract resource preparation failed' }
    $profile = if ($Release) { 'release' } else { 'debug' }
    if ($Release) { & cargo build -p pkms-worker --release --locked }
    else { & cargo build -p pkms-worker --locked }
    if ($LASTEXITCODE -ne 0) { throw 'Source worker build failed' }
    & pwsh -NoProfile -File (Join-Path $PSScriptRoot 'fetch-protoc.ps1')
    if ($LASTEXITCODE -ne 0) { throw 'Protocol compiler preparation failed' }
    $env:PROTOC = Join-Path $root '.local-archive/tooling/protoc-36.2/bin/protoc.exe'
    & python (Join-Path $PSScriptRoot 'fetch-onnxruntime.py') --repair-cargo-cache
    if ($LASTEXITCODE -ne 0) { throw 'ONNX native dependency preparation failed' }
    if ($Release) { & cargo build -p pkms-inference --features runtime --bin pkms-inference --release --locked }
    else { & cargo build -p pkms-inference --features runtime --bin pkms-inference --locked }
    if ($LASTEXITCODE -ne 0) { throw 'Inference worker build failed' }
    & python (Join-Path $PSScriptRoot 'build-license-bundle.py')
    if ($LASTEXITCODE -ne 0) { throw 'Complete third-party license/source bundle failed' }
    & (Join-Path $PSScriptRoot 'sync-native-binaries.ps1') -Profile $profile -Names @('pkms-worker','pkms-inference')
    $triple = ((& rustc -vV) | Select-String '^host: ').ToString().Substring(6)
    if ($triple -ne 'x86_64-pc-windows-msvc') { throw 'Only Windows x64 resources have been validated' }
    $bundle = Join-Path $root '.local-archive/bundle'
    New-Item -ItemType Directory -Path $bundle -Force | Out-Null
    # Shipped sidecar names follow the product name; cargo keeps its own crate
    # names. Remove a pre-rename leftover so nothing bundles the old name.
    Copy-Item -LiteralPath (Join-Path $resourceTargetRoot "$profile/pkms-worker.exe") -Destination (Join-Path $bundle "spanseek-worker-$triple.exe") -Force
    Copy-Item -LiteralPath (Join-Path $resourceTargetRoot "$profile/pkms-inference.exe") -Destination (Join-Path $bundle "spanseek-inference-$triple.exe") -Force
    foreach ($legacy in @("pkms-worker-$triple.exe", "pkms-inference-$triple.exe")) {
        $legacyPath = Join-Path $bundle $legacy
        if (Test-Path -LiteralPath $legacyPath) { Remove-Item -LiteralPath $legacyPath -Force }
    }
    if ($Dev) {
        & pnpm --dir (Join-Path $root 'apps/desktop') dev
        if ($LASTEXITCODE -ne 0) { throw 'Frontend development server failed' }
    }
} finally { Pop-Location }
