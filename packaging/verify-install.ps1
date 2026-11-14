# Made by Nordan
<#
.SYNOPSIS
    Installs each x64 package from dist\, checks the result, uninstalls it and
    checks that nothing is left.

.DESCRIPTION
    Run from an elevated PowerShell. It refuses to run if WindowCraft is already
    installed, so it never removes an install it did not create. Uninstall wipes
    %LOCALAPPDATA%\WindowCraft, so that folder is moved aside first and restored
    at the end, and a copy stays in target\package\data-backup. WindowCraft itself only runs with --version and --export-report,
    so no tweak or system change is applied.

    Start it from a normal elevated PowerShell. A shell started by an MSIX
    packaged app sees a virtualized %LOCALAPPDATA%, which uninstall cannot reach.

    The result goes to target\package\verify-install.log. The exit code is the
    number of failed checks.
#>
[CmdletBinding()]
param(
    [string] $Version,
    [int] $Rounds = 1
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$dist = Join-Path $root 'dist'

if (-not $Version) {
    $manifest = Get-Content (Join-Path $root 'Cargo.toml') -Raw
    if ($manifest -notmatch '(?m)^version = "(\d+\.\d+\.\d+)"') { throw 'No workspace version in Cargo.toml.' }
    $Version = $Matches[1]
}

$identity = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $identity.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Run this from an elevated PowerShell.'
}

$msi = Join-Path $dist "WindowCraft-$Version-x64.msi"
$setup = Join-Path $dist "WindowCraft-$Version-x64-setup.exe"
foreach ($file in $msi, $setup) {
    if (-not (Test-Path $file)) { throw "Missing $file" }
}

$package = Join-Path $root 'target\package'
New-Item -ItemType Directory -Force $package | Out-Null
$log = Join-Path $package 'verify-install.log'
Set-Content -Path $log -Value "WindowCraft $Version install check, $(Get-Date -Format s)" -Encoding utf8

$folder = Join-Path $env:ProgramFiles 'WindowCraft'
$exe = Join-Path $folder 'windowcraft.exe'
$shortcut = Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs\WindowCraft.lnk'
$data = Join-Path $env:LOCALAPPDATA 'WindowCraft'
$kept = "$data.verify-backup"
$regKey = 'HKLM:\SOFTWARE\WindowCraft'
$script:failures = 0

function Write-Result([string] $Line) {
    Write-Host $Line
    Add-Content -Path $log -Value $Line -Encoding utf8
}

function Test-Check([string] $Name, [bool] $Passed) {
    if (-not $Passed) { $script:failures++ }
    Write-Result ('{0}  {1}' -f $(if ($Passed) { 'PASS' } else { 'FAIL' }), $Name)
}

function Get-MachinePath {
    [Environment]::GetEnvironmentVariable('Path', 'Machine') -split ';' | Where-Object { $_ }
}

function Get-UninstallEntry {
    $keys = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*',
            'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*'
    Get-ItemProperty $keys -ErrorAction SilentlyContinue | Where-Object DisplayName -EQ 'WindowCraft'
}

