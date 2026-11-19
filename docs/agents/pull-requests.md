# Pull requests

* **Screenshots and videos go on the PR with `gh --attach`.** GitHub CLI
  2.99+ uploads them: `gh pr create/edit/comment --attach ./after.png`, with
  the same `./after.png` in the Markdown body, rewrites that link to a
  `user-attachments` URL. Never push images to a side branch, a gist or the
  repository to host them. `gh` exits 0 even when the link does not match, so
  check every image link before calling a PR done. The procedure, and the
  three ways it fails silently, are in
  [docs/pr-screenshots.md](../pr-screenshots.md); read it before the first
  `--attach`, and before any later body edit.
* **A PR that changes what the window shows has before/after screenshots**,
  of a contrived tree rather than a real disk;
  [docs/pr-screenshots.md](../pr-screenshots.md) covers offscreen macOS
  rendering and the lab fallback. Never open a window on the operator's
  desktop. If nothing can show the change, say why in the body and ask
  the operator.
* **Scans are private data.** A real scan shows user names in paths,
  project and client names, volume names and sizes, and the chooser lists
  mounts and SSH hosts. Show any screenshot that is not 100% contrived to the
  operator and wait for approval before attaching it.
* **Titles are Conventional Commits:** `type(scope): what changed`, in the
  imperative, as the Pwr family's other repositories do. Scopes follow the
  [table of owners](where-changes-belong.md): `core`, `scan`,
  `ssh`, `removal`, `macos`, `windows`, `app`, `ci`, `release`, `docs`. A
  port of an upstream PR says so in the title, `(port of tobi/disktree#N)`,
  and keeps the upstream commits and their authorship.
* **Don't edit `CHANGELOG.md` in feature or fix PRs.** Every open PR that
  adds a line conflicts with every other one. Put the user-facing summary in
  the PR body; one release commit (`chore(release): vX.Y.Z`) writes the
  version's section from changes since the last tag and is pushed directly
  to `main` as a fast-forward update, without a release-prep PR. Follow
  [the release skill](../../.agents/skills/release/SKILL.md) when cutting a
  release.
* **PRs go to the fork, `pwrdrvr/PwrDisk`.** In a checkout with several
  remotes, push only to the one pointing there; `tobi/disktree` and personal
  forks are read-only from here.

## PwrAgent threads

* Before opening or attaching a PR, check that the thread's workspace holds
  its head commit. Attaching a PR does not move the workspace.
* Attach the thread's PR and watch it with `notifyOn: ["merged", "closed"]`,
  so the thread hears when it lands or is dropped.
* Work worth doing that this change leaves out goes on a PwrAgent to-do card
  (`add_todo`, with a `start_thread` action whose prompt stands alone), not a
  note in the PR body, a commit message or a code comment. The operator does
  not read those notes; a card can be tracked and started.
