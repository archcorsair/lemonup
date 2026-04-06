param(
    [Parameter(Mandatory = $true)]
    [string]$SandboxRoot,
    [string]$Profile = 'manual-smoke'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'Invoke-Lemonup.ps1')

Ensure-LemonupBinary | Out-Null
$matrix = Read-ManualTestMatrix
$layout = Initialize-WowSandboxLayout -SandboxRoot $SandboxRoot

$results = New-Object System.Collections.Generic.List[string]

foreach ($target in $matrix | Where-Object { $_.requires_api_key }) {
    $installParts = Split-ManifestCommand -CommandText $target.install_command
    $preflight = Invoke-Lemonup -Profile $Profile -AddonDir $layout.AddonsDir -Arguments ($installParts + '--dry-run')
    Assert-ProcessSucceeded -Result $preflight -Context "preflight $($target.selector)"
    if ($preflight.Output -match 'reason=no Wago API key configured') {
        throw "seed preflight failed for $($target.selector): no Wago API key configured"
    }
}

$layout = Initialize-WowSandboxLayout -SandboxRoot $SandboxRoot -Reset
Reset-LemonupProfileState -Profile $Profile -PreserveConfig

foreach ($target in $matrix) {
    if ($target.provider -eq 'manual') {
        $fixtureName = $target.install_command.Substring('fixture:'.Length)
        Copy-ManualFixture -FixtureName $fixtureName -AddonsDir $layout.AddonsDir
        $results.Add((Get-CompactResultLine -Name $target.selector -Stage 'seed' -Status 'ok' -Detail 'manual fixture copied'))
        continue
    }

    $command = Split-ManifestCommand -CommandText $target.install_command
    $result = Invoke-Lemonup -Profile $Profile -AddonDir $layout.AddonsDir -Arguments $command
    Assert-ProcessSucceeded -Result $result -Context "seed install $($target.selector)"

    $skippedLine = $result.Output | Where-Object { $_ -like 'install skipped:*' } | Select-Object -Last 1
    if (-not [string]::IsNullOrWhiteSpace($skippedLine)) {
        throw "seed install for $($target.selector) was skipped: $skippedLine"
    }

    $installLine = $result.Output |
        Where-Object { $_ -match '^(wago|tukui|wowinterface|github) install:' } |
        Select-Object -Last 1
    if ([string]::IsNullOrWhiteSpace($installLine)) {
        $rendered = $result.Output -join [Environment]::NewLine
        throw "seed install for $($target.selector) did not produce an install summary`n$rendered"
    }

    $parsed = Get-OutputLineMap -Line $installLine
    if ($parsed['parent'] -ne $target.expected_parent) {
        throw "seed install for $($target.selector) produced unexpected parent '$($parsed['parent'])' (expected '$($target.expected_parent)')"
    }

    $results.Add((Get-CompactResultLine -Name $target.selector -Stage 'seed' -Status 'ok' -Detail "parent=$($parsed['parent'])"))
}

$sync = Invoke-Lemonup -Profile $Profile -AddonDir $layout.AddonsDir -Arguments @('sync')
Assert-ProcessSucceeded -Result $sync -Context 'seed sync'
$syncLine = $sync.Output | Where-Object { $_ -like 'sync summary:*' } | Select-Object -Last 1
if ([string]::IsNullOrWhiteSpace($syncLine)) {
    throw 'seed sync did not produce a sync summary'
}
$syncParsed = Get-OutputLineMap -Line $syncLine
$results.Add((Get-CompactResultLine -Name 'state' -Stage 'sync' -Status 'ok' -Detail "scanned=$($syncParsed['scanned_addons']) upserted=$($syncParsed['upserted_addons'])"))

Write-Host "manual seed results:"
foreach ($line in $results) {
    Write-Host "  $line"
}

$selectors = ($matrix | ForEach-Object { $_.selector }) -join ', '
Write-Host ""
Write-Host "seeded selectors: $selectors"
Write-Host "next smoke:"
Write-Host "  pwsh -File `"$PSScriptRoot\smoke-provider-matrix.ps1`" -SandboxRoot `"$SandboxRoot`" -Profile `"$Profile`""
Write-Host "next tui:"
$binary = Get-LemonupBinaryPath
Write-Host "  pwsh -Command `"& '$binary' --profile $Profile --addon-dir '$($layout.AddonsDir)' tui`""
