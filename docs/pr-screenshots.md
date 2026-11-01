# PR screenshots

`AGENTS.md` asks for before/after screenshots on any PR that changes what the
window shows. This file covers making them without leaking a real disk, and
attaching them with `gh` so the links render. The procedure is the Pwr
family's; PwrGit's `docs/pr-screenshots.md` and PwrAgent's `AGENTS.md` carry
the same rules.

## Making them

A disk scan is private by nature: paths carry user names, project and client
names; the status bar names the volume and its free space; the source chooser
lists mounts and the SSH hosts from your config. So point the app at a tree
built for the picture, never at `~` or a real volume.

Make them in the PwrSuiteLab headed E2E VM, never on the operator's Mac.
Launching PwrDisk on the operator's Mac and driving it with synthetic input
lands keys in whatever the operator is using; see
[AGENTS.md](../AGENTS.md#testing-on-a-desktop). Everything below runs inside
one guest job, and [headed-e2e-pwrsuitelab.md](headed-e2e-pwrsuitelab.md)
has the job's shape and an example. Without the PwrSuiteLab Control MCP,
stop and ask the operator.

1. In the job, create a disk image with an invented name and build the tree
   on it: `hdiutil create -ov -size 512m -fs APFS -volname Fixtures -attach
   target/fixtures.dmg` mounts it at `/Volumes/Fixtures`. Detach it at the
   end. The image is required in the lab: a tree anywhere else shows the
   guest's paths, volume name and free space, which are lab details. If
   `hdiutil` fails, stop and report it to the operator as a baseline issue.
2. On that image, build a contrived tree with invented names, sized so the
   treemap has something to show. Sparse files do not work, because sizes
   come from allocated blocks; write real bytes.

   ```sh
   root=/Volumes/Fixtures/Projects
   mkdir -p "$root"/{atlas/build,atlas/src,beacon/node_modules,photos/2024}
   # dd, not head -c 120m: macOS's head takes no size suffix.
   fill() { dd if=/dev/urandom of="$1" bs=1048576 count="$2" 2>/dev/null; }
   fill "$root/atlas/build/app.bin" 120
   fill "$root/beacon/node_modules/bundle.js" 40
   fill "$root/photos/2024/IMG_0001.raw" 80
   ```

3. Launch `pwrdisk <that root>` in the guest, in the background: with a path
   it scans directly and skips the chooser. Wait for its window, drive the
   screen under test from the job's command, and capture only the PwrDisk
   window (`screencapture -o -x -l <window id>`), not the screen. Write the
   images to a checkout-relative directory such as `target/lab-shots/`, list
   it in the job's `artifacts`, and quit the app.
4. Fetch them with `lab_e2e_collect`;
   [headed-e2e-pwrsuitelab.md](headed-e2e-pwrsuitelab.md) says where its
   `artifact_directory` is. Copy each image to a short local name, such as
   `./after.png`, and attach that copy; the artifact directory's own path
   never goes in a PR. Never commit the copy, and delete it once the
   uploaded links check out.
5. For "before", run the same job against a clean, committed worktree of
   `main` (`git fetch origin main && git worktree add --detach <path>
   origin/main`), with the same tree and the same window size. Remove it
   with `git worktree remove <path>` afterwards.

Lab VMs start with clipboard sharing off. Images come back as collected
artifacts, never through the clipboard, and the operator is never asked to
copy one out of the guest.

In the guest, the source chooser lists the guest's mounts and SSH config
hosts, which are lab details: never attach a guest capture of them. If the
change is to the chooser, say so in the body and ask the operator. Any other
screen that can only be shown with real data: **show the image to the
operator and wait for approval before attaching it.** Crop or redact user
names, host names, volume names and anything else that is not invented.

Say in the PR body how each image was made ("`pwrdisk` on a contrived tree on
a disk image named Fixtures, macOS 27, in the PwrSuiteLab VM"), without the
VM's name or any other lab detail. If nothing can show the change, including
a Linux or Windows window the lab cannot show, say why and ask the operator
before merging; a missing screenshot is not optional.

## Attaching them with `gh --attach`

GitHub CLI 2.99+ attaches images and videos with the repeatable `--attach`
flag on `gh pr create`, `gh pr edit` and `gh pr comment`. Reference the local
path in the Markdown body and `gh` uploads the file and rewrites the link to a
`https://github.com/user-attachments/assets/...` URL in place. Unreferenced
attachments are appended at the end. Check `gh --version` first.

Do not host images any other way: not on a side branch, a gist, or committed
to the repository. Those links break when the branch is deleted, and they put
binaries in the fork's history.

```sh
gh pr create --repo pwrdrvr/PwrDisk \
  --title "fix(app): keep tile labels inside their tile" \
  --body-file /tmp/pr-body.md \
  --attach ./before.png \
  --attach ./after.png
```

```markdown
## Before
![labels before](./before.png)

## After
![labels after](./after.png)
```

Alt text can follow the path after `#`, as in
`--attach './after.png#labels after'`; the `#alt` part is not matched.

Three ways this fails silently. `gh` exits 0 in every one of them:

* **The match is a literal string comparison.** `gh` rewrites `](path)` only
  when `path` is byte-identical to what `--attach` was given, so
  `--attach /tmp/shots/after.png` against `![after](./after.png)` does not
  match. The upload still happens, the dead relative link stays where it was,
  and a second copy is appended at the end. Use the same spelling in both
  places, and keep paths short enough to retype exactly.
* **After the upload, edit the live body, never your draft.** The rewrite
  exists only in the uploaded body; your local file keeps the local paths. For
  every later edit, run `gh pr view <n> --json body -q .body > <file>` and edit
  that copy. Re-sending the draft replaces the live `user-attachments` URLs
  with dead local paths. To add an image later, pass `--attach` for the new
  file only: existing `user-attachments` URLs match no `--attach` path, so
  `gh` leaves them alone, and attaching an uploaded image again appends a
  duplicate.
* **Check the result before calling the PR done.** Finding `user-attachments`
  URLs in the body is not enough; after a mismatch they are there, but in the
  wrong place. Every image link in the body must be a `user-attachments` URL,
  and each must fetch with HTTP 200 and the local file's byte count:

  ```sh
  gh pr view <n> --repo pwrdrvr/PwrDisk --json body -q .body \
    | grep -o '!\[[^]]*\]([^)]*)' | sed 's/.*](\(.*\))/\1/' \
    | while read -r url; do
        curl -sL -o /dev/null -w "%{http_code} %{size_download} $url\n" "$url"
      done
  ```

  To recover from a mismatch without uploading again, copy the URLs out of
  the body and `gh pr edit --body-file` with them where you wanted them.
