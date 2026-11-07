# WindowCraft PowerShell profile
# Made by ycangignacy. Installed by WindowCraft from System, and removed there too,
# which puts the previous profile back. It downloads nothing and changes nothing
# outside the current PowerShell session.

# History search and suggestions
if (Get-Module -ListAvailable -Name PSReadLine) {
    Import-Module PSReadLine
    Set-PSReadLineOption -EditMode Windows -BellStyle None -HistoryNoDuplicates -HistorySearchCursorMovesToEnd
    try { Set-PSReadLineOption -PredictionSource History -PredictionViewStyle ListView } catch { }
    Set-PSReadLineKeyHandler -Key UpArrow -Function HistorySearchBackward
    Set-PSReadLineKeyHandler -Key DownArrow -Function HistorySearchForward
    Set-PSReadLineKeyHandler -Key Tab -Function MenuComplete
    # Lines that look like they carry a secret stay out of the history file.
    Set-PSReadLineOption -AddToHistoryHandler {
        param([string] $line)
        return $line -notmatch '(?i)(password|passwd|secret|token|apikey|api_key)'
    }
}

# Branch of the git repository around the current folder, read from .git\HEAD
# so the prompt never starts git.exe.
function Get-WcGitBranch {
    $dir = (Get-Location).ProviderPath
    while ($dir) {
        $head = Join-Path $dir '.git\HEAD'
        if (Test-Path -LiteralPath $head -PathType Leaf) {
            $text = (Get-Content -LiteralPath $head -TotalCount 1)
            if ($text -match '^ref: refs/heads/(.+)$') { return $Matches[1] }
            return $text.Substring(0, [Math]::Min(7, $text.Length))
        }
        $dir = Split-Path -Parent $dir
    }
}

$script:WcIsAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)

function prompt {
    $ok = $?
    $path = (Get-Location).Path
    if ($path.StartsWith($HOME, [StringComparison]::OrdinalIgnoreCase)) { $path = '~' + $path.Substring($HOME.Length) }
    $esc = [char]27
    $line = "$esc[36m$path$esc[0m"
    $branch = Get-WcGitBranch
    if ($branch) { $line += " $esc[33m($branch)$esc[0m" }
    if ($script:WcIsAdmin) { $line += " $esc[31m[admin]$esc[0m" }
    $mark = if ($ok) { "$esc[32m>$esc[0m" } else { "$esc[31m>$esc[0m" }
    "$line`n$mark "
}

# Small helpers with names known from other shells
function which([string] $Name) { (Get-Command $Name -ErrorAction Stop).Source }
function ll { Get-ChildItem -Force @args }
function touch([string] $Path) {
    if (Test-Path -LiteralPath $Path) { (Get-Item -LiteralPath $Path).LastWriteTime = Get-Date }
    else { New-Item -ItemType File -Path $Path | Out-Null }
}
function mkcd([string] $Path) { New-Item -ItemType Directory -Force -Path $Path | Out-Null; Set-Location $Path }
function grep([string] $Pattern) { $input | Select-String -Pattern $Pattern }
function df { Get-Volume | Where-Object DriveLetter | Sort-Object DriveLetter |
    Format-Table DriveLetter, FileSystemLabel, @{ n = 'Free GB'; e = { [Math]::Round($_.SizeRemaining / 1GB, 1) } },
        @{ n = 'Size GB'; e = { [Math]::Round($_.Size / 1GB, 1) } } -AutoSize }
function uptime {
    $boot = (Get-CimInstance Win32_OperatingSystem).LastBootUpTime
    $span = (Get-Date) - $boot
    '{0} days, {1:00}:{2:00} since {3}' -f $span.Days, $span.Hours, $span.Minutes, $boot
}
function flushdns { Clear-DnsClientCache; 'DNS cache cleared.' }
function admin {
    $shell = (Get-Process -Id $PID).Path
    if (Get-Command wt.exe -ErrorAction Ignore) { Start-Process wt.exe -ArgumentList "`"$shell`"" -Verb RunAs }
    else { Start-Process $shell -Verb RunAs }
}
function reload { . $PROFILE.CurrentUserCurrentHost }
