[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$temporary = Join-Path ([System.IO.Path]::GetTempPath()) ('aura-quickjs-package-test-' + [guid]::NewGuid().ToString('N'))
$good = Join-Path $temporary 'good'
$unsafe = Join-Path $temporary 'unsafe'
$output = Join-Path $temporary 'good.npl'

try {
    New-Item -ItemType Directory -Force $good, $unsafe | Out-Null
    $plugin = @'
{"schemaVersion":5,"id":"dev.hmclce.test.javascript","name":"Test","version":"1.0.0","description":"Test","author":"Test","type":"java","runtime":"javascript","abi":1,"platforms":["windows-x64"],"entrypoint":"aura-javascript.json","executionMode":"isolated","runtimeProvider":"dev.hmclce.runtime.quickjs-host","dependencies":[],"permissions":[],"requiredPermissions":[],"hooks":[],"patches":[],"launcherVersion":">=27.1-0-next"}
'@
    Set-Content -LiteralPath (Join-Path $good 'plugin.json') -Value $plugin -NoNewline
    Set-Content -LiteralPath (Join-Path $good 'aura-javascript.json') -Value '{"schemaVersion":1,"module":"main.mjs"}' -NoNewline
    Set-Content -LiteralPath (Join-Path $good 'main.mjs') -Value "import './nested.mjs'; export function load(){}; export function enable(){}; export function invoke(_o,i){return i}; export function disable(){}; export function unload(){}" -NoNewline
    Set-Content -LiteralPath (Join-Path $good 'nested.mjs') -Value 'export const value = 42;' -NoNewline

    & (Join-Path $PSScriptRoot 'package-javascript-plugin.ps1') -Source $good -Output $output
    if (-not (Test-Path -LiteralPath $output -PathType Leaf)) {
        throw 'safe package was not created'
    }
    Add-Type -AssemblyName System.IO.Compression
    $archive = [System.IO.Compression.ZipFile]::OpenRead($output)
    try {
        $names = @($archive.Entries | ForEach-Object FullName)
    } finally {
        $archive.Dispose()
    }
    if (($names -join ',') -cne 'aura-javascript.json,main.mjs,nested.mjs,plugin.json') {
        throw "unexpected deterministic archive entries: $($names -join ',')"
    }

    Set-Content -LiteralPath (Join-Path $unsafe 'plugin.json') -Value $plugin -NoNewline
    Set-Content -LiteralPath (Join-Path $unsafe 'aura-javascript.json') -Value '{"schemaVersion":1,"module":"main.mjs"}' -NoNewline
    Set-Content -LiteralPath (Join-Path $unsafe 'main.mjs') -Value "import '../outside.mjs';" -NoNewline
    $failed = $false
    try {
        & (Join-Path $PSScriptRoot 'package-javascript-plugin.ps1') `
            -Source $unsafe `
            -Output (Join-Path $temporary 'unsafe.npl') 2>$null
    } catch {
        $failed = $true
    }
    if (-not $failed) {
        throw 'unsafe parent import was accepted'
    }
} finally {
    if (Test-Path -LiteralPath $temporary) {
        Remove-Item -LiteralPath $temporary -Recurse -Force
    }
}

Write-Output 'JavaScript packaging tests passed'
