[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
& node (Join-Path $PSScriptRoot 'validate-ci-workflows.mjs')
if ($LASTEXITCODE -ne 0) {
    throw 'QuickJS CI workflow contract validation failed'
}
