# Made by ycangignacy
<#
.SYNOPSIS
    Builds the release binary and every download into dist\.

.DESCRIPTION
    Produces, for one architecture:
      WindowCraft-<version>-<arch>-setup.exe      Installer with a simple UI
      WindowCraft-<version>-<arch>.msi            For Intune, GPO and unattended installs
      WindowCraft-<version>-<arch>-portable.exe   Runs from anywhere, installs nothing
      SHA256SUMS.txt

    WiX is unpacked into target\tools and nothing is installed on the machine.
#>
[CmdletBinding()]
param(
    [ValidateSet('x64', 'arm64')]
    [string] $Architecture = 'x64',
    [string] $ProjectUrl = 'https://github.com/ycangignacy/WindowCraft',
    [switch] $SkipBuild
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$wixUrl = 'https://github.com/wixtoolset/wix3/releases/download/wix3141rtm/wix314-binaries.zip'
$wixSha256 = '6AC824E1642D6F7277D0ED7EA09411A508F6116BA6FAE0AA5F2C7DAA2FF43D31'

$manifest = Get-Content 'Cargo.toml' -Raw
if ($manifest -notmatch '(?m)^version = "(\d+\.\d+\.\d+)"') { throw 'No workspace version in Cargo.toml.' }
$version = $Matches[1]

$cargoArguments = @('build', '--release', '--locked', '-p', 'windowcraft-app', '-p', 'windowcraft-cli')
$outputDir = Join-Path $root 'target\release'
if ($Architecture -eq 'arm64') {
    $cargoArguments += @('--target', 'aarch64-pc-windows-msvc')
    $outputDir = Join-Path $root 'target\aarch64-pc-windows-msvc\release'
    # ring compiles its C sources with clang on this target.
    if (-not (Get-Command clang -ErrorAction SilentlyContinue)) {
        $env:Path = "$root\target\tools\llvm\bin;$env:ProgramFiles\LLVM\bin;$env:Path"
    }
}
$binary = Join-Path $outputDir 'windowcraft.exe'
$cliBinary = Join-Path $outputDir 'windowcraft-cli.exe'

if (-not $SkipBuild) {
    Write-Host "Building windowcraft.exe for $Architecture"
    & cargo @cargoArguments
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed.' }
}
foreach ($file in $binary, $cliBinary) {
    if (-not (Test-Path $file)) { throw "Missing $file" }
}

# The executable must run on a clean Windows install.
$dumpbin = Get-ChildItem "${env:ProgramFiles(x86)}\Microsoft Visual Studio", "$env:ProgramFiles\Microsoft Visual Studio" `
    -Recurse -Filter dumpbin.exe -ErrorAction SilentlyContinue | Where-Object FullName -Match 'Hostx64\\x64' | Select-Object -First 1
if ($dumpbin) {
    $imports = & $dumpbin.FullName /dependents $binary
    if ($imports -match 'VCRUNTIME|api-ms-win-crt|MSVCP') { throw 'The binary depends on the C runtime. Check .cargo/config.toml.' }
}

$tools = Join-Path $root 'target\tools'
$wix = Join-Path $tools 'wix314'
if (-not (Test-Path (Join-Path $wix 'candle.exe'))) {
    New-Item -ItemType Directory -Force $tools | Out-Null
    $archive = Join-Path $tools 'wix314-binaries.zip'
    if (-not (Test-Path $archive)) { Invoke-WebRequest -UseBasicParsing -Uri $wixUrl -OutFile $archive }
    $actual = (Get-FileHash $archive -Algorithm SHA256).Hash
    if ($actual -ne $wixSha256) { throw "WiX archive hash mismatch: $actual" }
    Expand-Archive -Path $archive -DestinationPath $wix -Force
}

$work = Join-Path $root 'target\package'
$dist = Join-Path $root 'dist'
Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $work, $dist | Out-Null

# WixUI needs the license as RTF.
$license = (Get-Content (Join-Path $root 'LICENSE') -Raw).Replace('\', '\\').Replace('{', '\{').Replace('}', '\}')
$body = ($license -replace "`r?`n`r?`n", '\par\par ') -replace "`r?`n", ' '
$licenseRtf = Join-Path $work 'License.rtf'
"{\rtf1\ansi\deff0{\fonttbl{\f0 Segoe UI;}}\fs18 $body}" | Set-Content -Path $licenseRtf -Encoding ascii

$platform = if ($Architecture -eq 'arm64') { 'arm64' } else { 'x64' }
$defines = @(
    "-dVersion=$version"
    "-dPlatform=$platform"
    '-dWin64=yes'
    '-dProgramFilesFolder=ProgramFiles64Folder'
    "-dBinary=$binary"
    "-dCliBinary=$cliBinary"
    "-dRoot=$root"
    "-dAssets=$(Join-Path $root 'crates\windowcraft-app\assets')"
    "-dLicenseRtf=$licenseRtf"
    "-dProjectUrl=$ProjectUrl"
)

function Invoke-Wix([string] $Tool, [string[]] $Arguments) {
    & (Join-Path $wix $Tool) -nologo @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Tool failed with exit code $LASTEXITCODE." }
}

$stem = "WindowCraft-$version-$Architecture"
$msi = Join-Path $dist "$stem.msi"
$setup = Join-Path $dist "$stem-setup.exe"
$portable = Join-Path $dist "$stem-portable.exe"

Write-Host 'Building MSI'
Invoke-Wix 'candle.exe' (@('-arch', $platform, '-ext', 'WixUtilExtension', '-ext', 'WixUIExtension') + $defines + `
    @('-out', (Join-Path $work 'windowcraft.wixobj'), (Join-Path $PSScriptRoot 'windowcraft.wxs')))
# ICE43 and ICE57 expect a per-user package for a plain Start menu shortcut. This one is
# per-machine (ALLUSERS=1), so the shortcut goes to the all-users Start menu and the
# executable is the right key path.
Invoke-Wix 'light.exe' @('-ext', 'WixUtilExtension', '-ext', 'WixUIExtension', '-spdb', '-sice:ICE61', '-sice:ICE43', '-sice:ICE57',
    '-out', $msi, (Join-Path $work 'windowcraft.wixobj'))

Write-Host 'Building setup.exe'
Invoke-Wix 'candle.exe' (@('-ext', 'WixBalExtension', "-dMsi=$msi") + $defines + `
    @('-out', (Join-Path $work 'bundle.wixobj'), (Join-Path $PSScriptRoot 'bundle.wxs')))
Invoke-Wix 'light.exe' @('-ext', 'WixBalExtension', '-spdb', '-out', $setup, (Join-Path $work 'bundle.wixobj'))

Copy-Item $binary $portable -Force
Copy-Item (Join-Path $root 'THIRD-PARTY-NOTICES.md') $dist -Force

Push-Location $dist
try {
    Get-ChildItem -File -Filter "WindowCraft-$version-*" | ForEach-Object {
        '{0}  {1}' -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant(), $_.Name
    } | Set-Content -Path 'SHA256SUMS.txt' -Encoding ascii
} finally {
    Pop-Location
}

Get-ChildItem $dist | Select-Object Name, @{ n = 'MB'; e = { [math]::Round($_.Length / 1MB, 2) } } | Format-Table -AutoSize
