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
$results = New-Object System.Collections.Generic.List[string]

$sync = Invoke-Lemonup -Profile $Profile -AddonDir $layout.AddonsDir -Arguments @('sync')
Assert-ProcessSucceeded -Result $sync -Context 'pre-smoke sync'
$results.Add((Get-CompactResultLine -Name 'state' -Stage 'sync' -Status 'ok'))

foreach ($target in $matrix) {
    $check = Invoke-Lemonup -Profile $Profile -AddonDir $layout.AddonsDir -Arguments @('check', $target.selector)
    Assert-ProcessSucceeded -Result $check -Context "check $($target.selector)"
    $checkSummary = $check.Output | Where-Object { $_ -like 'check summary:*' } | Select-Object -Last 1
    $checkResult = $check.Output | Where-Object { $_ -like 'check result:*' } | Select-Object -Last 1
    if ([string]::IsNullOrWhiteSpace($checkSummary) -or [string]::IsNullOrWhiteSpace($checkResult)) {
        throw "check $($target.selector) did not produce expected summary/result lines"
    }
    $checkSummaryMap = Get-OutputLineMap -Line $checkSummary
    $checkResultMap = Get-OutputLineMap -Line $checkResult
    if ($checkSummaryMap['errors'] -ne '0') {
        throw "check $($target.selector) reported errors=$($checkSummaryMap['errors'])"
    }
    if ($target.provider -eq 'manual') {
        if ($checkResultMap['status'] -ne 'unknown') {
            throw "manual check for $($target.selector) expected status=unknown, got $($checkResultMap['status'])"
        }
    } elseif ($checkResultMap['status'] -notin @('up_to_date', 'update_available')) {
        throw "check $($target.selector) returned unexpected status=$($checkResultMap['status'])"
    }
    $results.Add((Get-CompactResultLine -Name $target.selector -Stage 'check' -Status 'ok' -Detail "status=$($checkResultMap['status'])"))

    $update = Invoke-Lemonup -Profile $Profile -AddonDir $layout.AddonsDir -Arguments @('update', $target.selector, '--dry-run')
    Assert-ProcessSucceeded -Result $update -Context "update dry-run $($target.selector)"
    $updateSummary = $update.Output | Where-Object { $_ -like 'update summary:*' } | Select-Object -Last 1
    $updateResult = $update.Output | Where-Object { $_ -like 'update result:*' } | Select-Object -Last 1
    if ([string]::IsNullOrWhiteSpace($updateSummary) -or [string]::IsNullOrWhiteSpace($updateResult)) {
        throw "update dry-run $($target.selector) did not produce expected summary/result lines"
    }
    $updateSummaryMap = Get-OutputLineMap -Line $updateSummary
    $updateResultMap = Get-OutputLineMap -Line $updateResult
    if ($updateSummaryMap['errors'] -ne '0') {
        throw "update dry-run $($target.selector) reported errors=$($updateSummaryMap['errors'])"
    }
    if ($updateResultMap['source'] -ne $target.expected_source) {
        throw "update dry-run $($target.selector) expected source=$($target.expected_source), got $($updateResultMap['source'])"
    }
    if ($target.provider -eq 'manual') {
        if ($updateResultMap['status'] -ne 'skipped_manual') {
            throw "manual update dry-run for $($target.selector) expected skipped_manual, got $($updateResultMap['status'])"
        }
    } elseif ($updateResultMap['status'] -notin @('up_to_date', 'updated')) {
        throw "provider update dry-run for $($target.selector) returned unexpected status=$($updateResultMap['status'])"
    }
    $results.Add((Get-CompactResultLine -Name $target.selector -Stage 'update-dry-run' -Status 'ok' -Detail "status=$($updateResultMap['status']) source=$($updateResultMap['source'])"))
}

$allUpdate = Invoke-Lemonup -Profile $Profile -AddonDir $layout.AddonsDir -Arguments @('update', '--dry-run')
Assert-ProcessSucceeded -Result $allUpdate -Context 'update --dry-run'
$allUpdateSummary = $allUpdate.Output | Where-Object { $_ -like 'update summary:*' } | Select-Object -Last 1
if ([string]::IsNullOrWhiteSpace($allUpdateSummary)) {
    throw 'update --dry-run did not produce a summary'
}
$allUpdateMap = Get-OutputLineMap -Line $allUpdateSummary
if ([int]$allUpdateMap['tracked_addons'] -lt $matrix.Count) {
    throw "update --dry-run tracked_addons=$($allUpdateMap['tracked_addons']) is lower than seeded target count $($matrix.Count)"
}
$results.Add((Get-CompactResultLine -Name 'all' -Stage 'update-dry-run' -Status 'ok' -Detail "tracked=$($allUpdateMap['tracked_addons'])"))

$updateAll = Invoke-Lemonup -Profile $Profile -AddonDir $layout.AddonsDir -Arguments @('update-all', '--dry-run')
Assert-ProcessSucceeded -Result $updateAll -Context 'update-all --dry-run'
$updateAllSummary = $updateAll.Output | Where-Object { $_ -like 'update summary:*' } | Select-Object -Last 1
if ([string]::IsNullOrWhiteSpace($updateAllSummary)) {
    throw 'update-all --dry-run did not produce a summary'
}
$results.Add((Get-CompactResultLine -Name 'all' -Stage 'update-all-dry-run' -Status 'ok'))

$managedSelectors = @($matrix | Where-Object { $_.provider -ne 'manual' } | ForEach-Object { $_.selector })
$mixedArgs = @('update') + $managedSelectors + @('--dry-run')
$mixed = Invoke-Lemonup -Profile $Profile -AddonDir $layout.AddonsDir -Arguments $mixedArgs
Assert-ProcessSucceeded -Result $mixed -Context 'mixed-provider update --dry-run'
$mixedResults = @($mixed.Output | Where-Object { $_ -like 'update result:*' })
if ($mixedResults.Count -ne $managedSelectors.Count) {
    throw "mixed-provider update expected $($managedSelectors.Count) result lines, got $($mixedResults.Count)"
}
$results.Add((Get-CompactResultLine -Name 'mixed' -Stage 'update-dry-run' -Status 'ok' -Detail "targets=$($managedSelectors -join '|')"))

Write-Host "manual smoke results:"
foreach ($line in $results) {
    Write-Host "  $line"
}
