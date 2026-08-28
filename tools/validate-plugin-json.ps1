[CmdletBinding()]
param(
    [Parameter(Mandatory = $false)]
    [string] $Path = ''
)

$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($Path)) {
    $Path = Join-Path $PSScriptRoot '..\host-plugin\plugin.json'
}
$manifest = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json

function Assert-Equal {
    param([object] $Actual, [object] $Expected, [string] $Label)
    if ($Actual -cne $Expected) {
        throw "$Label must be '$Expected', found '$Actual'"
    }
}

Assert-Equal $manifest.schemaVersion 5 'schemaVersion'
Assert-Equal $manifest.id 'dev.hmclce.runtime.quickjs-host' 'id'
Assert-Equal $manifest.version '0.1.0-beta.1' 'version'
Assert-Equal $manifest.entrypoint 'dev.hmclce.runtime.quickjs.QuickJsRuntimeHostPlugin' 'entrypoint'
Assert-Equal $manifest.launcherVersion '>=27.1-0-next' 'launcherVersion'

$expectedPlatforms = @('windows-x64', 'windows-arm64', 'linux-x64', 'linux-arm64', 'macos-x64', 'macos-arm64')
$actualPlatforms = @($manifest.platforms)
if (Compare-Object $expectedPlatforms $actualPlatforms -SyncWindow 0) {
    throw 'platforms must contain the exact six Aura targets in canonical order'
}

if ((@($manifest.permissions) -join ',') -cne 'native-code' -or
    (@($manifest.requiredPermissions) -join ',') -cne 'native-code') {
    throw 'permissions and requiredPermissions must contain only native-code'
}

$declarations = @($manifest.providesRuntimes)
if ($declarations.Count -ne 1) {
    throw 'providesRuntimes must contain exactly one declaration'
}
$declaration = $declarations[0]
Assert-Equal $declaration.runtime 'javascript' 'providesRuntimes.runtime'
Assert-Equal $declaration.bridgeAbi 1 'providesRuntimes.bridgeAbi'
if ((@($declaration.abis) -join ',') -cne '1' -or
    (@($declaration.executionModes) -join ',') -cne 'isolated' -or
    (@($declaration.features) -join ',') -cne 'bridge,hooks,native') {
    throw 'runtime declaration does not match JavaScript ABI 1 isolated Bridge contract'
}

Write-Output "Validated $Path"
