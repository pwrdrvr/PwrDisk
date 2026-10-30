---
name: release
description: Prepare and verify a PwrDisk release, land its release-prep PR, tag the verified commit, monitor signed publication, and hand off promotion. Use for cutting PwrDisk releases or checking release status; ordinary feature and fix PRs do not use this workflow.
---

# PwrDisk release

Release `pwrdrvr/PwrDisk` using the checked-in GitHub Actions workflow.
Read the applicable `AGENTS.md`, [release runbook](../../../docs/releasing.md),
[release workflow](../../../.github/workflows/release.yml), its adjacent
[design notes](../../../.github/workflows/README.md), and
[xtask](../../../xtask/src/main.rs) before changing release metadata.
Those files are authoritative; PwrRoute and PwrAgent have different publishing
and promotion behavior.

Match the user's requested scope: preparation ends with a reviewable PR;
a request to cut a release includes landing preparation and pushing its tag.
Signing-environment approvals and promotion to Latest belong to the operator.
Resume an existing preparation or run at its current stage rather than
duplicating it.

## Establish the version and changes

Inspect the worktree, remotes, current release PRs and published releases.
Push only to the remote pointing at `pwrdrvr/PwrDisk`; upstream and personal
forks are read-only. Examples below assume that remote is `origin`.
Preserve unrelated work, using an isolated worktree when needed.

```bash
git fetch origin main --tags
gh release list -R pwrdrvr/PwrDisk --limit 10
git log --oneline vPREV..origin/main
```

Substitute the previous release tag for `vPREV` and the intended version for
`X.Y.Z` throughout this skill. Read merged PR descriptions and relevant diffs
since that release, including direct commits. Distinguish fork releases from
upstream tags; do not select the highest local tag blindly. If the request and
context do not establish the version, ask while continuing scope inspection.

PwrDisk continues upstream's version sequence; its first fork release is
0.12.0. Use suffix-free `vX.Y.Z` for stable releases, `-rc.N` for candidates,
and `-beta.N` or `-alpha.N` for builds off main. Every release is initially a
GitHub Pre-release; only suffix-free tags may be promoted to Latest.

## Prepare the release PR

Start a release branch from current `origin/main`, or reuse an existing
release-prep branch after inspecting its diff. The normal preparation is:

| File | Change |
| --- | --- |
| `Cargo.toml` | Set `[workspace.package].version` to `X.Y.Z`. |
| `Cargo.lock` | Refresh workspace versions without upgrading dependencies. |
| `CHANGELOG.md` | Move Unreleased entries under `## vX.Y.Z`. |

All workspace crates inherit the root version. Refresh the lockfile with
`cargo metadata --format-version 1`, redirecting its large JSON output to a
temporary file. Do not use `--no-deps`, which skips the lockfile refresh.
Inspect the diff: only workspace package versions should change in the
lockfile. Investigate dependency drift rather than bundling it into the bump.
Bundle and desktop-entry versions are generated from the crate version;
do not replace unrelated version strings or change `com.pwrdrvr.pwrdisk`.

Preserve the changelog format, leave Unreleased empty, and reconcile its
existing bullets with the merged changes. If the target section already
exists, review and update it rather than creating a duplicate. Only release
preparation edits this file.

Write user-facing bullets from the included changes, for example
`- SSH scans - Show progress while scanning remote disks.`
Name the affected feature, describe what users gain or what is fixed, and
include measured results only when supported. Avoid PR-number lists and
internal implementation notes. This section supplies the GitHub release notes
and the notes Velopack displays beside an update.

## Verify and land preparation

Run the repository gates and inspect their actual results:

```bash
cargo xtask release-check --tag vX.Y.Z
cargo xtask lint
cargo xtask test
```

The metadata gate must report `release metadata agrees for vX.Y.Z`. Also
inspect workspace package versions in `Cargo.lock`; the gate checks the root
version, tag and nonempty changelog section. Report unavailable or failed
checks; do not claim the release is verified while the lint gate is red.

Commit the release files as `chore(release): vX.Y.Z` and open a release-prep
PR against the fork's `main`, following the repository PR guidance and
`~/.codex/pr-style.md`. Describe the included user-visible changes and actual
validation. A metadata-only release has no window change to screenshot.
Before opening or attaching the PR, ensure the thread workspace holds its
head commit. Attach and watch it using the PwrAgent tools as required by
`AGENTS.md`.

For a preparation-only request, hand off the PR here. For a release request,
wait for required checks and merge under the repository's configured policy.
Fetch main again and identify the actual merged commit. Verify its release
metadata (including `release-check`) and diff against the checked preparation;
never tag the pre-merge
branch SHA by assumption. If additional changes entered the release scope,
reconcile notes and repeat affected checks before tagging.

