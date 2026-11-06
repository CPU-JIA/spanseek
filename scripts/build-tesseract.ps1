param([string]$BuildRoot = '', [string]$Destination = '')
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$recipe = Join-Path $root 'third-party/tesseract/build'
$manifest = Get-Content -LiteralPath (Join-Path $recipe 'vcpkg.json') -Raw | ConvertFrom-Json
$baseline = $manifest.'builtin-baseline'
$version = $manifest.'version-string'
$triplet = 'x64-windows-static-release'
if (-not $BuildRoot) { $BuildRoot = $env:SPANSEEK_NATIVE_BUILD_ROOT }
if (-not $BuildRoot) { $BuildRoot = Join-Path $root '.local-archive/native-ocr' }
if (-not $Destination) { $Destination = Join-Path $root ".local-archive/dependencies/tesseract-source-$version" }
$build = [IO.Path]::GetFullPath($BuildRoot)
$target = [IO.Path]::GetFullPath($Destination)
$fingerprintFiles = @(
    'scripts/build-tesseract.ps1', 'scripts/check-ocr-pe.py',
    'third-party/tesseract/build/vcpkg.json',
    'third-party/tesseract/build/utf8.manifest',
    'third-party/tesseract/build/triplets/x64-windows-static-release.cmake'
)
$fingerprint = ($fingerprintFiles | ForEach-Object {
    '{0}:{1}' -f $_, (Get-FileHash -LiteralPath (Join-Path $root $_) -Algorithm SHA256).Hash
}) -join "`n"
$receiptPath = Join-Path $target 'build-receipt.json'
if (Test-Path -LiteralPath $receiptPath) {
    $cached = Get-Content -LiteralPath $receiptPath -Raw | ConvertFrom-Json
    if ($cached.recipe -eq $fingerprint -and $cached.baseline -eq $baseline -and
        $cached.version -eq $version -and @($cached.files).Count -gt 1 -and
        @($cached.files | Where-Object path -eq 'tesseract.exe').Count -eq 1) {
        foreach ($entry in $cached.files) {
            $file = Join-Path $target $entry.path
            if (-not (Test-Path -LiteralPath $file) -or
                (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash -ne $entry.sha256) {
                throw "Cached OCR resource mismatch: $($entry.path)"
            }
        }
        & python (Join-Path $PSScriptRoot 'check-ocr-pe.py') (Join-Path $target 'tesseract.exe')
        if ($LASTEXITCODE -ne 0) { throw 'Cached OCR PE dependency verification failed' }
        Write-Output "Verified cached OCR source build: $version"
        return
    }
}
New-Item -ItemType Directory -Path $build -Force | Out-Null
$vcpkg = Join-Path $build 'vcpkg'
if (-not (Test-Path -LiteralPath (Join-Path $vcpkg '.git'))) {
    & git clone --filter=blob:none --no-checkout https://github.com/microsoft/vcpkg.git $vcpkg
    if ($LASTEXITCODE -ne 0) { throw 'vcpkg checkout failed' }
}
& git -C $vcpkg checkout --detach $baseline
if ($LASTEXITCODE -ne 0) { throw 'Pinned vcpkg baseline unavailable' }
$changes = & git -C $vcpkg status --porcelain --untracked-files=no
if ($LASTEXITCODE -ne 0 -or $changes) { throw 'vcpkg tracked sources must match the pinned baseline' }
$env:VCPKG_DISABLE_METRICS = '1'
$env:VCPKG_DEFAULT_BINARY_CACHE = Join-Path $build 'binary-cache'
New-Item -ItemType Directory -Path $env:VCPKG_DEFAULT_BINARY_CACHE -Force | Out-Null
$env:VCPKG_DOWNLOADS = Join-Path $build 'downloads'
New-Item -ItemType Directory -Path $env:VCPKG_DOWNLOADS -Force | Out-Null
& (Join-Path $vcpkg 'bootstrap-vcpkg.bat') -disableMetrics
if ($LASTEXITCODE -ne 0) { throw 'vcpkg bootstrap failed' }
$installed = Join-Path $build 'installed'
& (Join-Path $vcpkg 'vcpkg.exe') install "--triplet=$triplet" "--x-manifest-root=$recipe" `
    "--overlay-triplets=$recipe/triplets" "--x-install-root=$installed" `
    "--x-buildtrees-root=$build/buildtrees" "--x-packages-root=$build/packages"
if ($LASTEXITCODE -ne 0) { throw 'Pinned OCR source build failed' }
$prefix = Join-Path $installed $triplet
$sourceExe = Join-Path $prefix 'tools/tesseract/tesseract.exe'
New-Item -ItemType Directory -Path $target -Force | Out-Null
$exe = Join-Path $target 'tesseract.exe'
Copy-Item -LiteralPath $sourceExe -Destination $exe -Force
# Narrow argv/fopen in Tesseract and Leptonica otherwise depend on the machine's
# ANSI code page. Merge a process-local UTF-8 manifest, preserving UAC settings.
$sdk = @(
    (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots' -ErrorAction SilentlyContinue).KitsRoot10,
    (Get-ItemProperty 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows Kits\Installed Roots' -ErrorAction SilentlyContinue).KitsRoot10
) | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1
if (-not $sdk) { throw 'Windows SDK is required to embed the OCR UTF-8 manifest' }
$mt = Get-ChildItem -LiteralPath (Join-Path $sdk 'bin') -Filter mt.exe -File -Recurse |
    Where-Object { $_.Directory.Name -eq 'x64' } | Sort-Object FullName -Descending | Select-Object -First 1
if (-not $mt) { throw 'Windows SDK x64 manifest tool is missing' }
& $mt.FullName -nologo "-inputresource:$exe;#1" -manifest (Join-Path $recipe 'utf8.manifest') "-outputresource:$exe;#1"
if ($LASTEXITCODE -ne 0) { throw 'OCR UTF-8 manifest merge failed' }
$pe = & python (Join-Path $PSScriptRoot 'check-ocr-pe.py') $exe
if ($LASTEXITCODE -ne 0) { throw 'OCR PE dependency verification failed' }
$engine = (& $exe --version 2>&1) -join "`n"
if ($LASTEXITCODE -ne 0 -or $engine -notmatch "(?m)^tesseract $([regex]::Escape($version))$") {
    throw 'Unexpected OCR engine version'
}
$components = @()
$expectedFiles = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
[void]$expectedFiles.Add('tesseract.exe')
$manifestCopy = Join-Path $target 'licenses/tesseract/utf8.manifest'
New-Item -ItemType Directory -Path (Split-Path -Parent $manifestCopy) -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $recipe 'utf8.manifest') -Destination $manifestCopy -Force
[void]$expectedFiles.Add('licenses/tesseract/utf8.manifest')
foreach ($block in ((Get-Content -LiteralPath (Join-Path $installed 'vcpkg/status') -Raw) -split '\r?\n\r?\n')) {
    $fields = @{}
    foreach ($line in ($block -split '\r?\n')) {
        if ($line -match '^([\w-]+): (.*)$') { $fields[$Matches[1]] = $Matches[2] }
    }
    if ($fields.Architecture -ne $triplet -or $fields.ContainsKey('Feature')) { continue }
    $name = $fields.Package
    $license = Join-Path $prefix "share/$name/copyright"
    if (-not (Test-Path -LiteralPath $license)) { throw "Missing native license: $name" }
    $licenseDir = Join-Path $target "licenses/$name"
    New-Item -ItemType Directory -Path $licenseDir -Force | Out-Null
    Copy-Item -LiteralPath $license -Destination (Join-Path $licenseDir 'copyright') -Force
    # Include the exact port recipe, patches and source archive hashes.
    $port = Join-Path $vcpkg "ports/$name"
    Copy-Item -LiteralPath $port -Destination $licenseDir -Recurse -Force
    [void]$expectedFiles.Add("licenses/$name/copyright")
    foreach ($portFile in (Get-ChildItem -LiteralPath $port -File -Recurse)) {
        [void]$expectedFiles.Add("licenses/$name/$name/" + [IO.Path]::GetRelativePath($port, $portFile.FullName).Replace('\', '/'))
    }
    $portManifest = Get-Content -LiteralPath (Join-Path $port 'vcpkg.json') -Raw | ConvertFrom-Json
    $components += [ordered]@{
        name = $name; version = $fields.Version; port_version = $fields.'Port-Version'
        license = $portManifest.license; abi = $fields.Abi
        recipe = "https://github.com/microsoft/vcpkg/tree/$baseline/ports/$name"
        license_file = "licenses/$name/copyright"
    }
}
$files = @($expectedFiles | Sort-Object | ForEach-Object {
    $path = Join-Path $target $_
    [ordered]@{ path = $_; bytes = (Get-Item -LiteralPath $path).Length
        sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash }
})
[ordered]@{
    version = $version; baseline = $baseline; triplet = $triplet; recipe = $fingerprint
    engine = $engine; pe = ($pe | ConvertFrom-Json); components = $components; files = $files
} | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $receiptPath -Encoding utf8NoBOM
Write-Output "Built and verified OCR $version with $($components.Count) dependency notices"
