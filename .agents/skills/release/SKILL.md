---
name: release
description: Prepare and verify a PwrDisk release, push one release commit directly to main, tag the verified commit, monitor signed publication, and hand off promotion. Use for cutting PwrDisk releases or checking release status; ordinary feature and fix PRs do not use this workflow.
---

# PwrDisk release

Release `pwrdrvr/PwrDisk` using the checked-in GitHub Actions workflow.
Read the applicable `AGENTS.md`, [release runbook](../../../docs/releasing.md),
[release workflow](../../../.github/workflows/release.yml), its adjacent
[design notes](../../../.github/workflows/README.md), and
[xtask](../../../xtask/src/main.rs) before changing release metadata.
Those files are authoritative; PwrRoute and PwrAgent have different publishing
and promotion behavior.

Match the user's requested scope: preparation ends with a reviewable local
commit; a request to cut a release authorizes pushing that one release commit
directly to `main`, then pushing its tag. Do not open a release-prep PR.
Signing-environment approvals and promotion to Latest belong to the operator.
Resume an existing preparation or run at its current stage rather than
duplicating it.

## Establish the version and changes

Inspect the worktree, remotes, existing release commits and published releases.
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

## Prepare one release commit

Use a clean checkout of current `origin/main`. An isolated worktree can keep
unrelated edits safe; it does not require a preparation branch or PR. Reuse
existing unpublished preparation after inspecting its diff. Keep all release
preparation in one commit on top of main. The normal preparation is:

| File | Change |
| --- | --- |
| `Cargo.toml` | Set `[workspace.package].version` to `X.Y.Z`. |
| `Cargo.lock` | Refresh workspace versions without upgrading dependencies. |
| `CHANGELOG.md` | Move Unreleased entries under `## vX.Y.Z`. |
| `THIRD_PARTY_LICENSES` | Regenerate with `cargo xtask licenses` after the version bump. |

All workspace crates inherit the root version. Refresh the lockfile with
`cargo metadata --format-version 1`, redirecting its large JSON output to a
temporary file. Do not use `--no-deps`, which skips the lockfile refresh.
Inspect the diff: only workspace package versions should change in the
lockfile. Investigate dependency drift rather than bundling it into the bump.
Bundle and desktop-entry versions are generated from the crate version;
do not replace unrelated version strings or change `com.pwrdrvr.pwrdisk`.

`THIRD_PARTY_LICENSES` embeds all six workspace crate versions
(`disktree-app`, `disktree-core`, `disktree-link`, `disktree-mcp`,
`disktree-scan` and `xtask`), so ordinary version bumps require regeneration
even when dependencies are unchanged. Use the `cargo-about` version pinned
in [CI](../../../.github/workflows/ci.yml), currently `0.9.2`, for both
generation and verification:

```bash
cargo install --locked cargo-about@0.9.2 --features cli
cargo xtask licenses
```

Review the generated diff for the expected workspace-version changes.
Investigate any other changes; do not upgrade dependencies or alter license
policy to make an ordinary bump pass, and do not edit the notices by hand.
Include the generated file in the same release commit as the version,
lockfile and changelog.

This differs from `licenses/velopack.txt`, the notices for Velopack's native
binaries: regenerate that input only when the pinned Velopack tooling
changes, not for an ordinary PwrDisk version bump.

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

## Verify and push main

Run the repository gates and inspect their actual results. Follow
[remote verification](../../../docs/sandboxer.md) for Linux checks when
Sandboxer is available. Commit preparation locally before transferring it;
if corrections are needed, amend the unpublished release commit and verify
its final tree.

```bash
cargo xtask release-check --tag vX.Y.Z
cargo xtask licenses --check
cargo xtask lint
cargo xtask test
```

The notices check must report `THIRD_PARTY_LICENSES is current`, using the
same CI-pinned `cargo-about` version as regeneration.
The metadata gate must report `release metadata agrees for vX.Y.Z`. Also
inspect workspace package versions in `Cargo.lock`; the gate checks the root
version, tag and nonempty changelog section. Report unavailable or failed
checks; do not claim the release is verified while the lint gate is red.

Commit the release files together as `chore(release): vX.Y.Z`. For a
preparation-only request, hand off the verified local commit here.

For a release request, fetch `origin/main` again before pushing. If main
advanced, replay the unpublished release commit onto it without a merge
commit, reconcile the added changes with the release notes, and repeat
affected checks. Confirm the final tree was verified and the only commit
being added is the release preparation:

```bash
git fetch origin main
git merge-base --is-ancestor origin/main HEAD
git rev-list --count origin/main..HEAD
```

The ancestor check must succeed and the count must be exactly `1`. Push that
commit as a normal fast-forward update with `git push origin HEAD:main`.
Do not force-push, add a merge commit, or fall back to a PR if repository
policy rejects the push; report the rejection and stop. Verify remote main
holds the exact release commit before tagging it.

Report the commit and verification results. Stop the verification sandbox
after checks; this workflow has no PR watch to clean it up automatically.

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
Keep the `velopack` crate pin, `VPK_VERSION`, package hash and
`licenses/velopack.txt` aligned if a release includes a Velopack upgrade;
an ordinary version bump leaves those unchanged but still regenerates
`THIRD_PARTY_LICENSES` as described above.

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
