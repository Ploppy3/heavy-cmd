# Installs heavy-cmd's newest release on Windows as $env:HEAVY_CMD_INSTALL_DIR\heavy-cmd.exe
# (%LOCALAPPDATA%\Programs\heavy-cmd) and puts that folder on your PATH; run it again to update. On Linux: install.sh.
#
#   irm https://raw.githubusercontent.com/Ploppy3/heavy-cmd/main/install.ps1 | iex
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$Dir = if ($env:HEAVY_CMD_INSTALL_DIR) { $env:HEAVY_CMD_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\heavy-cmd' }
$Url = 'https://github.com/Ploppy3/heavy-cmd/releases/latest/download/heavy-cmd-windows-x86_64.exe'
if (-not [Environment]::Is64BitOperatingSystem) { throw 'heavy-cmd install: heavy-cmd is built for 64-bit Windows' }

$Exe = Join-Path $Dir 'heavy-cmd.exe'
$New = Join-Path $Dir "heavy-cmd.$PID.new"
New-Item -ItemType Directory -Force -Path $Dir | Out-Null
Get-ChildItem -Path $Dir -Filter '*.old' -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
try {
  Invoke-WebRequest -Uri $Url -OutFile $New -UseBasicParsing
  # A running heavy-cmd.exe cannot be replaced, but it can be renamed: it is removed by the next install.
  if (Test-Path $Exe) { Rename-Item -Path $Exe -NewName "heavy-cmd.$PID.old" }
  Move-Item $New $Exe
} finally {
  Remove-Item -Force $New -ErrorAction SilentlyContinue
}

# The folder on the user's PATH, kept as the registry has it (%VARIABLES% unexpanded).
$Key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
$UserPath = [string]$Key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
if (-not (($UserPath -split ';') -contains $Dir)) {
  $Key.SetValue('Path', ((@($UserPath.TrimEnd(';'), $Dir) | Where-Object { $_ }) -join ';'), [Microsoft.Win32.RegistryValueKind]::ExpandString)
  # Setting a variable through .NET tells running programs the environment changed; this one goes at once.
  [Environment]::SetEnvironmentVariable('HEAVY_CMD_INSTALLING', '1', 'User')
  [Environment]::SetEnvironmentVariable('HEAVY_CMD_INSTALLING', $null, 'User')
  $env:Path = "$env:Path;$Dir"
  Write-Host "heavy-cmd install: $Dir is now on your PATH; terminals opened from now on find heavy-cmd"
}
$Key.Close()
Write-Host "heavy-cmd installed: $Exe"
