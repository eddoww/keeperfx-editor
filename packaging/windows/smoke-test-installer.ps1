# Try a freshly built installer: install silently into a temporary folder for the current user,
# check the files and the Start menu shortcut, run kfx-editor-cli.exe, uninstall silently and
# check that everything is gone. Used by .github/workflows/ci.yml and release.yml.
#
#   ./packaging/windows/smoke-test-installer.ps1 -Setup out\kfx-editor-v0.1.2-windows-x86_64-setup.exe -Version 0.1.2
param(
    [Parameter(Mandatory = $true)] [string] $Setup,
    [Parameter(Mandatory = $true)] [string] $Version
)
$ErrorActionPreference = 'Stop'

$tmp = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [System.IO.Path]::GetTempPath() }
$dir = Join-Path $tmp 'kfx-smoke\KeeperFX Map Editor'
$log = Join-Path $tmp 'kfx-smoke-setup.log'
# the editor's settings and update state go to a throwaway folder
$env:KFX_EDITOR_CONFIG_DIR = Join-Path $tmp 'kfx-smoke-config'

$p = Start-Process -FilePath (Resolve-Path $Setup).Path -Wait -PassThru -ArgumentList @(
    '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', "/DIR=`"$dir`"", "/LOG=`"$log`"")
if ($p.ExitCode -ne 0) {
    if (Test-Path $log) { Get-Content $log }
    throw "setup failed with exit code $($p.ExitCode)"
}

$expected = @('kfx-editor.exe', 'kfx-editor-cli.exe', 'README.md', 'CHANGELOG.md', 'LICENSE',
    'THIRD_PARTY.md', 'kfx-editor.ico', 'setup-install', 'unins000.exe', 'docs\manual\README.md')
foreach ($f in $expected) {
    if (-not (Test-Path (Join-Path $dir $f))) { throw "not installed: $f" }
}
$lnk = Join-Path ([Environment]::GetFolderPath('Programs')) 'KeeperFX Map Editor.lnk'
if (-not (Test-Path $lnk)) { throw "no Start menu shortcut at $lnk" }

$cli = Join-Path $dir 'kfx-editor-cli.exe'
# The first line is the editor version; the following lines name the KeeperFX data revision.
$v = ((& $cli --version | Out-String).Trim() -split "`r?`n")[0]
if ($LASTEXITCODE -ne 0 -or $v -ne "kfx-editor $Version") { throw "--version printed '$v' (exit $LASTEXITCODE)" }
Write-Host $v
# 0: up to date, 10: a newer release exists, 1: offline or rate-limited by GitHub. Any other
# code (a crash) fails the test.
& $cli update --check
$code = $LASTEXITCODE
Write-Host "update --check exit code $code"
if ($code -notin @(0, 1, 10)) { throw "update --check failed with exit code $code" }

$u = Start-Process -FilePath (Join-Path $dir 'unins000.exe') -Wait -PassThru -ArgumentList @(
    '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART')
if ($u.ExitCode -ne 0) { throw "the uninstaller failed with exit code $($u.ExitCode)" }
# the uninstaller finishes from a copy of itself in the temporary folder: wait for it
$deadline = (Get-Date).AddSeconds(120)
while ((Test-Path $dir) -and ((Get-Date) -lt $deadline)) { Start-Sleep -Seconds 1 }
if (Test-Path $dir) {
    Get-ChildItem -Recurse $dir | ForEach-Object { $_.FullName }
    throw 'the install folder is still there after uninstalling'
}
if (Test-Path $lnk) { throw 'the Start menu shortcut is still there after uninstalling' }
Write-Host 'installer smoke test passed'
