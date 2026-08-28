[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $Platform,

    [Parameter(Mandatory = $true)]
    [string] $Version,

    [string] $ProcessHost = '',
    [string] $ProviderJar = '',
    [string] $OutputDirectory = ''
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Assert-Condition {
    param([bool] $Condition, [string] $Message)
    if (-not $Condition) { throw $Message }
}

$platforms = @(
    'windows-x64',
    'windows-arm64',
    'linux-x64',
    'linux-arm64',
    'macos-x64',
    'macos-arm64'
)
Assert-Condition ($Platform -cin $platforms) "Unsupported QuickJS Host platform: $Platform"
Assert-Condition ($Version -ceq '0.1.0-beta.1') 'QuickJS Host version must be 0.1.0-beta.1'
$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$executableName = if ($Platform.StartsWith('windows-')) {
    'aura-quickjs-host.exe'
} else {
    'aura-quickjs-host'
}
if ([string]::IsNullOrWhiteSpace($ProcessHost)) {
    $ProcessHost = Join-Path $repositoryRoot "target\release\$executableName"
}
if ([string]::IsNullOrWhiteSpace($ProviderJar)) {
    $ProviderJar = Join-Path $repositoryRoot `
        'host-plugin\build\libs\aura-quickjs-runtime-host-plugin.jar'
}
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $repositoryRoot 'artifacts'
}
Assert-Condition (Test-Path -LiteralPath $ProcessHost -PathType Leaf) `
    "QuickJS process Host does not exist: $ProcessHost"
Assert-Condition ((Split-Path -Leaf $ProcessHost) -ceq $executableName) `
    "QuickJS process Host for $Platform must be named $executableName"
Assert-Condition (Test-Path -LiteralPath $ProviderJar -PathType Leaf) `
    "QuickJS Java Provider JAR does not exist: $ProviderJar"
$pluginJson = Join-Path $repositoryRoot 'host-plugin\plugin.json'
$license = Join-Path $repositoryRoot 'LICENSE'
Assert-Condition (Test-Path -LiteralPath $pluginJson -PathType Leaf) 'Host plugin.json is missing'
Assert-Condition (Test-Path -LiteralPath $license -PathType Leaf) 'GPL license file is missing'

$outputRoot = [System.IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Path $outputRoot -Force | Out-Null
$packageName = "dev.hmclce.runtime.quickjs-host-v$Version-$Platform.npl"
$packagePath = Join-Path $outputRoot $packageName
if (Test-Path -LiteralPath $packagePath) {
    Remove-Item -LiteralPath $packagePath -Force
}
$entries = [ordered]@{
    'LICENSE' = $license
    'libs/aura-quickjs-runtime-host-plugin.jar' = $ProviderJar
    "native/$Platform/$executableName" = $ProcessHost
    'plugin.json' = $pluginJson
}
$archive = [System.IO.Compression.ZipFile]::Open(
    $packagePath,
    [System.IO.Compression.ZipArchiveMode]::Create
)
try {
    $timestamp = [System.DateTimeOffset]::new(1980, 1, 1, 0, 0, 0, [TimeSpan]::Zero)
    foreach ($entryName in $entries.Keys) {
        $entry = $archive.CreateEntry($entryName, [System.IO.Compression.CompressionLevel]::Optimal)
        $entry.LastWriteTime = $timestamp
        $input = [System.IO.File]::OpenRead($entries[$entryName])
        $output = $entry.Open()
        try {
            $input.CopyTo($output)
        } finally {
            $output.Dispose()
            $input.Dispose()
        }
    }
} finally {
    $archive.Dispose()
}

$record = [pscustomobject][ordered]@{
    platform = $Platform
    package = $packageName
    sha256 = (Get-FileHash -LiteralPath $packagePath -Algorithm SHA256).Hash.ToLowerInvariant()
    size = (Get-Item -LiteralPath $packagePath).Length
}
$recordJson = ($record | ConvertTo-Json -Depth 4) + "`n"
[System.IO.File]::WriteAllText(
    (Join-Path $outputRoot "quickjs-runtime-host-$Platform.json"),
    $recordJson,
    [System.Text.UTF8Encoding]::new($false)
)
$manifest = [pscustomobject][ordered]@{
    schemaVersion = 1
    version = $Version
    aura = [pscustomobject][ordered]@{
        repository = 'Egg-China/Aura-Launcher'
        commit = 'c2d7ec3201825308c360c1a41aeafebcd7145e74'
        runId = '33196503483'
        jarSha256 = '2153be49da69c055232872c95a171091a526be24357b6f2b82b5af8f6d2a67c3'
    }
    artifacts = @($record)
}
[System.IO.File]::WriteAllText(
    (Join-Path $outputRoot 'manifest.json'),
    ($manifest | ConvertTo-Json -Depth 8) + "`n",
    [System.Text.UTF8Encoding]::new($false)
)
Write-Output $packagePath
