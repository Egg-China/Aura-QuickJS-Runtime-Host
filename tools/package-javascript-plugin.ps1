[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $Source,

    [Parameter(Mandatory = $true)]
    [string] $Output
)

$ErrorActionPreference = 'Stop'

function Assert-ExactProperties {
    param([object] $Value, [string[]] $Expected, [string] $Label)
    $actual = @($Value.PSObject.Properties.Name | Sort-Object)
    $expectedSorted = @($Expected | Sort-Object)
    if ((Compare-Object $actual $expectedSorted -SyncWindow 0)) {
        throw "$Label contains missing or unknown fields"
    }
}

$sourceRoot = (Resolve-Path -LiteralPath $Source).Path
if (-not (Test-Path -LiteralPath $sourceRoot -PathType Container)) {
    throw 'Source must be a directory'
}
$outputPath = [System.IO.Path]::GetFullPath($Output)
$relativeOutput = $outputPath.Substring(0, [Math]::Min($outputPath.Length, $sourceRoot.Length))
if ($relativeOutput -ceq $sourceRoot -and
    ($outputPath.Length -eq $sourceRoot.Length -or $outputPath[$sourceRoot.Length] -eq [System.IO.Path]::DirectorySeparatorChar)) {
    throw 'Output must be outside Source'
}

$pluginPath = Join-Path $sourceRoot 'plugin.json'
$descriptorPath = Join-Path $sourceRoot 'aura-javascript.json'
if (-not (Test-Path -LiteralPath $pluginPath -PathType Leaf) -or
    -not (Test-Path -LiteralPath $descriptorPath -PathType Leaf)) {
    throw 'Source must contain plugin.json and aura-javascript.json'
}

$plugin = Get-Content -LiteralPath $pluginPath -Raw | ConvertFrom-Json
$descriptor = Get-Content -LiteralPath $descriptorPath -Raw | ConvertFrom-Json
Assert-ExactProperties $descriptor @('schemaVersion', 'module') 'aura-javascript.json'
if ($descriptor.schemaVersion -ne 1 -or $descriptor.module -isnot [string]) {
    throw 'aura-javascript.json must contain schemaVersion 1 and a string module'
}
Assert-ExactProperties $plugin @(
    'schemaVersion', 'id', 'name', 'version', 'description', 'author', 'type', 'runtime', 'abi',
    'platforms', 'entrypoint', 'executionMode', 'runtimeProvider', 'dependencies', 'permissions',
    'requiredPermissions', 'hooks', 'patches', 'launcherVersion'
) 'plugin.json'
if ($plugin.schemaVersion -ne 5 -or $plugin.runtime -cne 'javascript' -or $plugin.abi -ne 1 -or
    $plugin.entrypoint -cne 'aura-javascript.json' -or $plugin.executionMode -cne 'isolated' -or
    $plugin.runtimeProvider -cne 'dev.hmclce.runtime.quickjs-host') {
    throw 'plugin.json does not declare the exact isolated JavaScript runtime contract'
}

$validator = Join-Path $PSScriptRoot 'validate-javascript-modules.mjs'
$moduleJson = & node $validator $sourceRoot $descriptor.module
if ($LASTEXITCODE -ne 0) {
    throw 'JavaScript module graph validation failed'
}
$decodedModules = $moduleJson | ConvertFrom-Json
$modules = @($decodedModules | ForEach-Object { $_ })
$entries = @('plugin.json', 'aura-javascript.json') + $modules
if (Test-Path -LiteralPath (Join-Path $sourceRoot 'README.md') -PathType Leaf) {
    $entries += 'README.md'
}
$entries = @($entries | Sort-Object -Unique)

$present = @(Get-ChildItem -LiteralPath $sourceRoot -Recurse -File | ForEach-Object {
    $_.FullName.Substring($sourceRoot.Length).TrimStart('\', '/').Replace('\', '/')
} | Sort-Object)
if (Compare-Object $present $entries -SyncWindow 0) {
    throw "Source contains files outside the validated module graph; present=$($present -join ','); expected=$($entries -join ',')"
}

$outputDirectory = Split-Path -Parent $outputPath
if (-not [string]::IsNullOrEmpty($outputDirectory)) {
    New-Item -ItemType Directory -Force $outputDirectory | Out-Null
}
if (Test-Path -LiteralPath $outputPath) {
    Remove-Item -LiteralPath $outputPath -Force
}

Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [System.IO.Compression.ZipFile]::Open($outputPath, [System.IO.Compression.ZipArchiveMode]::Create)
try {
    $timestamp = [System.DateTimeOffset]::new(1980, 1, 1, 0, 0, 0, [System.TimeSpan]::Zero)
    foreach ($entryName in $entries) {
        $entry = $archive.CreateEntry($entryName, [System.IO.Compression.CompressionLevel]::Optimal)
        $entry.LastWriteTime = $timestamp
        $input = [System.IO.File]::OpenRead((Join-Path $sourceRoot $entryName.Replace('/', '\')))
        $outputStream = $entry.Open()
        try {
            $input.CopyTo($outputStream)
        } finally {
            $outputStream.Dispose()
            $input.Dispose()
        }
    }
} finally {
    $archive.Dispose()
}

Write-Output $outputPath