function Test-OnPath {
    @(Get-MachinePath | Where-Object { $_.TrimEnd('\') -eq $folder }).Count -gt 0
}

function Test-Installed([string] $Name, [int] $ExitCode) {
    Test-Check "$Name exit code is 0 or 3010 (got $ExitCode)" ($ExitCode -in 0, 3010)
    Test-Check "$Name placed windowcraft.exe" (Test-Path $exe)
    Test-Check "$Name placed LICENSE.txt and NOTICE.md" ((Test-Path "$folder\LICENSE.txt") -and (Test-Path "$folder\NOTICE.md"))
    Test-Check "$Name created the Start menu entry" (Test-Path $shortcut)
    Test-Check "$Name added the folder to the machine PATH" (Test-OnPath)
    Test-Check "$Name registered an uninstall entry" ($null -ne (Get-UninstallEntry))
    $cli = Join-Path $folder 'windowcraft-cli.exe'
    Test-Check "$Name placed windowcraft-cli.exe" (Test-Path $cli)
    if (Test-Path $cli) {
        $reported = & $cli --version 2>&1 | Out-String
        Test-Check "$Name binary reports $Version" ($reported -match [regex]::Escape($Version))
    }
}

function New-UserData([string] $Name) {
    # A real run, so the snapshot database is the one users would have.
    & $exe --export-report (Join-Path $package 'report.json') | Out-Null
    Test-Check "$Name created the snapshot database" (Test-Path (Join-Path $data 'timemachine.redb'))
    $media = Join-Path $data 'iso\media'
    New-Item -ItemType Directory -Force $media | Out-Null
    $image = Join-Path $media 'install.wim'
    Set-Content -Path $image -Value 'image' -Force
    Set-ItemProperty -Path $image -Name IsReadOnly -Value $true
}

# KB-9: uninstall must close an open window instead of leaving it running.
function Start-OpenWindow([string] $Name) {
    $process = Start-Process -FilePath $exe -PassThru
    $deadline = (Get-Date).AddSeconds(15)
    while ($process.MainWindowHandle -eq 0 -and -not $process.HasExited -and (Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 200
        $process.Refresh()
    }
    Test-Check "$Name opened a WindowCraft window before uninstall" ($process.MainWindowHandle -ne 0)
    $process
}

function Test-WindowClosed([string] $Name, $Process) {
    $Process.Refresh()
    Test-Check "$Name uninstall closed the open window" $Process.HasExited
    if (-not $Process.HasExited) { $Process | Stop-Process -Force }
}

function Get-Leftover {
    $installer = Get-ItemProperty 'HKLM:\SOFTWARE\Classes\Installer\Products\*' -ErrorAction SilentlyContinue |
        Where-Object ProductName -EQ 'WindowCraft' | ForEach-Object { $_.PSPath }
    $files = @(
        Get-ChildItem (Join-Path $env:ProgramData 'Package Cache') -Recurse -Force -Filter 'WindowCraft*' -ErrorAction SilentlyContinue
        Get-ChildItem (Join-Path $env:windir 'Installer') -Recurse -Force -Filter 'WindowCraft*' -ErrorAction SilentlyContinue
        Get-ChildItem $env:TEMP -Force -Filter 'WindowCraft*' -ErrorAction SilentlyContinue
        Get-ChildItem $env:LOCALAPPDATA -Force -Filter 'WindowCraft*' -ErrorAction SilentlyContinue | Where-Object FullName -NE $kept
    ) | ForEach-Object { $_.FullName }
    $tasks = Get-ScheduledTask -ErrorAction SilentlyContinue | Where-Object { $_.TaskName -match 'WindowCraft' } | ForEach-Object { $_.TaskName }
    $services = Get-Service -ErrorAction SilentlyContinue | Where-Object { $_.Name -match 'WindowCraft' } | ForEach-Object { $_.Name }
    $reg = if (Test-Path $regKey) { $regKey } else { $null }
    @($installer) + @($files) + @($tasks) + @($services) + @($reg) | Where-Object { $_ }
}

function Test-Removed([string] $Name, [int] $ExitCode) {
    Test-Check "$Name uninstall exit code is 0 or 3010 (got $ExitCode)" ($ExitCode -in 0, 3010)
    Test-Check "$Name removed the program folder" (-not (Test-Path $folder))
    Test-Check "$Name removed the Start menu entry" (-not (Test-Path $shortcut))
    Test-Check "$Name removed the PATH entry" (-not (Test-OnPath))
    Test-Check "$Name removed the uninstall entry" ($null -eq (Get-UninstallEntry))
    Test-Check "$Name removed the registry key" (-not (Test-Path $regKey))
    Test-Check "$Name removed the user data folder" (-not (Test-Path $data))
    if (Test-Path $data) {
        Get-ChildItem $data -Recurse -Force | ForEach-Object { Write-Result "      still there: $($_.FullName)" }
    }
    $leftover = @(Get-Leftover)
    $leftover | ForEach-Object { Write-Result "      left behind: $_" }
    Test-Check "$Name left nothing else behind" ($leftover.Count -eq 0)
}

function Get-DataListing([string] $Path) {
    Get-ChildItem $Path -Recurse -Force -File -ErrorAction SilentlyContinue |
        ForEach-Object { '{0}|{1}' -f $_.FullName.Substring($Path.Length), $_.Length } | Sort-Object
}

function Invoke-Installer([string] $File, [string[]] $Arguments) {
    (Start-Process -FilePath $File -ArgumentList $Arguments -Wait -PassThru).ExitCode
}

if ((Get-UninstallEntry) -or (Test-Path $folder) -or (Test-Path $regKey)) {
    throw 'WindowCraft is already installed. Uninstall it first.'
}
if (Test-Path $kept) { throw "$kept exists from an interrupted run. Restore or delete it first." }

$pathBefore = (Get-MachinePath) -join ';'
$dataBefore = @(Get-DataListing $data)
if (Test-Path $data) {
    $backup = Join-Path $package 'data-backup'
    Remove-Item -LiteralPath $backup -Recurse -Force -ErrorAction SilentlyContinue
    Copy-Item -LiteralPath $data -Destination $backup -Recurse
    Rename-Item -LiteralPath $data -NewName (Split-Path $kept -Leaf)
}

try {
    foreach ($round in 1..$Rounds) {
        Write-Result "--- MSI, round $round"
        $msiLog = Join-Path $package "msi-install-$round.log"
        $code = Invoke-Installer 'msiexec.exe' @('/i', "`"$msi`"", '/qn', '/norestart', '/l*v', "`"$msiLog`"")
        Test-Installed 'MSI' $code
        New-UserData 'MSI'
        $open = Start-OpenWindow 'MSI'
        $code = Invoke-Installer 'msiexec.exe' @('/x', "`"$msi`"", '/qn', '/norestart', '/l*v', "`"$(Join-Path $package "msi-uninstall-$round.log")`"")
        Test-WindowClosed 'MSI' $open
        Test-Removed 'MSI' $code
        Remove-Item -LiteralPath $data -Recurse -Force -ErrorAction SilentlyContinue

        Write-Result "--- MSI with WCLANGUAGE=pl, round $round"
        $msiLangLog = Join-Path $package "msi-install-lang-$round.log"
        $code = Invoke-Installer 'msiexec.exe' @('/i', "`"$msi`"", 'WCLANGUAGE=pl', '/qn', '/norestart', '/l*v', "`"$msiLangLog`"")
        Test-Installed 'MSI with WCLANGUAGE=pl' $code
        $languageValue = (Get-ItemProperty $regKey -ErrorAction SilentlyContinue).Language
        Test-Check "MSI with WCLANGUAGE=pl wrote Language=pl to registry (got '$languageValue')" ($languageValue -eq 'pl')
        $code = Invoke-Installer 'msiexec.exe' @('/x', "`"$msi`"", '/qn', '/norestart', '/l*v', "`"$(Join-Path $package "msi-uninstall-lang-$round.log")`"")
        Test-Removed 'MSI with WCLANGUAGE=pl' $code
        Remove-Item -LiteralPath $data -Recurse -Force -ErrorAction SilentlyContinue

        # No /log here: the check below proves a normal run writes none to %TEMP%.
        Write-Result "--- setup.exe, round $round"
        $code = Invoke-Installer $setup @('/quiet', '/norestart')
        Test-Installed 'setup.exe' $code
        New-UserData 'setup.exe'
        $open = Start-OpenWindow 'setup.exe'
        $code = Invoke-Installer $setup @('/uninstall', '/quiet', '/norestart', '/log', "`"$(Join-Path $package "setup-uninstall-$round.log")`"")
        Test-WindowClosed 'setup.exe' $open
        Test-Removed 'setup.exe' $code
        Remove-Item -LiteralPath $data -Recurse -Force -ErrorAction SilentlyContinue
    }

    Test-Check 'Machine PATH is unchanged afterwards' (((Get-MachinePath) -join ';') -eq $pathBefore)
} finally {
    # An interrupted run must not leave the product installed.
    if (Get-UninstallEntry) {
        Write-Result 'Run was interrupted: uninstalling what it installed.'
        $null = Invoke-Installer $setup @('/uninstall', '/quiet', '/norestart')
        $null = Invoke-Installer 'msiexec.exe' @('/x', "`"$msi`"", '/qn', '/norestart')
    }
    if (Test-Path $regKey) {
        Remove-Item -Path $regKey -Recurse -Force -ErrorAction SilentlyContinue
    }
    if (Test-Path $kept) {
        Remove-Item -LiteralPath $data -Recurse -Force -ErrorAction SilentlyContinue
        Rename-Item -LiteralPath $kept -NewName (Split-Path $data -Leaf)
        Write-Result 'Restored the existing data folder.'
    }
    $difference = @(Compare-Object $dataBefore @(Get-DataListing $data) | Where-Object InputObject -NotMatch 'timemachine.redb')
    $difference | ForEach-Object { Write-Result "      data changed: $($_.SideIndicator) $($_.InputObject)" }
    Test-Check 'Existing data folder came back complete' ($difference.Count -eq 0)
}

Write-Result "Failed checks: $script:failures"
exit $script:failures
