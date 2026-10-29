# Releasing

Bump `version` in `[workspace.package]` of the root `Cargo.toml`, merge, then
push a matching `vX.Y.Z` tag. `release.yml` drafts the release, builds Linux,
macOS and Windows, and publishes it once every build is in.

## Signing

Release signing uses two GitHub environments. Both accept deployments only
from `v*` tags and wait for a required reviewer, so every release run pauses
until it is approved in the Actions tab. A release without full signing
credentials fails; it never ships unsigned.

### `apple-signing`

Used by the macOS build jobs. Only the signing steps see the secrets; the
tests and the build run first, without them.

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

The app is signed with the hardened runtime, notarized, stapled, and checked
with `spctl` before upload.

### `windows-signing`

Used by `windows-sign`, which signs the executables both Windows build jobs
hand over. It never checks out or builds source. Signing uses Azure Artifact
Signing, with the same account and profile as PwrAgent; see PwrAgent's
`docs/desktop-windows-signing.md` for how they were set up.

| Variable | Value |
| --- | --- |
| `WIN_AZURE_SIGN_ENDPOINT` | `https://eus.codesigning.azure.net/` |
| `WIN_AZURE_SIGN_ACCOUNT` | `pwrdrvrsigning` |
| `WIN_AZURE_SIGN_PROFILE` | `pwrdrvr-public-trust` |
| `WIN_AZURE_SIGN_PUBLISHER_NAME` | `PwrDrvr` |

| Secret | What |
| --- | --- |
| `AZURE_TENANT_ID` | the Entra directory (tenant) ID |
| `AZURE_CLIENT_ID` | the signing app registration's client ID |
| `AZURE_CLIENT_SECRET` | one of its client secrets (expires within 24 months) |

The signature must be valid, timestamped and issued to the publisher above,
or the job fails.

## Runners

Every job uses standard GitHub-hosted runners, which are free for a public
repository, macOS included. Larger runners are billed even on public
repositories, so do not switch to them.
