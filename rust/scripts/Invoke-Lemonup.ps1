Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$script:RustRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$script:ManualFixtureRoot = Join-Path $script:RustRoot 'testdata\manual-addons'
$script:LemonupBinaryEnsured = $false

function Get-ManualTestMatrixPath {
    Join-Path $PSScriptRoot 'manual-test-matrix.toml'
}

function Split-ManifestCommand {
    param(
        [Parameter(Mandatory = $true)]
        [string]$CommandText
    )

    $matches = [regex]::Matches($CommandText, '"[^"]*"|\S+')
    if ($matches.Count -eq 0) {
        throw "manifest command is empty"
    }

    return @($matches | ForEach-Object {
        $value = $_.Value
        if ($value.Length -ge 2 -and $value.StartsWith('"') -and $value.EndsWith('"')) {
            $value.Substring(1, $value.Length - 2)
        } else {
            $value
        }
    })
}

function Read-ManualTestMatrix {
    param(
        [string]$Path = (Get-ManualTestMatrixPath)
    )

    if (-not (Test-Path -LiteralPath $Path)) {
        throw "manual test matrix not found: $Path"
    }

    $targets = New-Object System.Collections.Generic.List[object]
    $current = $null

    foreach ($line in Get-Content -LiteralPath $Path) {
        $trimmed = $line.Trim()
        if ([string]::IsNullOrWhiteSpace($trimmed) -or $trimmed.StartsWith('#')) {
            continue
        }

        if ($trimmed -eq '[[targets]]') {
            if ($null -ne $current) {
                $targets.Add([pscustomobject]$current)
            }
            $current = [ordered]@{}
            continue
        }

        if ($null -eq $current) {
            throw "manual test matrix key found before [[targets]] section: $trimmed"
        }

        if ($trimmed -match '^(?<key>[A-Za-z0-9_]+)\s*=\s*"(?<value>.*)"$') {
            $current[$matches.key] = $matches.value
            continue
        }

        if ($trimmed -match '^(?<key>[A-Za-z0-9_]+)\s*=\s*(?<value>true|false)$') {
            $current[$matches.key] = [System.Convert]::ToBoolean($matches.value)
            continue
        }

        throw "unsupported manual test matrix line: $trimmed"
    }

    if ($null -ne $current) {
        $targets.Add([pscustomobject]$current)
    }

    if ($targets.Count -eq 0) {
        throw "manual test matrix is empty"
    }

    foreach ($target in $targets) {
        foreach ($required in 'provider', 'install_command', 'selector', 'expected_source', 'expected_parent', 'requires_api_key') {
            if (-not ($target.PSObject.Properties.Name -contains $required)) {
                throw "manual test matrix target is missing '$required'"
            }
        }
    }

    return $targets.ToArray()
}

function Get-LemonupProfilePaths {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Profile
    )

    $appData = Join-Path $env:APPDATA 'archcorsair\lemonup'
    $localData = Join-Path $env:LOCALAPPDATA 'archcorsair\lemonup'

    if ($Profile -eq 'default') {
        return [pscustomobject]@{
            ConfigDir = Join-Path $appData 'config'
            DataDir   = Join-Path $appData 'data'
            CacheDir  = Join-Path $localData 'cache'
        }
    }

    return [pscustomobject]@{
        ConfigDir = Join-Path $appData "config\profiles\$Profile"
        DataDir   = Join-Path $appData "data\profiles\$Profile"
        CacheDir  = Join-Path $localData "cache\profiles\$Profile"
    }
}

function Reset-LemonupProfileState {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Profile
    )

    $paths = Get-LemonupProfilePaths -Profile $Profile
    foreach ($path in @($paths.ConfigDir, $paths.DataDir, $paths.CacheDir)) {
        if (Test-Path -LiteralPath $path) {
            Remove-Item -LiteralPath $path -Recurse -Force
        }
    }
}

function Get-WowSandboxLayout {
    param(
        [Parameter(Mandatory = $true)]
        [string]$SandboxRoot
    )

    $resolved = [System.IO.Path]::GetFullPath($SandboxRoot)
    [pscustomobject]@{
        RootDir      = $resolved
        RetailDir    = Join-Path $resolved '_retail_'
        AddonsDir    = Join-Path $resolved '_retail_\Interface\AddOns'
        RootDataDir  = Join-Path $resolved 'Data'
        WtfDir       = Join-Path $resolved 'WTF'
        BuildInfo    = Join-Path $resolved '.build.info'
        RetailExe    = Join-Path $resolved '_retail_\Wow.exe'
    }
}

function Initialize-WowSandboxLayout {
    param(
        [Parameter(Mandatory = $true)]
        [string]$SandboxRoot,
        [switch]$Reset
    )

    $layout = Get-WowSandboxLayout -SandboxRoot $SandboxRoot
    if ($Reset -and (Test-Path -LiteralPath $layout.RootDir)) {
        Remove-Item -LiteralPath $layout.RootDir -Recurse -Force
    }

    New-Item -ItemType Directory -Force -Path $layout.AddonsDir | Out-Null
    New-Item -ItemType Directory -Force -Path $layout.RootDataDir | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $layout.WtfDir 'Account\ManualSmoke') | Out-Null
    if (-not (Test-Path -LiteralPath $layout.BuildInfo)) {
        New-Item -ItemType File -Force -Path $layout.BuildInfo | Out-Null
    }
    if (-not (Test-Path -LiteralPath $layout.RetailExe)) {
        New-Item -ItemType File -Force -Path $layout.RetailExe | Out-Null
    }
    if (-not (Test-Path -LiteralPath (Join-Path $layout.WtfDir 'Config.wtf'))) {
        Set-Content -LiteralPath (Join-Path $layout.WtfDir 'Config.wtf') -Value 'SET accountName "ManualSmoke"' -NoNewline
    }
    if (-not (Test-Path -LiteralPath (Join-Path $layout.WtfDir 'Account\ManualSmoke\bindings-cache.wtf'))) {
        Set-Content -LiteralPath (Join-Path $layout.WtfDir 'Account\ManualSmoke\bindings-cache.wtf') -Value 'bindings-cache' -NoNewline
    }

    return $layout
}

