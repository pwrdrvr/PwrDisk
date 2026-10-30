# Pack PwrDisk for Windows with vpk: Setup.exe, a per-user MSI, the
# portable zip and the update feed, for one architecture. release.yml's
# windows-sign job runs it from the signing input (with release notes and
# Azure signing); preview-build.yml runs it from the checkout (unsigned).
# One script, so the preview packs what the release packs.
#
# - -Arch x64 | arm64: vpk's --runtime (win-x64, win-arm64). Without it vpk
#   defaults to x86 Setup.exe and Update.exe.
# - --channel: x64 keeps Velopack's default `win`, so its asset names
#   (PwrDisk-win-Setup.exe, releases.win.json) match the rest of the Pwr
#   family; arm64 is `win-arm64`, a feed of its own, so an install never
#   updates to the other architecture's build.
# - --instLocation PerUser: PwrDisk installs per user, like the rest of the
#   family, and needs no elevation to install or update. vpk's default
#   dual-scope MSI preselects per-machine in its dialog.
# - --azureTrustedSignFile: vpk signs pwrdisk.exe, Update.exe and the stub,
#   then Setup.exe and the MSI, through Azure Artifact Signing; the dlib reads
#   the service principal from AZURE_TENANT_ID / AZURE_CLIENT_ID /
#   AZURE_CLIENT_SECRET.
# - --icon: assets/disktree.ico, the icon pwrdisk.exe embeds, for Setup.exe,
#   the MSI and the Start menu shortcut.
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)] [string]$Version,
  [Parameter(Mandatory = $true)] [ValidateSet("x64", "arm64")] [string]$Arch,
  [Parameter(Mandatory = $true)] [string]$PackDir,
  [Parameter(Mandatory = $true)] [string]$OutputDir,
  [Parameter(Mandatory = $true)] [string]$Icon,
  [string]$ReleaseNotes,
  [string]$AzureSignFile
)

$ErrorActionPreference = "Stop"

$channel = if ($Arch -eq "x64") { "win" } else { "win-$Arch" }

$vpkArgs = @(
  "--skip-updates", "pack",
  "--packId", "PwrDisk",
  "--packVersion", $Version,
  "--runtime", "win-$Arch",
  "--channel", $channel,
  "--packDir", $PackDir,
  "--mainExe", "pwrdisk.exe",
  "--packTitle", "PwrDisk",
  "--packAuthors", "PwrDrvr LLC",
  "--shortcuts", "StartMenuRoot",
  "--icon", $Icon,
  "--msi", "--instLocation", "PerUser",
  "--outputDir", $OutputDir
)
if ($ReleaseNotes) { $vpkArgs += @("--releaseNotes", $ReleaseNotes) }
if ($AzureSignFile) { $vpkArgs += @("--azureTrustedSignFile", $AzureSignFile) }

& vpk @vpkArgs
if ($LASTEXITCODE -ne 0) { throw "vpk pack failed (exit code $LASTEXITCODE)." }
