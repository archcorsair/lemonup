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
    # Without a key the install is refused, which is a failed run: name the cause first.
    if ($preflight.Output -match 'Add your Wago\.io key') {
        throw "seed preflight failed for $($target.selector): no Wago API key configured"
    }
    # A sandbox seeded before still has the addon, so its dry run is refused. The reset below
    # removes it; anything else that fails is worth knowing before the sandbox is wiped.
    # Output is a list of lines: -match keeps the lines that match, so no match is an empty list.
    $alreadyThere = $preflight.Output -match 'is already installed|already in your AddOns folder'
    if ($preflight.ExitCode -eq 0 -or -not $alreadyThere) {
        Assert-ProcessSucceeded -Result $preflight -Context "preflight $($target.selector)"
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
    $result = Invoke-Lemonup -Profile $Profile -AddonDir $layout.AddonsDir -Arguments ($command + '--json')
    Assert-ProcessSucceeded -Result $result -Context "seed install $($target.selector)"

    $installed = (Get-JsonDocument -Result $result -Context "seed install $($target.selector)").addon
    if ($installed.parent_folder -ne $target.expected_parent) {
        throw "seed install for $($target.selector) produced unexpected parent '$($installed.parent_folder)' (expected '$($target.expected_parent)')"
    }

    $results.Add((Get-CompactResultLine -Name $target.selector -Stage 'seed' -Status 'ok' -Detail "parent=$($installed.parent_folder)"))
}

$sync = Invoke-Lemonup -Profile $Profile -AddonDir $layout.AddonsDir -Arguments @('sync', '--json')
Assert-ProcessSucceeded -Result $sync -Context 'seed sync'
$syncSummary = (Get-JsonDocument -Result $sync -Context 'seed sync').summary
$results.Add((Get-CompactResultLine -Name 'state' -Stage 'sync' -Status 'ok' -Detail "scanned=$($syncSummary.scanned_addons) upserted=$($syncSummary.upserted_addons)"))

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
