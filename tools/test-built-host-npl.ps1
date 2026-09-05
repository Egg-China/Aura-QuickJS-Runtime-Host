[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $ArtifactManifest,
    [Parameter(Mandatory = $true)] [string] $PackageDirectory
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

# Test copies of the actual build output; never mutate the deliverable.
$manifestText = Get-Content -LiteralPath $ArtifactManifest -Raw
$sourceManifest = $manifestText | ConvertFrom-Json
if (@($sourceManifest.artifacts).Count -ne 1) { throw 'Expected one native build artifact' }
$sourceRecord = @($sourceManifest.artifacts)[0]
$sourcePackage = Join-Path $PackageDirectory $sourceRecord.package
$verifier = Join-Path $PSScriptRoot 'verify-quickjs-host-artifacts.ps1'
& $verifier -ArtifactManifest $ArtifactManifest -PackageDirectory $PackageDirectory

function Assert-Rejected {
    param([scriptblock] $Action, [string] $ExpectedMessage, [string] $Case)
    try { & $Action } catch {
        if ($ExpectedMessage -and $_.Exception.Message -notlike "*$ExpectedMessage*") {
            throw "${Case}: unexpected rejection: $($_.Exception.Message)"
        }
        return
    }
    throw "${Case}: invalid built NPL was accepted"
}

$temporaryBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$temporary = Join-Path $temporaryBase ('aura-quickjs-built-npl-' + [guid]::NewGuid().ToString('N'))
try {
    New-Item -ItemType Directory -Path $temporary | Out-Null
    foreach ($case in @('bridge', 'hooks', 'patches', 'native', 'schema', 'entrypoint', 'corrupt')) {
        $package = Join-Path $temporary $sourceRecord.package
        Copy-Item -LiteralPath $sourcePackage -Destination $package -Force
        $expected = 'NPL JavaScript runtime declaration is invalid'
        if ($case -ceq 'corrupt') {
            [IO.File]::WriteAllBytes($package, [byte[]]@(0x50, 0x4b, 0x03, 0x04))
            $expected = ''
        } else {
            $archive = [IO.Compression.ZipFile]::Open($package, [IO.Compression.ZipArchiveMode]::Update)
            try {
                $entry = $archive.GetEntry('plugin.json')
                $reader = [IO.StreamReader]::new($entry.Open())
                try { $plugin = $reader.ReadToEnd() | ConvertFrom-Json } finally { $reader.Dispose() }
                if ($case -ceq 'schema') {
                    $plugin.schemaVersion = 4
                    $expected = 'NPL plugin.json must use schema v5'
                } elseif ($case -ceq 'entrypoint') {
                    $plugin.entrypoint = 'invalid.Provider'
                    $expected = 'NPL Java Provider entrypoint is invalid'
                } else {
                    $plugin.providesRuntimes[0].features = @(
                        $plugin.providesRuntimes[0].features | Where-Object { $_ -cne $case }
                    )
                }
                $entry.Delete()
                $writer = [IO.StreamWriter]::new($archive.CreateEntry('plugin.json').Open(), [Text.UTF8Encoding]::new($false))
                try { $writer.Write(($plugin | ConvertTo-Json -Depth 32)) } finally { $writer.Dispose() }
            } finally { $archive.Dispose() }
        }
        # Recompute outer integrity so rejection must inspect the NPL itself.
        $manifest = $manifestText | ConvertFrom-Json
        $manifest.artifacts[0].sha256 = (Get-FileHash -LiteralPath $package -Algorithm SHA256).Hash.ToLowerInvariant()
        $manifest.artifacts[0].size = (Get-Item -LiteralPath $package).Length
        $testManifest = Join-Path $temporary 'manifest.json'
        [IO.File]::WriteAllText($testManifest, ($manifest | ConvertTo-Json -Depth 32), [Text.UTF8Encoding]::new($false))
        Assert-Rejected { & $verifier -ArtifactManifest $testManifest -PackageDirectory $temporary } $expected $case
    }
} finally {
    $resolved = [IO.Path]::GetFullPath($temporary)
    $expectedParent = [IO.Path]::GetFullPath($temporaryBase).TrimEnd([IO.Path]::DirectorySeparatorChar)
    if ([IO.Path]::GetDirectoryName($resolved) -cne $expectedParent -or
        -not [IO.Path]::GetFileName($resolved).StartsWith('aura-quickjs-built-npl-')) {
        throw 'Refusing cleanup outside the exact test temporary directory'
    }
    if (Test-Path -LiteralPath $resolved) { Remove-Item -LiteralPath $resolved -Recurse -Force }
}
Write-Output 'Built QuickJS NPL: valid artifact accepted; seven internal corruptions rejected'
