param([string]$Destination = "$PSScriptRoot/../.local-archive/dependencies/pdfium-7881")
$ErrorActionPreference = 'Stop'
$url = 'https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F7881/pdfium-win-x64.tgz'
$archiveHash = '73CC0DE638AC2095E7445BF56A38200A5B7C7CA0E9F4BA144598F2457377AC08'
$dllHash = '79D4676B656CFB1ABCEA88F9ADE3B4B0826C5200382DB5F4EC72A636C598C118'
$target = [IO.Path]::GetFullPath($Destination)
New-Item -ItemType Directory -Force -Path $target | Out-Null
$archive = Join-Path $target 'pdfium-win-x64.tgz'
if (-not (Test-Path -LiteralPath $archive)) { Invoke-WebRequest -Uri $url -OutFile $archive }
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $archiveHash) { throw 'PDFium archive checksum mismatch' }
& tar -xzf $archive -C $target
if ($LASTEXITCODE -ne 0) { throw 'PDFium extraction failed' }
$library = Join-Path $target 'bin/pdfium.dll'
if ((Get-FileHash -LiteralPath $library -Algorithm SHA256).Hash -ne $dllHash) { throw 'PDFium library checksum mismatch' }
if (-not (Test-Path -LiteralPath (Join-Path $target 'licenses/pdfium.txt'))) { throw 'PDFium license missing' }
Write-Output $library
