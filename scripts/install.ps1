# StrangeTimer one-command installer (Windows).
#
#   irm https://github.com/AdarshGuptaa/strange_timer/releases/latest/download/install.ps1 | iex
#
# Installs into %LOCALAPPDATA%\StrangeTimer (no admin), adds the bin
# directory to the user PATH, installs PowerShell completions, creates a
# user-level scheduled task for autostart and starts the daemon.
# Options: -Version <tag>, -NoAutostart, -NoCompletions,
#          -Uninstall, -PurgeData

param(
    [string]$Version = "",
    [switch]$NoAutostart,
    [switch]$NoCompletions,
    [switch]$Uninstall,
    [switch]$PurgeData
)

$ErrorActionPreference = "Stop"
$Repo = "AdarshGuptaa/strange_timer"
$Root = Join-Path $env:LOCALAPPDATA "StrangeTimer"
$BinDir = Join-Path $Root "bin"
$PayloadRoot = Join-Path $Root "lib"
$DataDir = Join-Path $env:LOCALAPPDATA "strangetimer"

function Say($m) { Write-Host $m -ForegroundColor Cyan }
function Die($m) { Write-Error $m; exit 1 }

$arch = $env:PROCESSOR_ARCHITECTURE
$arch = if ($arch -in @("AMD64", "x86_64")) { "x86_64" } elseif ($arch -in @("ARM64", "aarch64")) { "aarch64" } else { Die "unsupported architecture: $arch" }

if ($Uninstall) {
    Say "Uninstalling StrangeTimer..."
    $exe = Join-Path $BinDir "strangetimer.exe"
    if (Test-Path $exe) {
        & $exe daemon stop 2>$null | Out-Null
        & $exe daemon uninstall 2>$null | Out-Null
    }
    & schtasks /Delete /TN "StrangeTimerDaemon" /F 2>$null | Out-Null
    Remove-Item -Recurse -Force $Root -ErrorAction SilentlyContinue
    if ($PurgeData) {
        Remove-Item -Recurse -Force $DataDir -ErrorAction SilentlyContinue
        Say "Timer data purged."
    } else {
        Say "Your timer data was kept in $DataDir."
    }
    Write-Host "Done. PATH changes take effect in a NEW terminal." -ForegroundColor Yellow
    exit 0
}

if (-not $Version) {
    Say "Resolving the latest release..."
    try {
        $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers @{ "User-Agent" = "strangetimer-installer" }
    } catch {
        # /releases/latest ignores prereleases; fall back to the most
        # recent release (which may be a prerelease) so a beta-only repo
        # still installs.
        $release = (Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases?per_page=1" -Headers @{ "User-Agent" = "strangetimer-installer" })[0]
    }
    $Version = $release.tag_name
}
Say "Installing StrangeTimer $Version (windows-$arch) into $Root"

$archive = "strangetimer-$Version-windows-$arch.zip"
$url = "https://github.com/$Repo/releases/download/$Version/$archive"
$tmp = Join-Path $env:TEMP ("strangetimer-" + [guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    $zip = Join-Path $tmp $archive
    Say "Downloading $archive..."
    Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing

    Say "Verifying SHA-256..."
    $sha = (Get-FileHash -Algorithm SHA256 $zip).Hash
    try {
        $sums = (Invoke-WebRequest -Uri "https://github.com/$Repo/releases/download/$Version/checksums.txt" -UseBasicParsing).Content
        if ($sums -notmatch $sha) { Die "checksum verification failed" }
    } catch {
        Write-Host "  (no checksums file published; skipping verification)" -ForegroundColor Yellow
    }

    Say "Extracting..."
    Expand-Archive -Path $zip -DestinationPath $tmp -Force
    if (-not (Test-Path (Join-Path $tmp "strangetimer.exe"))) { Die "archive missing strangetimer.exe" }
    if (-not (Test-Path (Join-Path $tmp "strangetimer-daemon.exe"))) { Die "archive missing strangetimer-daemon.exe" }

    $dest = Join-Path $PayloadRoot $Version
    $destTmp = "$dest.tmp"
    Remove-Item -Recurse -Force $destTmp -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Path $destTmp | Out-Null
    Copy-Item (Join-Path $tmp "strangetimer.exe") $destTmp
    Copy-Item (Join-Path $tmp "strangetimer-daemon.exe") $destTmp
    Copy-Item -Recurse (Join-Path $tmp "assets") (Join-Path $destTmp "assets")
    Move-Item $destTmp $dest -ErrorAction Stop
    Remove-Item (Join-Path $PayloadRoot "current") -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType SymbolicLink -Path (Join-Path $PayloadRoot "current") -Target $dest | Out-Null
    New-Item -ItemType Directory -Path $BinDir | Out-Null
    New-Item -ItemType SymbolicLink -Path (Join-Path $BinDir "strangetimer.exe") -Target (Join-Path $dest "strangetimer.exe") -Force | Out-Null
    New-Item -ItemType SymbolicLink -Path (Join-Path $BinDir "strangetimer-daemon.exe") -Target (Join-Path $dest "strangetimer-daemon.exe") -Force | Out-Null
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}

$cli = Join-Path $BinDir "strangetimer.exe"

# PATH (user scope)
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$BinDir*") {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$BinDir", "User")
    Write-Host "  added $BinDir to your user PATH (new terminals only)" -ForegroundColor Yellow
}

if (-not $NoCompletions) {
    Say "Installing PowerShell completions..."
    $profileDir = Split-Path $PROFILE
    if ($profileDir) { New-Item -ItemType Directory -Path $profileDir -Force | Out-Null }
    $line = "strangetimer completions powershell | Out-String | Invoke-Expression"
    if (Test-Path $PROFILE) {
        $content = Get-Content $PROFILE -Raw -ErrorAction SilentlyContinue
        if ($content -notlike "*$line*") { Add-Content -Path $PROFILE -Value "`n$line" }
    } else {
        Set-Content -Path $PROFILE -Value $line
    }
}

if ($NoAutostart) {
    Say "Installed. Start the daemon with: strangetimer daemon start"
} else {
    Say "Creating autostart task and starting the daemon..."
    & schtasks /Create /TN "StrangeTimerDaemon" /SC ONLOGON /TR "`"$cli`"" /RL LIMITED /F | Out-Null
    & $cli daemon start | Out-Null
    if ($LASTEXITCODE -ne 0) { Die "daemon failed to start" }
}

Say "StrangeTimer $Version installed!"
Write-Host "  Open a NEW terminal, then try:" -ForegroundColor Cyan
Write-Host "    strangetimer create timer demo 1m" -ForegroundColor Cyan
Write-Host "    strangetimer run demo" -ForegroundColor Cyan
Write-Host "    strangetimer view timers" -ForegroundColor Cyan