function Resolve-CargoInvocation {
    $mise = Get-Command mise -ErrorAction SilentlyContinue
    if ($null -ne $mise) {
        try {
            $cargoPath = & $mise.Source which cargo 2>$null
            if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($cargoPath) -and -not $cargoPath.StartsWith('mise ERROR')) {
                return [pscustomobject]@{
                    Executable = $cargoPath.Trim()
                    Prefix     = @()
                }
            }
        } catch {
        }
    }

    $cargoShim = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
    if (-not (Test-Path -LiteralPath $cargoShim)) {
        throw "cargo shim not found at $cargoShim"
    }

    $toolchainRoot = Join-Path $env:USERPROFILE '.rustup\toolchains'
    $toolchain = Get-ChildItem -LiteralPath $toolchainRoot -Directory |
        Sort-Object Name -Descending |
        Select-Object -First 1

    if ($null -eq $toolchain) {
        throw "no rustup toolchains found under $toolchainRoot"
    }

    return [pscustomobject]@{
        Executable = $cargoShim
        Prefix     = @("+$($toolchain.Name)")
    }
}

function Build-LemonupBinary {
    $cargo = Resolve-CargoInvocation
    $arguments = @()
    $arguments += $cargo.Prefix
    $arguments += @('build', '-p', 'lemonup-app', '--bin', 'lemonup')

    Push-Location $script:RustRoot
    try {
        & $cargo.Executable @arguments
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build failed with exit code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
    }
}

function Get-LemonupBinaryPath {
    $binary = Join-Path $script:RustRoot 'target\debug\lemonup.exe'
    if (-not (Test-Path -LiteralPath $binary)) {
        Build-LemonupBinary
    }
    return $binary
}

function Ensure-LemonupBinary {
    if (-not $script:LemonupBinaryEnsured) {
        Build-LemonupBinary
        $script:LemonupBinaryEnsured = $true
    }

    return Get-LemonupBinaryPath
}

function Invoke-Lemonup {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments,
        [string]$Profile = 'manual-smoke',
        [string]$AddonDir
    )

    $binary = Get-LemonupBinaryPath
    $invocation = @('--profile', $Profile)
    if (-not [string]::IsNullOrWhiteSpace($AddonDir)) {
        $invocation += @('--addon-dir', $AddonDir)
    }
    $invocation += $Arguments

    $output = & $binary @invocation 2>&1 | ForEach-Object { $_.ToString() }
    $exitCode = $LASTEXITCODE

    [pscustomobject]@{
        Command  = "$binary $($invocation -join ' ')"
        ExitCode = $exitCode
        Output   = @($output)
    }
}

function Get-OutputLineMap {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Line
    )

    $separator = $Line.IndexOf(':')
    if ($separator -lt 0) {
        throw "line does not contain a ':' separator: $Line"
    }

    $body = $Line.Substring($separator + 1).Trim()
    $map = [ordered]@{}
    foreach ($segment in $body -split ',\s+') {
        $pair = $segment.Split('=', 2)
        if ($pair.Count -ne 2) {
            continue
        }
        $map[$pair[0].Trim()] = $pair[1].Trim()
    }

    return $map
}

function Assert-ProcessSucceeded {
    param(
        [Parameter(Mandatory = $true)]
        $Result,
        [Parameter(Mandatory = $true)]
        [string]$Context
    )

    if ($Result.ExitCode -ne 0) {
        $rendered = $Result.Output -join [Environment]::NewLine
        throw "$Context failed (exit $($Result.ExitCode))`n$rendered"
    }
}

function Copy-ManualFixture {
    param(
        [Parameter(Mandatory = $true)]
        [string]$FixtureName,
        [Parameter(Mandatory = $true)]
        [string]$AddonsDir
    )

    $source = Join-Path $script:ManualFixtureRoot $FixtureName
    if (-not (Test-Path -LiteralPath $source)) {
        throw "manual fixture not found: $source"
    }

    $destination = Join-Path $AddonsDir $FixtureName
    if (Test-Path -LiteralPath $destination) {
        Remove-Item -LiteralPath $destination -Recurse -Force
    }
    Copy-Item -LiteralPath $source -Destination $destination -Recurse -Force
}

function Get-CompactResultLine {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [Parameter(Mandatory = $true)]
        [string]$Stage,
        [Parameter(Mandatory = $true)]
        [string]$Status,
        [string]$Detail = ''
    )

    if ([string]::IsNullOrWhiteSpace($Detail)) {
        return "$Name | $Stage | $Status"
    }

    return "$Name | $Stage | $Status | $Detail"
}
