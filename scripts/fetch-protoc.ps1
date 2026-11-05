param([string]$Destination = "$PSScriptRoot/../.local-archive/tooling/protoc-36.2")
$ErrorActionPreference = 'Stop'
$protocVersion = '36.2'
$protocUrl = 'https://github.com/protocolbuffers/protobuf/releases/download/v36.2/protoc-36.2-win64.zip'
$protocArchiveHash = 'F0C128DC0D8492ECEECE83BB459A4C0E316764B929FFBF1AA416357FD644EDD3'
$protocExecutableHash = '5A1B5350308309C9729CE4484A798981F281C10909144C4BDEFD6A37688E4B1F'
$protocTarget = [IO.Path]::GetFullPath($Destination)
$protocExecutable = Join-Path $protocTarget 'bin/protoc.exe'
if (Test-Path -LiteralPath $protocExecutable) {
    if ((Get-FileHash -LiteralPath $protocExecutable -Algorithm SHA256).Hash -ne $protocExecutableHash) {
        throw 'Cached protoc executable checksum mismatch'
    }
    Write-Output $protocExecutable
    return
}
New-Item -ItemType Directory -Force -Path $protocTarget | Out-Null
$protocArchive = Join-Path $protocTarget "protoc-$protocVersion-win64.zip"
if (-not (Test-Path -LiteralPath $protocArchive)) {
    $protocPending = "$protocArchive.download"
    Invoke-WebRequest -Uri $protocUrl -OutFile $protocPending
    if ((Get-FileHash -LiteralPath $protocPending -Algorithm SHA256).Hash -ne $protocArchiveHash) {
        throw 'Downloaded protoc archive checksum mismatch'
    }
    Move-Item -LiteralPath $protocPending -Destination $protocArchive
}
if ((Get-FileHash -LiteralPath $protocArchive -Algorithm SHA256).Hash -ne $protocArchiveHash) {
    throw 'Cached protoc archive checksum mismatch'
}
Expand-Archive -LiteralPath $protocArchive -DestinationPath $protocTarget -Force
if ((Get-FileHash -LiteralPath $protocExecutable -Algorithm SHA256).Hash -ne $protocExecutableHash) {
    throw 'Extracted protoc executable checksum mismatch'
}
Write-Output $protocExecutable
