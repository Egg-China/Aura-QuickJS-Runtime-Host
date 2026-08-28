[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Assert-Condition {
    param([bool] $Condition, [string] $Message)
    if (-not $Condition) { throw $Message }
}

function Assert-Fails {
    param([scriptblock] $Action, [string] $ExpectedMessage)
    try {
        & $Action
    } catch {
        Assert-Condition ($_.Exception.Message -like "*$ExpectedMessage*") `
            "Expected '$ExpectedMessage', got '$($_.Exception.Message)'"
        return
    }
    throw "Expected failure containing '$ExpectedMessage'"
}

function New-HostBinary {
    param([string] $Path, [string] $Platform)
    $bytes = [byte[]]::new(512)
    if ($Platform.StartsWith('windows-')) {
        $bytes[0] = 0x4d
        $bytes[1] = 0x5a
        [BitConverter]::GetBytes([int32]0x80).CopyTo($bytes, 0x3c)
        $bytes[0x80] = 0x50
        $bytes[0x81] = 0x45
        $machine = if ($Platform.EndsWith('-x64')) { [uint16]0x8664 } else { [uint16]0xaa64 }
        [BitConverter]::GetBytes($machine).CopyTo($bytes, 0x84)
    } elseif ($Platform.StartsWith('linux-')) {
        $magic = [byte[]]@(0x7f, 0x45, 0x4c, 0x46)
        $magic.CopyTo($bytes, 0)
        $bytes[4] = 2
        $bytes[5] = 1
        $machine = if ($Platform.EndsWith('-x64')) { [uint16]62 } else { [uint16]183 }
        [BitConverter]::GetBytes($machine).CopyTo($bytes, 18)
    } else {
        $magic = [byte[]]@(0xcf, 0xfa, 0xed, 0xfe)
        $magic.CopyTo($bytes, 0)
        $cpu = if ($Platform.EndsWith('-x64')) { [uint32]0x01000007 } else { [uint32]0x0100000c }
        [BitConverter]::GetBytes($cpu).CopyTo($bytes, 4)
    }
    [System.IO.File]::WriteAllBytes($Path, $bytes)
}

function New-Package {
    param(
        [string] $Root,
        [string] $Platform,
        [string] $BinaryPlatform = $Platform,
        [switch] $DuplicatePluginJson
    )
    $executableName = if ($Platform.StartsWith('windows-')) {
        'aura-quickjs-host.exe'
    } else {
        'aura-quickjs-host'
    }
    $binary = Join-Path $Root "$Platform-$executableName"
    New-HostBinary -Path $binary -Platform $BinaryPlatform
    $jar = Join-Path $Root "$Platform-provider.jar"
    [System.IO.File]::WriteAllBytes($jar, [byte[]](0x50, 0x4b, 0x03, 0x04))
    $package = Join-Path $Root "dev.hmclce.runtime.quickjs-host-v0.1.0-beta.1-$Platform.npl"
    if (Test-Path -LiteralPath $package) {
        Remove-Item -LiteralPath $package -Force
    }
    $archive = [System.IO.Compression.ZipFile]::Open(
        $package,
        [System.IO.Compression.ZipArchiveMode]::Create
    )
    try {
        $files = [ordered]@{
            'LICENSE' = (Join-Path $PSScriptRoot '..\LICENSE')
            'libs/aura-quickjs-runtime-host-plugin.jar' = $jar
            "native/$Platform/$executableName" = $binary
            'plugin.json' = (Join-Path $PSScriptRoot '..\host-plugin\plugin.json')
        }
        foreach ($entryName in $files.Keys) {
            [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
                $archive,
                $files[$entryName],
                $entryName
            ) | Out-Null
        }
        if ($DuplicatePluginJson) {
            [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
                $archive,
                (Join-Path $PSScriptRoot '..\host-plugin\plugin.json'),
                'plugin.json'
            ) | Out-Null
        }
    } finally {
        $archive.Dispose()
    }
    return $package
}

function New-Record {
    param([string] $Platform, [string] $Package)
    return [pscustomobject][ordered]@{
        platform = $Platform
        package = Split-Path -Leaf $Package
        sha256 = (Get-FileHash -LiteralPath $Package -Algorithm SHA256).Hash.ToLowerInvariant()
        size = (Get-Item -LiteralPath $Package).Length
    }
}

function Write-Manifest {
    param([string] $Path, [object[]] $Artifacts, [string] $AuraCommit = 'c2d7ec3201825308c360c1a41aeafebcd7145e74')
    $manifest = [pscustomobject][ordered]@{
        schemaVersion = 1
        version = '0.1.0-beta.1'
        aura = [pscustomobject][ordered]@{
            repository = 'Egg-China/Aura-Launcher'
            commit = $AuraCommit
            runId = '33196503483'
            jarSha256 = '2153be49da69c055232872c95a171091a526be24357b6f2b82b5af8f6d2a67c3'
        }
        artifacts = $Artifacts
    }
    [System.IO.File]::WriteAllText(
        $Path,
        ($manifest | ConvertTo-Json -Depth 8) + "`n",
        [System.Text.UTF8Encoding]::new($false)
    )
}

$platforms = @(
    'windows-x64',
    'windows-arm64',
    'linux-x64',
    'linux-arm64',
    'macos-x64',
    'macos-arm64'
)
$temporary = Join-Path ([System.IO.Path]::GetTempPath()) `
    ('aura-quickjs-artifact-test-' + [guid]::NewGuid().ToString('N'))
$verifier = Join-Path $PSScriptRoot 'verify-quickjs-host-artifacts.ps1'

try {
    New-Item -ItemType Directory -Path $temporary | Out-Null
    $records = @()
    foreach ($platform in $platforms) {
        $package = New-Package -Root $temporary -Platform $platform
        $records += New-Record -Platform $platform -Package $package
    }
    $manifestPath = Join-Path $temporary 'manifest.json'
    Write-Manifest -Path $manifestPath -Artifacts $records
    & $verifier -ArtifactManifest $manifestPath -PackageDirectory $temporary

    $wrongProvenance = Join-Path $temporary 'wrong-provenance.json'
    Write-Manifest -Path $wrongProvenance -Artifacts $records -AuraCommit ('0' * 40)
    Assert-Fails {
        & $verifier -ArtifactManifest $wrongProvenance -PackageDirectory $temporary
    } 'Aura commit'

    $badHashRecord = New-Record -Platform 'windows-x64' `
        -Package (Join-Path $temporary 'dev.hmclce.runtime.quickjs-host-v0.1.0-beta.1-windows-x64.npl')
    $badHashRecord.sha256 = '0' * 64
    $badHashManifest = Join-Path $temporary 'bad-hash.json'
    Write-Manifest -Path $badHashManifest -Artifacts @($badHashRecord)
    Assert-Fails {
        & $verifier -ArtifactManifest $badHashManifest -PackageDirectory $temporary
    } 'SHA-256'

    $wrongArchitecture = New-Package -Root $temporary -Platform 'windows-x64' -BinaryPlatform 'windows-arm64'
    $wrongArchitectureManifest = Join-Path $temporary 'wrong-architecture.json'
    Write-Manifest -Path $wrongArchitectureManifest `
        -Artifacts @((New-Record -Platform 'windows-x64' -Package $wrongArchitecture))
    Assert-Fails {
        & $verifier -ArtifactManifest $wrongArchitectureManifest -PackageDirectory $temporary
    } 'architecture'

    $duplicate = New-Package -Root $temporary -Platform 'linux-x64' -DuplicatePluginJson
    $duplicateManifest = Join-Path $temporary 'duplicate.json'
    Write-Manifest -Path $duplicateManifest `
        -Artifacts @((New-Record -Platform 'linux-x64' -Package $duplicate))
    Assert-Fails {
        & $verifier -ArtifactManifest $duplicateManifest -PackageDirectory $temporary
    } 'duplicate ZIP entry'
} finally {
    if (Test-Path -LiteralPath $temporary) {
        Remove-Item -LiteralPath $temporary -Recurse -Force
    }
}

Write-Output 'QuickJS Host artifact verifier tests passed'
