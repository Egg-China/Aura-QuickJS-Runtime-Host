[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Assert-Condition {
    param([bool] $Condition, [string] $Message)
    if (-not $Condition) { throw $Message }
}

$temporary = Join-Path ([System.IO.Path]::GetTempPath()) `
    ('aura-quickjs-publishing-test-' + [guid]::NewGuid().ToString('N'))
$packager = Join-Path $PSScriptRoot 'package-host-npl.ps1'
$merger = Join-Path $PSScriptRoot 'merge-artifact-manifests.ps1'
$verifier = Join-Path $PSScriptRoot 'verify-quickjs-host-artifacts.ps1'
$sbomWriter = Join-Path $PSScriptRoot 'write-sbom.ps1'

try {
    New-Item -ItemType Directory -Path $temporary | Out-Null
    $binary = Join-Path $temporary 'aura-quickjs-host.exe'
    $bytes = [byte[]]::new(512)
    $bytes[0] = 0x4d
    $bytes[1] = 0x5a
    [BitConverter]::GetBytes([int32]0x80).CopyTo($bytes, 0x3c)
    $bytes[0x80] = 0x50
    $bytes[0x81] = 0x45
    [BitConverter]::GetBytes([uint16]0x8664).CopyTo($bytes, 0x84)
    [System.IO.File]::WriteAllBytes($binary, $bytes)
    $jar = Join-Path $temporary 'aura-quickjs-runtime-host-plugin.jar'
    [System.IO.File]::WriteAllBytes($jar, [byte[]](0x50, 0x4b, 0x03, 0x04))
    $output = Join-Path $temporary 'output'

    & $packager -Platform windows-x64 -Version 0.1.0-beta.1 `
        -ProcessHost $binary -ProviderJar $jar -OutputDirectory $output
    $package = Join-Path $output `
        'dev.hmclce.runtime.quickjs-host-v0.1.0-beta.1-windows-x64.npl'
    $firstHash = (Get-FileHash -LiteralPath $package -Algorithm SHA256).Hash
    & $packager -Platform windows-x64 -Version 0.1.0-beta.1 `
        -ProcessHost $binary -ProviderJar $jar -OutputDirectory $output
    Assert-Condition ((Get-FileHash -LiteralPath $package -Algorithm SHA256).Hash -ceq $firstHash) `
        'QuickJS NPL packaging is not deterministic'
    & $verifier -ArtifactManifest (Join-Path $output 'manifest.json') -PackageDirectory $output

    $archive = [System.IO.Compression.ZipFile]::OpenRead($package)
    try {
        $timestamps = @($archive.Entries | ForEach-Object { $_.LastWriteTime.DateTime })
        Assert-Condition ($timestamps.Count -eq 4) 'QuickJS NPL must contain exactly four files'
        foreach ($timestamp in $timestamps) {
            Assert-Condition ($timestamp -eq [datetime]'1980-01-01T00:00:00') `
                'QuickJS NPL contains a non-deterministic timestamp'
        }
    } finally {
        $archive.Dispose()
    }

    $records = Join-Path $temporary 'records'
    New-Item -ItemType Directory -Path $records | Out-Null
    $platforms = @(
        'windows-x64',
        'windows-arm64',
        'linux-x64',
        'linux-arm64',
        'macos-x64',
        'macos-arm64'
    )
    foreach ($platform in $platforms) {
        $record = [pscustomobject][ordered]@{
            platform = $platform
            package = "dev.hmclce.runtime.quickjs-host-v0.1.0-beta.1-$platform.npl"
            sha256 = 'a' * 64
            size = 123
        }
        [System.IO.File]::WriteAllText(
            (Join-Path $records "quickjs-runtime-host-$platform.json"),
            ($record | ConvertTo-Json) + "`n",
            [System.Text.UTF8Encoding]::new($false)
        )
    }
    $mergedPath = Join-Path $temporary 'merged.json'
    & $merger -RecordDirectory $records -Output $mergedPath
    $merged = Get-Content -LiteralPath $mergedPath -Raw | ConvertFrom-Json
    Assert-Condition (@($merged.artifacts).Count -eq 6) 'Merged manifest does not contain six artifacts'
    Assert-Condition ((@($merged.artifacts.platform) -join ',') -ceq ($platforms -join ',')) `
        'Merged manifest platform order is not canonical'
    Assert-Condition ($merged.aura.jarSha256 -ceq `
        '2153be49da69c055232872c95a171091a526be24357b6f2b82b5af8f6d2a67c3') `
        'Merged manifest lost Aura JAR provenance'

    $sbomPath = Join-Path $temporary 'quickjs-runtime-host.cdx.json'
    & $sbomWriter -Output $sbomPath
    $sbom = Get-Content -LiteralPath $sbomPath -Raw | ConvertFrom-Json
    Assert-Condition ($sbom.bomFormat -ceq 'CycloneDX' -and $sbom.specVersion -ceq '1.6') `
        'SBOM does not identify CycloneDX 1.6'
    $componentNames = @($sbom.components | ForEach-Object { [string]$_.name })
    Assert-Condition ($componentNames -ccontains 'aura-quickjs-host') `
        'SBOM does not contain the process Host crate'
    Assert-Condition ($componentNames -ccontains 'libquickjs-ng-sys') `
        'SBOM does not contain the embedded QuickJS dependency'
} finally {
    if (Test-Path -LiteralPath $temporary) {
        Remove-Item -LiteralPath $temporary -Recurse -Force
    }
}

Write-Output 'QuickJS publishing tool tests passed'