## Tag and monitor publication

Before pushing, confirm the intended commit is on `origin/main` and that both
`apple-signing` and `windows-signing` still require reviewer approval and
allow only `v*` tag deployments. Inspect configuration and secret names,
never secret values; [the runbook](../../../docs/releasing.md) lists them.
Missing configuration is a blocker, not permission to alter signing policy.

Inspect local and remote tags first. If the tag already exists, inspect its
target and run; never move or overwrite it as routine release preparation.
Create an annotated tag on the exact verified full commit SHA, preserving any
configured signing policy, and push only that tag:

```bash
git tag -a vX.Y.Z RELEASE_COMMIT_SHA -m "vX.Y.Z"
git push origin refs/tags/vX.Y.Z
git ls-remote origin 'refs/tags/vX.Y.Z*'
gh run list -R pwrdrvr/PwrDisk --workflow release.yml --limit 10
```

Verify the remote tag's peeled commit equals the intended release commit.
Find the workflow run for that tag and commit, not simply the newest run.
Monitor without blocking user updates for long periods. Give the operator
the run link and pending environment when approval is needed, and continue
monitoring after approval. Reaching a signing gate is not publication.

The current pipeline checks metadata and lints, builds Linux for `x86_64`
and `aarch64`, prepares a universal macOS app and both Windows architectures,
then signs and packs macOS and Windows in separate jobs. Only `macos-sign`
and `windows-sign` use their respective signing environments. They verify
the preparation archives' digests and never check out or build source.
Publication creates one GitHub Pre-release only after all assets are ready;
a failed build leaves no partial GitHub release.

On failure, inspect the failed job logs, report the cause, and stop publication
progression. Do not bypass approvals, downgrade signing, force-push, move an
existing tag, or dispatch replacement runs as an automatic retry. Recovery
requires explicit direction. For an authorized rebuild of an existing tag,
both the workflow ref and its tag input must name that same tag:

```bash
gh workflow run release.yml -R pwrdrvr/PwrDisk --ref vX.Y.Z -f tag=vX.Y.Z
```

An existing GitHub release causes publication to fail; rebuilding it requires
deleting that release first, which can interrupt update feeds. Report that
condition and obtain explicit authorization for deletion rather than removing
it automatically. Monitor one authorized recovery attempt and stop to diagnose
if it fails again.

## Verify the published release

```bash
gh release view vX.Y.Z -R pwrdrvr/PwrDisk \
  --json url,isDraft,isPrerelease,assets,body
gh release view -R pwrdrvr/PwrDisk --json tagName,url
```

Check the intended release is a non-draft Pre-release, every required job
succeeded, its body matches the prepared changelog section, and the expected
assets are present. Use the runbook's asset table as the authoritative list:

| Platform | Required coverage |
| --- | --- |
| Linux | x86_64 and aarch64 tarballs, each with `.sha256`. |
| macOS | Universal DMG, portable zip, full package, osx feeds and checksums. |
| Windows | Setup.exe, MSI, portable zip, full package and feeds for both architectures; shared checksums. |

Optional delta packages depend on an available previous full package, so do
not require a fixed asset count. Preserve Velopack's channel feeds (`osx`,
`win`, `win-arm64`); installed copies need them for updates. Download assets
to a temporary directory and verify their checksum manifests. Check signing
logs for Developer ID signing, team ID, notarization, staples and Gatekeeper
verification of both app and DMG, and valid timestamped Windows signatures
with the configured publisher, including installers and updater binaries.

Do not manually create the release ahead of CI or replace its changelog notes
with generated notes. There is no Winget or Homebrew distribution job to run.
Keep the `velopack` crate pin, `VPK_VERSION`, package hash and generated
notices aligned if a release includes a Velopack upgrade; an ordinary version
bump does not require changing them.

## Hand off promotion

Have the operator install from the DMG and Setup.exe and check the result
before promoting a suffix-free tag. Report installation and updater smoke
tests separately from CI signing verification; do not infer a working
installation from successful builds alone. Give the operator this command:

```bash
gh release edit vX.Y.Z -R pwrdrvr/PwrDisk --latest --prerelease=false
```

Do not execute promotion as part of a routine release request. Stable · Latest
is the default update channel and sees the release only after promotion;
before that it is available to Stable · Prerelease and Beta · Prerelease.
After the operator promotes, verify Latest points to the intended tag.
Suffixed tags remain prereleases.

Report the release URL, exact commit and tag, verification results, signing
and asset status, promotion status, and any remaining blockers. Distinguish a
published prerelease awaiting smoke checks from a promoted stable release.
