param(
    [Parameter(Mandatory = $true)]
    [string]$SandboxRoot,
    [string]$Profile = 'manual-smoke'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'Invoke-Lemonup.ps1')

Ensure-LemonupBinary | Out-Null
$layout = Get-WowSandboxLayout -SandboxRoot $SandboxRoot
if (-not (Test-Path -LiteralPath $layout.AddonsDir)) {
    throw "sandbox AddOns directory does not exist: $($layout.AddonsDir)"
}

$matrix = Read-ManualTestMatrix

# The JSON document a `--json` run printed on stdout. Invoke-Lemonup merges stderr into Output,
# so the document is the line that starts with '{'.
function Get-JsonDocument {
    param(
        [Parameter(Mandatory = $true)]
        $Result,
        [Parameter(Mandatory = $true)]
        [string]$Context
    )

    $line = $Result.Output | Where-Object { $_ -like '{*' } | Select-Object -Last 1
    if ([string]::IsNullOrWhiteSpace($line)) {
        throw "$Context did not print a JSON document"
    }
    return $line | ConvertFrom-Json
}
$results = New-Object System.Collections.Generic.List[string]

$sync = Invoke-Lemonup -Profile $Profile -AddonDir $layout.AddonsDir -Arguments @('sync')
Assert-ProcessSucceeded -Result $sync -Context 'pre-smoke sync'
$results.Add((Get-CompactResultLine -Name 'state' -Stage 'sync' -Status 'ok'))

foreach ($target in $matrix) {
    $check = Invoke-Lemonup -Profile $Profile -AddonDir $layout.AddonsDir -Arguments @('check', $target.selector, '--json')
    Assert-ProcessSucceeded -Result $check -Context "check $($target.selector)"
    $checkDocument = Get-JsonDocument -Result $check -Context "check $($target.selector)"
    $checkResult = @($checkDocument.addons) | Select-Object -Last 1
    if ($null -eq $checkResult) {
        throw "check $($target.selector) did not report the addon"
    }
    if ($checkDocument.summary.errors -ne 0) {
        throw "check $($target.selector) reported errors=$($checkDocument.summary.errors)"
    }
    if ($target.provider -eq 'manual') {
        if ($checkResult.status -ne 'unknown') {
            throw "manual check for $($target.selector) expected status=unknown, got $($checkResult.status)"
        }
    } elseif ($checkResult.status -notin @('up_to_date', 'update_available')) {
        throw "check $($target.selector) returned unexpected status=$($checkResult.status)"
    }
    $results.Add((Get-CompactResultLine -Name $target.selector -Stage 'check' -Status 'ok' -Detail "status=$($checkResult.status)"))

    $update = Invoke-Lemonup -Profile $Profile -AddonDir $layout.AddonsDir -Arguments @('update', $target.selector, '--dry-run', '--json')
    Assert-ProcessSucceeded -Result $update -Context "update dry-run $($target.selector)"
    $updateDocument = Get-JsonDocument -Result $update -Context "update dry-run $($target.selector)"
    $updateResult = @($updateDocument.addons) | Select-Object -Last 1
    if ($null -eq $updateResult) {
        throw "update dry-run $($target.selector) did not report the addon"
    }
    if ($updateDocument.summary.errors -ne 0) {
        throw "update dry-run $($target.selector) reported errors=$($updateDocument.summary.errors)"
    }
    if ($updateResult.source -ne $target.expected_source) {
        throw "update dry-run $($target.selector) expected source=$($target.expected_source), got $($updateResult.source)"
    }
    if ($target.provider -eq 'manual') {
        if ($updateResult.status -ne 'skipped_manual') {
            throw "manual update dry-run for $($target.selector) expected skipped_manual, got $($updateResult.status)"
        }
    } elseif ($updateResult.status -notin @('up_to_date', 'updated')) {
        throw "provider update dry-run for $($target.selector) returned unexpected status=$($updateResult.status)"
    }
    $results.Add((Get-CompactResultLine -Name $target.selector -Stage 'update-dry-run' -Status 'ok' -Detail "status=$($updateResult.status) source=$($updateResult.source)"))
}

$allUpdate = Invoke-Lemonup -Profile $Profile -AddonDir $layout.AddonsDir -Arguments @('update', '--dry-run', '--json')
Assert-ProcessSucceeded -Result $allUpdate -Context 'update --dry-run'
$allUpdateDocument = Get-JsonDocument -Result $allUpdate -Context 'update --dry-run'
if ([int]$allUpdateDocument.summary.tracked_addons -lt $matrix.Count) {
    throw "update --dry-run tracked_addons=$($allUpdateDocument.summary.tracked_addons) is lower than seeded target count $($matrix.Count)"
}
$results.Add((Get-CompactResultLine -Name 'all' -Stage 'update-dry-run' -Status 'ok' -Detail "tracked=$($allUpdateDocument.summary.tracked_addons)"))

$updateAll = Invoke-Lemonup -Profile $Profile -AddonDir $layout.AddonsDir -Arguments @('update-all', '--dry-run', '--json')
Assert-ProcessSucceeded -Result $updateAll -Context 'update-all --dry-run'
$updateAllDocument = Get-JsonDocument -Result $updateAll -Context 'update-all --dry-run'
if ($null -eq $updateAllDocument.summary) {
    throw 'update-all --dry-run did not produce a summary'
}
$results.Add((Get-CompactResultLine -Name 'all' -Stage 'update-all-dry-run' -Status 'ok'))

$managedSelectors = @($matrix | Where-Object { $_.provider -ne 'manual' } | ForEach-Object { $_.selector })
$mixedArgs = @('update') + $managedSelectors + @('--dry-run', '--json')
$mixed = Invoke-Lemonup -Profile $Profile -AddonDir $layout.AddonsDir -Arguments $mixedArgs
Assert-ProcessSucceeded -Result $mixed -Context 'mixed-provider update --dry-run'
$mixedResults = @((Get-JsonDocument -Result $mixed -Context 'mixed-provider update --dry-run').addons)
if ($mixedResults.Count -ne $managedSelectors.Count) {
    throw "mixed-provider update expected $($managedSelectors.Count) results, got $($mixedResults.Count)"
}
$results.Add((Get-CompactResultLine -Name 'mixed' -Stage 'update-dry-run' -Status 'ok' -Detail "targets=$($managedSelectors -join '|')"))

Write-Host "manual smoke results:"
foreach ($line in $results) {
    Write-Host "  $line"
}
