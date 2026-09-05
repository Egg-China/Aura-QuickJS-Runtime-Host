[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $ArtifactManifest,

    [Parameter(Mandatory = $true)]
    [string] $PackageDirectory
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

$expectedVersion = '0.1.0-beta.1'
$expectedAuraRepository = 'Egg-China/Aura-Launcher'
$expectedAuraCommit = '636b06aad03c5d21946369c836280c891c13054d'
$expectedAuraRun = '33931508945'
$expectedAuraJarHash = '674f717f5f97a5b7e8f7f20e4d60aa2e25451d71a96ab475f4595d0482f99d4b'
$platforms = @(
    'windows-x64',
    'windows-arm64',
    'linux-x64',
    'linux-arm64',
    'macos-x64',
    'macos-arm64'
)

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

function Get-ZipEntryBytes {
    param([System.IO.Compression.ZipArchiveEntry] $Entry)
    $input = $Entry.Open()
    $output = [System.IO.MemoryStream]::new()
    try {
        $input.CopyTo($output)
        return $output.ToArray()
    } finally {
        $output.Dispose()
        $input.Dispose()
    }
}

function Assert-BinaryArchitecture {
    param([byte[]] $Bytes, [string] $Platform)
    if ($Platform.StartsWith('windows-')) {
        Assert-Condition ($Bytes.Length -ge 0x88 -and $Bytes[0] -eq 0x4d -and $Bytes[1] -eq 0x5a) `
            "$Platform executable architecture is not a valid PE image"
        $header = [BitConverter]::ToInt32($Bytes, 0x3c)
        Assert-Condition ($header -ge 0 -and $header + 6 -le $Bytes.Length) `
            "$Platform executable architecture has an invalid PE header"
        Assert-Condition (
            $Bytes[$header] -eq 0x50 -and $Bytes[$header + 1] -eq 0x45 -and
            $Bytes[$header + 2] -eq 0 -and $Bytes[$header + 3] -eq 0
        ) "$Platform executable architecture has an invalid PE signature"
        $actual = [BitConverter]::ToUInt16($Bytes, $header + 4)
        $expected = if ($Platform.EndsWith('-x64')) { [uint16]0x8664 } else { [uint16]0xaa64 }
        Assert-Condition ($actual -eq $expected) "$Platform executable architecture does not match its platform"
        return
    }

    if ($Platform.StartsWith('linux-')) {
        Assert-Condition (
            $Bytes.Length -ge 20 -and $Bytes[0] -eq 0x7f -and $Bytes[1] -eq 0x45 -and
            $Bytes[2] -eq 0x4c -and $Bytes[3] -eq 0x46 -and $Bytes[4] -eq 2 -and $Bytes[5] -eq 1
        ) "$Platform executable architecture is not a little-endian ELF64 image"
        $actual = [BitConverter]::ToUInt16($Bytes, 18)
        $expected = if ($Platform.EndsWith('-x64')) { [uint16]62 } else { [uint16]183 }
        Assert-Condition ($actual -eq $expected) "$Platform executable architecture does not match its platform"
        return
    }

    Assert-Condition (
        $Bytes.Length -ge 8 -and $Bytes[0] -eq 0xcf -and $Bytes[1] -eq 0xfa -and
        $Bytes[2] -eq 0xed -and $Bytes[3] -eq 0xfe
    ) "$Platform executable architecture is not a little-endian Mach-O 64 image"
    $actual = [BitConverter]::ToUInt32($Bytes, 4)
    $expected = if ($Platform.EndsWith('-x64')) { [uint32]0x01000007 } else { [uint32]0x0100000c }
    Assert-Condition ($actual -eq $expected) "$Platform executable architecture does not match its platform"
}

function Assert-PluginManifest {
    param([System.IO.Compression.ZipArchiveEntry] $Entry, [string] $Platform)
    $stream = $Entry.Open()
    $reader = [System.IO.StreamReader]::new(
        $stream,
        [System.Text.UTF8Encoding]::new($false, $true),
        $false
    )
    try {
        $plugin = $reader.ReadToEnd() | ConvertFrom-Json
    } finally {
        $reader.Dispose()
        $stream.Dispose()
    }
    Assert-Condition ($plugin.schemaVersion -eq 5) 'NPL plugin.json must use schema v5'
    Assert-Condition ($plugin.id -ceq 'dev.hmclce.runtime.quickjs-host') 'NPL plugin ID is invalid'
    Assert-Condition ($plugin.version -ceq $expectedVersion) 'NPL plugin version is invalid'
    Assert-Condition ($plugin.pluginKind -ceq 'runtime-provider') 'NPL plugin kind is invalid'
    Assert-Condition ($plugin.entrypoint -ceq 'dev.hmclce.runtime.quickjs.QuickJsRuntimeHostPlugin') `
        'NPL Java Provider entrypoint is invalid'
    Assert-Condition ($plugin.launcherVersion -ceq '>=27.1-0-next') 'NPL launcherVersion is invalid'
    Assert-Condition ($Platform -cin @($plugin.platforms)) 'NPL plugin.json does not declare its platform'
    Assert-Condition ((@($plugin.permissions) -join ',') -ceq 'native-code') 'NPL permissions are invalid'
    Assert-Condition ((@($plugin.requiredPermissions) -join ',') -ceq 'native-code') `
        'NPL required permissions are invalid'
    $declarations = @($plugin.providesRuntimes)
    Assert-Condition ($declarations.Count -eq 1) 'NPL must provide exactly one runtime'
    $runtime = $declarations[0]
    Assert-Condition (
        $runtime.runtime -ceq 'javascript' -and
        (@($runtime.abis) -join ',') -ceq '1' -and
        $runtime.bridgeAbi -eq 1 -and
        (@($runtime.executionModes) -join ',') -ceq 'isolated' -and
        (@($runtime.features) -join ',') -ceq 'bridge,hooks,patches,native'
    ) 'NPL JavaScript runtime declaration is invalid'
}

function Assert-Package {
    param([string] $Package, [string] $Platform)
    $executable = if ($Platform.StartsWith('windows-')) {
        'aura-quickjs-host.exe'
    } else {
        'aura-quickjs-host'
    }
    $required = @(
        'LICENSE',
        'libs/aura-quickjs-runtime-host-plugin.jar',
        "native/$Platform/$executable",
        'plugin.json'
    ) | Sort-Object
    $archive = [System.IO.Compression.ZipFile]::OpenRead($Package)
    try {
        $seen = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
        $files = @()
        foreach ($entry in $archive.Entries) {
            $name = $entry.FullName
            Assert-Condition (-not [string]::IsNullOrWhiteSpace($name)) 'NPL contains a blank ZIP entry'
            Assert-Condition (
                -not $name.Contains('\') -and -not $name.StartsWith('/') -and
                -not $name.Contains(':') -and -not $name.Contains([char]0)
            ) "NPL contains an unsafe ZIP entry: $name"
            $segments = @($name.TrimEnd('/').Split('/'))
            Assert-Condition (-not ($segments | Where-Object { $_ -ceq '' -or $_ -ceq '.' -or $_ -ceq '..' })) `
                "NPL contains an unsafe ZIP entry: $name"
            Assert-Condition ($seen.Add($name)) "NPL contains duplicate ZIP entry: $name"
            if (-not $name.EndsWith('/')) { $files += $name }
        }
        Assert-Condition (-not (Compare-Object @($files | Sort-Object) $required -SyncWindow 0)) `
            'NPL file entries do not match the exact Host package layout'

        $pluginEntry = $archive.GetEntry('plugin.json')
        Assert-Condition ($null -ne $pluginEntry) 'NPL is missing plugin.json'
        Assert-PluginManifest -Entry $pluginEntry -Platform $Platform

        $executableEntry = $archive.GetEntry("native/$Platform/$executable")
        Assert-Condition ($null -ne $executableEntry) 'NPL is missing the process Host executable'
        Assert-BinaryArchitecture -Bytes (Get-ZipEntryBytes -Entry $executableEntry) -Platform $Platform
    } finally {
        $archive.Dispose()
    }
}

$manifestPath = (Resolve-Path -LiteralPath $ArtifactManifest).Path
$packageRoot = (Resolve-Path -LiteralPath $PackageDirectory).Path
Assert-Condition (Test-Path -LiteralPath $manifestPath -PathType Leaf) 'Artifact manifest must be a file'
Assert-Condition (Test-Path -LiteralPath $packageRoot -PathType Container) 'Package directory must be a directory'
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
Assert-ExactProperties -Value $manifest -Expected @('schemaVersion', 'version', 'aura', 'artifacts') `
    -Label 'artifact manifest'
Assert-Condition ($manifest.schemaVersion -eq 1) 'Artifact manifest schemaVersion must be 1'
Assert-Condition ($manifest.version -ceq $expectedVersion) 'Artifact manifest version is invalid'
Assert-ExactProperties -Value $manifest.aura `
    -Expected @('repository', 'commit', 'runId', 'jarSha256') -Label 'artifact manifest Aura provenance'
Assert-Condition ($manifest.aura.repository -ceq $expectedAuraRepository) 'Aura repository provenance is invalid'
Assert-Condition ($manifest.aura.commit -ceq $expectedAuraCommit) 'Aura commit provenance is invalid'
Assert-Condition ([string]$manifest.aura.runId -ceq $expectedAuraRun) 'Aura run provenance is invalid'
Assert-Condition ($manifest.aura.jarSha256 -ceq $expectedAuraJarHash) 'Aura JAR SHA-256 provenance is invalid'

$artifacts = @($manifest.artifacts)
Assert-Condition ($artifacts.Count -ge 1 -and $artifacts.Count -le $platforms.Count) `
    'Artifact manifest must contain between one and six packages'
$seenPlatforms = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
foreach ($artifact in $artifacts) {
    Assert-ExactProperties -Value $artifact -Expected @('platform', 'package', 'sha256', 'size') `
        -Label 'artifact record'
    $platform = [string]$artifact.platform
    Assert-Condition ($platform -cin $platforms) "Unsupported QuickJS Host platform: $platform"
    Assert-Condition ($seenPlatforms.Add($platform)) "Duplicate artifact platform: $platform"
    $packageName = [string]$artifact.package
    $expectedName = "dev.hmclce.runtime.quickjs-host-v$expectedVersion-$platform.npl"
    Assert-Condition (
        [System.IO.Path]::GetFileName($packageName) -ceq $packageName -and $packageName -ceq $expectedName
    ) "Artifact package name is invalid for $platform"
    $packagePath = Join-Path $packageRoot $packageName
    Assert-Condition (Test-Path -LiteralPath $packagePath -PathType Leaf) `
        "Artifact package does not exist: $packageName"
    $actualHash = (Get-FileHash -LiteralPath $packagePath -Algorithm SHA256).Hash.ToLowerInvariant()
    Assert-Condition ($actualHash -ceq [string]$artifact.sha256) "$platform NPL SHA-256 does not match"
    Assert-Condition ((Get-Item -LiteralPath $packagePath).Length -eq [int64]$artifact.size) `
        "$platform NPL size does not match"
    Assert-Package -Package $packagePath -Platform $platform
}

Write-Output "Verified $($artifacts.Count) QuickJS Host artifact(s) from Aura run $expectedAuraRun"
