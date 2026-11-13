# Installs heavy-cmd's newest release on Windows, or makes sure it is there: run it again at will.
#
#   irm https://raw.githubusercontent.com/Ploppy3/heavy-cmd/main/install.ps1 | iex
#
# It puts the release's heavy-cmd.exe in $env:HEAVY_CMD_INSTALL_DIR (%LOCALAPPDATA%\Programs\heavy-cmd)
# as bin\heavy-cmd.exe once it runs, keeping the one before as bin\heavy-cmd.old.exe, and puts that bin
# folder on your PATH. `heavy-cmd update` moves it on. On Linux: install.sh.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$InstallDir = if ($env:HEAVY_CMD_INSTALL_DIR) { $env:HEAVY_CMD_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\heavy-cmd' }
$Api = if ($env:HEAVY_CMD_RELEASES_API) { $env:HEAVY_CMD_RELEASES_API } else { 'https://api.github.com/repos/Ploppy3/heavy-cmd/releases/latest' }
$Asset = 'heavy-cmd-windows-x86_64.zip'

if (-not [Environment]::Is64BitOperatingSystem) { throw 'heavy-cmd install: heavy-cmd is built for 64-bit Windows' }

$Release = Invoke-RestMethod -Uri $Api -Headers @{ Accept = 'application/vnd.github+json'; 'User-Agent' = 'heavy-cmd' }
$Tag = [string]$Release.tag_name
if ($Tag -notmatch '^v\d+\.\d+\.\d+$') { throw "heavy-cmd install: $Api names no release tag" }
$Url = ($Release.assets | Where-Object { $_.name -eq $Asset } | Select-Object -First 1).browser_download_url
if (-not $Url) { throw "heavy-cmd install: release $Tag has no $Asset" }

$Bin = Join-Path $InstallDir 'bin'
$Exe = Join-Path $Bin 'heavy-cmd.exe'
$Old = Join-Path $Bin 'heavy-cmd.old.exe'
New-Item -ItemType Directory -Force -Path $Bin | Out-Null

# What `<path> --version` prints, or why it did not run.
function Get-Said([string]$Path) {
  try {
    $Out = (& $Path --version 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) { return "failed: $Out" }
    return $Out
  } catch { return "failed: $_" }
}

if ((Test-Path $Exe) -and ((Get-Said $Exe) -eq "heavy-cmd $Tag")) {
  Write-Host "heavy-cmd install: $Tag is already installed in $Exe"
} else {
  $Work = Join-Path $InstallDir ".install-$PID"
  Remove-Item -Recurse -Force $Work -ErrorAction SilentlyContinue
  New-Item -ItemType Directory -Path $Work | Out-Null
  try {
    $Zip = Join-Path $Work $Asset
    Invoke-WebRequest -Uri $Url -OutFile $Zip -UseBasicParsing
    Expand-Archive -Path $Zip -DestinationPath $Work
    $New = Join-Path $Work 'heavy-cmd\bin\heavy-cmd.exe'
    $Said = Get-Said $New
    if ($Said -ne "heavy-cmd $Tag") { throw "heavy-cmd install: $Tag does not run here: its --version said `"$Said`"" }
    # A running heavy-cmd.exe cannot be replaced, but it can be renamed: the one in use becomes heavy-cmd.old.exe.
    if (Test-Path $Old) {
      Remove-Item -Force $Old -ErrorAction SilentlyContinue
      if (Test-Path $Old) { Rename-Item -Path $Old -NewName ".discarded-$PID-$(Get-Random).exe" }
    }
    if (Test-Path $Exe) { Move-Item -Force $Exe $Old }
    Move-Item $New $Exe
  } finally {
    Remove-Item -Recurse -Force $Work -ErrorAction SilentlyContinue
  }
}
Set-Content -Path (Join-Path $InstallDir '.heavy-cmd-install') -Value '' -NoNewline

# The bin folder on the user's PATH, kept as the registry has it (%VARIABLES% unexpanded).
$Key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
$UserPath = [string]$Key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
if (-not (($UserPath -split ';') -contains $Bin)) {
  $Key.SetValue('Path', ((@($UserPath.TrimEnd(';'), $Bin) | Where-Object { $_ }) -join ';'), [Microsoft.Win32.RegistryValueKind]::ExpandString)
  # Setting a variable through .NET tells running programs the environment changed; this one goes at once.
  [Environment]::SetEnvironmentVariable('HEAVY_CMD_INSTALLING', '1', 'User')
  [Environment]::SetEnvironmentVariable('HEAVY_CMD_INSTALLING', $null, 'User')
  $env:Path = "$env:Path;$Bin"
  Write-Host "heavy-cmd install: $Bin is now on your PATH; terminals opened from now on find heavy-cmd"
}
$Key.Close()
Write-Host "heavy-cmd $Tag installed: $Exe"
