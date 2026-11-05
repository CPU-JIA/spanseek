# The SVG is the original artwork; Tauri's installed CLI renders every platform asset.
# Run this script after editing it. No app build, bundling, or installation is performed.
$ErrorActionPreference = 'Stop'
$iconRepoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$iconAppRoot = Join-Path $iconRepoRoot 'apps/desktop'
$iconOutput = Join-Path $iconAppRoot 'src-tauri/icons'
$iconSource = Join-Path $iconAppRoot 'src-tauri/app-icon.svg'

if (-not (Test-Path -LiteralPath $iconSource -PathType Leaf)) {
    throw "Icon source not found: $iconSource"
}

& pnpm --dir $iconAppRoot exec tauri icon $iconSource --output $iconOutput
if ($LASTEXITCODE -ne 0) { throw 'Tauri platform icon generation failed' }

# Keep the smallest Windows sizes available as PNGs for direct visual inspection.
# The regular generation above also embeds 16/24/32/48/64/256 px frames in icon.ico.
& pnpm --dir $iconAppRoot exec tauri icon $iconSource --output $iconOutput --png 16 --png 24
if ($LASTEXITCODE -ne 0) { throw 'Small icon preview generation failed' }

# The browser favicon shares the same artwork as the installed application.
Copy-Item -LiteralPath $iconSource -Destination (Join-Path $iconAppRoot 'public/icon.svg') -Force
