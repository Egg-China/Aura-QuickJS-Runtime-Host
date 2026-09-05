[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $RecordDirectory,

    [Parameter(Mandatory = $true)]
    [string] $Output
)

$ErrorActionPreference = 'Stop'

function Assert-Condition {
    param([bool] $Condition, [string] $Message)
    if (-not $Condition) { throw $Message }
}

function Assert-ExactProperties {
    param([object] $Value, [string[]] $Expected, [string] $Label)
    $actual = @($Value.PSObject.Properties.Name | Sort-Object)
    $expectedSorted = @($Expected | Sort-Object)
    Assert-Condition (-not (Compare-Object $actual $expectedSorted -SyncWindow 0)) `
        "$Label contains missing or unknown fields"
}

$platforms = @(
    'windows-x64',
    'windows-arm64',
    'linux-x64',
    'linux-arm64',
    'macos-x64',
    'macos-arm64'
)
$recordRoot = (Resolve-Path -LiteralPath $RecordDirectory).Path
Assert-Condition (Test-Path -LiteralPath $recordRoot -PathType Container) `
    'Artifact record directory must be a directory'
$files = @(Get-ChildItem -LiteralPath $recordRoot -Recurse -File `
    -Filter 'quickjs-runtime-host-*.json')
Assert-Condition ($files.Count -eq $platforms.Count) `
    "Expected exactly six QuickJS artifact records, found $($files.Count)"
$byPlatform = @{}
foreach ($file in $files) {
    $record = Get-Content -LiteralPath $file.FullName -Raw | ConvertFrom-Json
    Assert-ExactProperties -Value $record -Expected @('platform', 'package', 'sha256', 'size') `
        -Label 'artifact record'
    $platform = [string]$record.platform
    Assert-Condition ($platform -cin $platforms) "Unsupported QuickJS Host platform: $platform"
    Assert-Condition (-not $byPlatform.ContainsKey($platform)) "Duplicate artifact platform: $platform"
    Assert-Condition ([string]$record.package -ceq `
        "dev.hmclce.runtime.quickjs-host-v0.1.0-beta.1-$platform.npl") `
        "Artifact package name is invalid for $platform"
    Assert-Condition ([string]$record.sha256 -cmatch '^[0-9a-f]{64}$') `
        "Artifact SHA-256 is invalid for $platform"
    Assert-Condition ([int64]$record.size -gt 0) "Artifact size is invalid for $platform"
    $byPlatform[$platform] = $record
}
$records = @($platforms | ForEach-Object { $byPlatform[$_] })
$manifest = [pscustomobject][ordered]@{
    schemaVersion = 1
    version = '0.1.0-beta.1'
    aura = [pscustomobject][ordered]@{
        repository = 'Egg-China/Aura-Launcher'
        commit = '636b06aad03c5d21946369c836280c891c13054d'
        runId = '33931508945'
        jarSha256 = '674f717f5f97a5b7e8f7f20e4d60aa2e25451d71a96ab475f4595d0482f99d4b'
    }
    artifacts = $records
}
$outputPath = [System.IO.Path]::GetFullPath($Output)
$outputParent = Split-Path -Parent $outputPath
if (-not [string]::IsNullOrWhiteSpace($outputParent)) {
    New-Item -ItemType Directory -Path $outputParent -Force | Out-Null
}
[System.IO.File]::WriteAllText(
    $outputPath,
    ($manifest | ConvertTo-Json -Depth 8) + "`n",
    [System.Text.UTF8Encoding]::new($false)
)
Write-Output $outputPath
