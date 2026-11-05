param([string]$TempDirectory = '')
$ErrorActionPreference = 'Stop'
$docRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not (Get-Command pandoc -ErrorAction SilentlyContinue)) { throw 'Pandoc is required to regenerate the website user guides' }
if (-not $TempDirectory) { $TempDirectory = Join-Path $docRoot '.local-archive/test-runs/user-docs-build' }
New-Item -ItemType Directory -Path $TempDirectory -Force | Out-Null
$documents = @(
    @{Source='docs/quick-start.md'; Name='quick-start'; Title='快速上手'},
    @{Source='docs/user-guide.md'; Name='user-guide'; Title='用户指南'},
    @{Source='docs/known-issues.md'; Name='known-issues'; Title='已知问题'},
    @{Source='docs/privacy.md'; Name='privacy'; Title='隐私说明'},
    @{Source='docs/format-support-matrix.md'; Name='format-support'; Title='格式支持'},
    @{Source='THIRD-PARTY-NOTICES.md'; Name='third-party-notices'; Title='第三方许可'},
    @{Source='LICENSE'; Name='license'; Title='使用许可'},
    @{Source='CHANGELOG.md'; Name='changelog'; Title='版本记录'}
)
$linkTargets = @{}
foreach ($document in $documents) { $linkTargets[[IO.Path]::GetFullPath((Join-Path $docRoot $document.Source))] = "$($document.Name).html" }
foreach ($document in $documents) {
    $sourcePath = Join-Path $docRoot $document.Source
    $sourceParent = Split-Path -Parent $sourcePath
    $markdown = Get-Content -LiteralPath $sourcePath -Raw
    $markdown = [regex]::Replace($markdown, '\]\(([^)]+)\)', {
        param($match)
        $url = $match.Groups[1].Value
        if ($url -match '^(?:[a-z]+:|#|//)') { return $match.Value }
        $pieces = $url -split '#', 2
        $target = [IO.Path]::GetFullPath((Join-Path $sourceParent $pieces[0]))
        $relative = if ($linkTargets.ContainsKey($target)) { $linkTargets[$target] } else {
            [IO.Path]::GetRelativePath((Join-Path $docRoot 'docs/guide'), $target).Replace('\','/')
        }
        if ($pieces.Count -eq 2) { $relative += '#' + $pieces[1] }
        return '](' + $relative + ')'
    })
    if ($document.Name -eq 'license') { $markdown = "# 使用许可`n`n" + $markdown }
    $inputPath = Join-Path $TempDirectory "$($document.Name).md"
    $markdown | Set-Content -LiteralPath $inputPath -Encoding utf8NoBOM
    & pandoc --from=gfm --to=html5 --standalone --wrap=none --template (Join-Path $docRoot 'docs/guide/doc-template.html') --metadata "title=$($document.Title)" --output (Join-Path $docRoot "docs/guide/$($document.Name).html") $inputPath
    if ($LASTEXITCODE -ne 0) { throw "User guide generation failed: $($document.Name)" }
}
Write-Output "Generated $($documents.Count) user guides from maintained sources"
