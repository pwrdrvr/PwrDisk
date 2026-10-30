# Releasing

1. Bump `version` in `[workspace.package]` of the root `Cargo.toml`, and move
   `CHANGELOG.md`'s Unreleased entries under a matching `## vX.Y.Z` heading.
   `cargo xtask release-check --tag vX.Y.Z` checks the two agree.
2. Merge, then push the `vX.Y.Z` tag from `main`.
3. Approve the `apple-signing` and `windows-signing` deployments in the
   Actions tab when the run pauses for them.
4. The run creates a GitHub **Pre-release** with every asset once all
   platforms are in. Install it from the DMG and Setup.exe, check it, then
   promote it:

   ```sh
   gh release edit vX.Y.Z --repo pwrdrvr/PwrDisk --latest --prerelease=false
   ```

   Installed copies on Stable · Latest, the default channel, see a release
   only once it is promoted; until then it reaches Stable · Prerelease and
   Beta · Prerelease. Promote suffix-free tags only.

The changelog section is the release's notes on GitHub and the notes
Velopack keeps with the update. A tag whose version or section is missing
fails in the first job, before anything is built or signed.

To rebuild an existing tag, run the Release workflow by hand with **Use
workflow from** set to that tag, and the same tag in the form. The signing
environments accept only `v*` tags, so a run started from a branch stops in
the first job. The release is created only when every platform is in, so a
failed run leaves nothing to clean up; a run for a tag that already has a
release fails at the end, so delete that release first.

## What a release contains

| Platform | Assets |
| --- | --- |
| macOS | `PwrDisk-<v>-universal.dmg`, `PwrDisk-osx-Portable.zip`, `PwrDisk-<v>-osx-full.nupkg` (and `-delta`), `releases.osx.json`, `assets.osx.json`, `RELEASES-osx`, `PwrDisk-<v>-macos-SHA256SUMS` |
| Windows x64 | `PwrDisk-win-Setup.exe`, `PwrDisk-win.msi`, `PwrDisk-win-Portable.zip`, `PwrDisk-<v>-full.nupkg` (and `-delta`), `releases.win.json`, `assets.win.json`, `RELEASES` |
| Windows ARM64 | the same with `win-arm64` in place of `win` |
| Windows, both | `PwrDisk-<v>-windows-SHA256SUMS` |
| Linux | `pwrdisk-<v>-<arch>-linux.tar.gz` and `.sha256`, for x86_64 and aarch64 |

Velopack's installer and portable names carry no version, so
`https://github.com/pwrdrvr/PwrDisk/releases/latest/download/PwrDisk-win-Setup.exe`
stays a good link once a release is promoted. The `releases.<channel>.json`
feeds are what installed copies read; never delete them from a release.

## Versions

PwrDisk's first release is 0.12.0: the fork carries on from disktree 0.11.0,
the last version it took from upstream, rather than restarting, so a version
always says which is newer. Tags follow the Pwr family's channels: a
suffix-free `vX.Y.Z` for stable releases, `-rc.N` for candidates, `-beta.N`
and `-alpha.N` for builds off `main`. Velopack compares versions as SemVer.

A preview build for a pull request is versioned `<base>-pr.<number>`; see
`.github/workflows/README.md`.

## Velopack

