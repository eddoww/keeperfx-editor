# Build the Windows installer from a staged folder with Inno Setup's compiler (iscc).
# Used by .github/workflows/ci.yml and release.yml on the Windows runner; works on any Windows
# machine with Inno Setup 6.3 or newer.
#
#   ./packaging/windows/build-installer.ps1 -Version 0.1.2 -SourceDir <staged folder> `
#       -OutputDir out -OutputBase kfx-editor-v0.1.2-windows-x86_64-setup
#
# The staged folder holds kfx-editor.exe, kfx-editor-cli.exe, README.md, CHANGELOG.md, LICENSE,
# THIRD_PARTY.md and docs\ (see installer.iss).
param(
    [Parameter(Mandatory = $true)] [string] $Version,
    [Parameter(Mandatory = $true)] [string] $SourceDir,
    [Parameter(Mandatory = $true)] [string] $OutputDir,
    [Parameter(Mandatory = $true)] [string] $OutputBase
)
$ErrorActionPreference = 'Stop'

# GitHub's Windows runner images ship Inno Setup with iscc on PATH; install it only if missing.
$iscc = Get-Command iscc -ErrorAction SilentlyContinue
if (-not $iscc) {
    Write-Host 'iscc is not on PATH: installing Inno Setup with Chocolatey'
    choco install innosetup -y --no-progress
    if ($LASTEXITCODE -ne 0) { throw "choco install innosetup failed ($LASTEXITCODE)" }
    $env:Path += ";${env:ProgramFiles(x86)}\Inno Setup 6"
    $iscc = Get-Command iscc -ErrorAction Stop
}
Write-Host "using $($iscc.Source)"

New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
$src = (Resolve-Path $SourceDir).Path
$out = (Resolve-Path $OutputDir).Path
$script = Join-Path $PSScriptRoot 'installer.iss'
& $iscc.Source "/DAppVersion=$Version" "/DSourceDir=$src" "/DOutputDir=$out" "/DOutputBase=$OutputBase" $script
if ($LASTEXITCODE -ne 0) { throw "iscc failed ($LASTEXITCODE)" }
Get-Item (Join-Path $out "$OutputBase.exe") | Format-List FullName, Length
