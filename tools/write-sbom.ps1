[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $Output
)

$ErrorActionPreference = 'Stop'
$metadataJson = (& cargo metadata --format-version 1 --locked | Out-String)
if ($LASTEXITCODE -ne 0) {
    throw 'cargo metadata failed while generating the QuickJS Host SBOM'
}
$metadata = $metadataJson | ConvertFrom-Json
$references = @{}
foreach ($package in $metadata.packages) {
    $references[[string]$package.id] = "pkg:cargo/$($package.name)@$($package.version)"
}
$components = @($metadata.packages | Sort-Object name, version | ForEach-Object {
    $licenses = if ([string]::IsNullOrWhiteSpace([string]$_.license)) {
        @()
    } else {
        @([pscustomobject][ordered]@{ expression = [string]$_.license })
    }
    [pscustomobject][ordered]@{
        type = 'library'
        'bom-ref' = $references[[string]$_.id]
        name = [string]$_.name
        version = [string]$_.version
        licenses = $licenses
        purl = $references[[string]$_.id]
    }
})
$dependencies = @($metadata.resolve.nodes | ForEach-Object {
    $reference = $references[[string]$_.id]
    $dependsOn = @($_.dependencies | ForEach-Object { $references[[string]$_] } | Sort-Object)
    [pscustomobject][ordered]@{
        ref = $reference
        dependsOn = $dependsOn
    }
} | Sort-Object ref)
$sbom = [pscustomobject][ordered]@{
    bomFormat = 'CycloneDX'
    specVersion = '1.6'
    version = 1
    metadata = [pscustomobject][ordered]@{
        component = [pscustomobject][ordered]@{
            type = 'application'
            'bom-ref' = 'pkg:generic/aura-quickjs-runtime-host@0.1.0-beta.1'
            name = 'Aura QuickJS Runtime Host'
            version = '0.1.0-beta.1'
        }
    }
    components = $components
    dependencies = $dependencies
}
$outputPath = [System.IO.Path]::GetFullPath($Output)
$outputParent = Split-Path -Parent $outputPath
if (-not [string]::IsNullOrWhiteSpace($outputParent)) {
    New-Item -ItemType Directory -Path $outputParent -Force | Out-Null
}
[System.IO.File]::WriteAllText(
    $outputPath,
    ($sbom | ConvertTo-Json -Depth 12) + "`n",
    [System.Text.UTF8Encoding]::new($false)
)
Write-Output $outputPath