Windows and macOS install and update through
[Velopack](https://docs.velopack.io), as PwrRoute does. The app runs
Velopack's startup hook first in `main` (`crates/disktree-app/src/updater.rs`)
and reads the release list from GitHub's API without a token, so the
repository must stay public for updates to work.

The sign jobs install the `vpk` CLI with `scripts/release/install-vpk.sh`,
only from a package whose SHA-512 matches the one pinned there. Its
`VPK_VERSION` must equal the `velopack` crate's `=` pin in
`crates/disktree-app/Cargo.toml`: upgrade both, and the hash, together, then
rerun `scripts/release/velopack-notices.sh` to regenerate
`licenses/velopack.txt`, the notices for the binaries `vpk` adds
(`Update.exe`, `Setup.exe`, the stub and `UpdateMac`). Both the Windows
install and the Mac app carry that file as `VELOPACK-NOTICES.txt`.

Linux does not use Velopack: `vpk`'s Linux output is an AppImage, and its
updater only runs from one. The tarball updates by downloading the next
release.

To rebuild an existing tag, run the Release workflow by hand with **Use
workflow from** set to that tag, and the same tag in the form. The signing
environments accept only `v*` tags, so a run started from a branch stops
before drafting anything.

## Signing

Release signing uses two GitHub environments. Both accept deployments only
from `v*` tags and wait for a required reviewer, so every release run pauses
until it is approved in the Actions tab. A release without full signing
credentials fails; it never ships unsigned.

### `apple-signing`

Used by `macos-sign` only. `macos-prepare` builds the universal app on a
hosted runner without any credential and hands it over as an archive whose
SHA-256 travels as a job output; `macos-sign` checks both, never checks out
or builds source, and fails if any of the five secrets is missing.

| Secret | What |
| --- | --- |
| `CSC_LINK` | the Developer ID Application `.p12`, base64 |
| `CSC_KEY_PASSWORD` | its password |
| `APPLE_API_KEY_BASE64` | an App Store Connect API key (`AuthKey_*.p8`), base64 |
| `APPLE_API_KEY_ID` | that key's ID |
| `APPLE_API_ISSUER` | its issuer ID |

The signing identity is read from the certificate itself. Upload all five from
1Password, the same item PwrAgent and PwrRoute use:

```sh
scripts/release/upload-csc-link-from-1password.sh --list-accounts
OP_ACCOUNT=<USER ID> scripts/release/upload-csc-link-from-1password.sh
```

`scripts/release/macos-signing.py` imports the certificate into a keychain
of the job's own and saves the API key as the notarytool profile
`pwrdisk-notary`; it restores the runner's keychain list and deletes both
afterwards, even when a later step fails. `vpk pack` signs the app with the
hardened runtime and `packaging/macos/PwrDisk.entitlements` (none),
notarizes and staples it; `macos-dmg.sh` does the same for the DMG cut from
it. The job checks the team identifier (`T44CNHC4UH`), both staples and
`spctl` before anything is uploaded.

### `windows-signing`

Used by `windows-sign`, which packs and signs what both Windows build jobs
(x64 on `windows-2025`, ARM64 on `windows-11-arm`) hand over, checking each
archive against the digest its build job reported. It never checks out or
builds source, and fails if any of the seven values is missing. `vpk pack`
signs `pwrdisk.exe`, `Update.exe` and the launcher, then `Setup.exe` and
the MSI, through the Artifact Signing dlib it ships. Signing uses Azure Artifact
Signing, with the same signing account and profile as PwrAgent and its own
Entra app registration; see PwrAgent's `docs/desktop-windows-signing.md` for
how those were set up. Upload all seven values from the 1Password item
"PwrDisk - Windows Azure Trusted Signing":

```sh
OP_ACCOUNT=<USER ID> scripts/release/upload-azure-signing-from-1password.sh
```

| Variable | Value |
| --- | --- |
| `WIN_AZURE_SIGN_ENDPOINT` | `https://eus.codesigning.azure.net/` |
| `WIN_AZURE_SIGN_ACCOUNT` | `pwrdrvrsigning` |
| `WIN_AZURE_SIGN_PROFILE` | `pwrdrvr-public-trust` |
| `WIN_AZURE_SIGN_PUBLISHER_NAME` | `PwrDrvr LLC` |

| Secret | What |
| --- | --- |
| `AZURE_TENANT_ID` | the Entra directory (tenant) ID |
| `AZURE_CLIENT_ID` | the signing app registration's client ID |
| `AZURE_CLIENT_SECRET` | its client secret; the 1Password item records when it expires |

Every one of those files' signatures must be valid, timestamped and issued
to the publisher above, or the job fails.

## Runners

Every job uses standard GitHub-hosted runners, which are free for a public
repository, macOS included. Larger runners are billed even on public
repositories, so do not switch to them. Nor to self-hosted ones: this repository takes pull
requests from forks, and those must never reach a persistent machine.
