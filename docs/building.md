# Building PwrDisk from source

Most people want the signed installers linked from the
[README](../README.md#install). This page is for building it yourself and
for working on it.

## Linux

```sh
git clone https://github.com/pwrdrvr/PwrDisk
cd PwrDisk
make install
```

`make install` builds a release binary and puts three things under `~/.local`
(no root needed):

- `~/.local/bin/pwrdisk`
- a desktop entry, so PwrDisk is in the launcher and in a file manager's
  **Open with** for a directory (it adds a handler; it never becomes the
  default)
- an icon

`sudo make install PREFIX=/usr/local` installs system-wide; `make uninstall`
removes exactly what was installed.

If another installed version wins on `PATH`, the installer warns which one.
Run `export PATH="$HOME/.local/bin:$PATH"` and add that setting to your shell
configuration so the locally installed build takes precedence.

For Linux builds with embedded SSH helpers, also install the two Rust
standard-library targets (no external C cross-compiler is needed):

```sh
rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
```

You need Rust 1.97 or newer and a Wayland or X11 session with a GPU that GPUI
can drive (Vulkan). Distributions often package an older Rust;
[rustup](https://rustup.rs) installs a current one. The repo pins 1.97 in
`rust-toolchain.toml`, so with rustup the right toolchain is fetched on the
first build even if `rustup default` points at something older.

## macOS

Build it with Rust 1.97 or newer and Xcode or its Command Line Tools.
macOS does not come with Rust; install it with [rustup](https://rustup.rs).

```sh
make install     # ~/Applications/PwrDisk.app, and ~/.local/bin/pwrdisk
make uninstall
```

`make bundle` (or `cargo xtask bundle`) builds `target/bundle/PwrDisk.app`,
a zip of it and a DMG laid out like the release's, signed ad hoc; add
`--universal` for both architectures. To sign and notarize a build for
others, with a Developer ID certificate in the keychain and credentials
saved by `xcrun notarytool store-credentials`:

```sh
NOTARY_PROFILE=<profile> cargo xtask bundle \
  --sign "Developer ID Application: Name (TEAMID)" --notarize
```

A build that was not signed and notarized (one you made yourself and copied
to another Mac, say) is stopped by Gatekeeper: macOS says it "is damaged
and can't be opened" or "cannot be verified". Clear the mark once:

```sh
xattr -dr com.apple.quarantine /Applications/PwrDisk.app
```

(Or open it once, then choose **Open Anyway** in System Settings › Privacy &
Security.)

## Windows

Build it with Rust 1.97 or newer, from
[rustup](https://rustup.rs), and the MSVC toolchain (Visual Studio Build
Tools, C++ workload):

```powershell
git clone https://github.com/pwrdrvr/PwrDisk
cd PwrDisk
cargo build --release    # target\release\pwrdisk.exe
```

## Develop

```sh
make run      # release build, opening the source/root chooser
make lint     # rustfmt --check, then clippy with every warning an error
make test     # scanner, layout and removal tests, plus window-harness tests
make test-ssh # real localhost SSH scans/errors; Linux, bun + openssh required
make ci       # lint, then test
```

The lint gate is strict on purpose: `clippy::all` and `clippy::pedantic` are
errors, and every exception is written down with its reason in `Cargo.toml`.
The window-harness tests draw real frames and press real keys — including one
that marks a directory, confirms the deletion and checks that the files are
gone while their neighbours are not.

| path | what lives there |
| --- | --- |
| `crates/disktree-core` | scanning, the tree, the squarified layout, free space and removal — no UI |
| `crates/disktree-mcp` | `pwrdisk mcp`: the agent's tools and skills over stdio, on `disktree-core` |
| `crates/disktree-link` | the socket between a window and an agent, and its messages |
| `skills/` | the skills `pwrdisk mcp` compiles in: explain usage, free space, and each system's cautions |
| `crates/disktree-app/src/state.rs` | every action the interface can take, and the key map |
| `crates/disktree-app/src/views.rs` | the screens |
| `crates/disktree-app/src/treemap_view.rs` | painting the mosaic and its labels |
| `crates/disktree-app/src/ui.rs` | the spacing, type and size scale, in `rem` |
| `crates/disktree-app/src/tests.rs` | end-to-end tests through a real window |
| `packaging/`, `assets/`, `Makefile` | the desktop entry, the icon, and install |

The interface follows the
[GPUI Kit design guides](https://gpui-kit.com/versions/main/docs/design-guides/):
every size is on one `rem` scale so interface zoom keeps its proportions,
primary is reserved for what Enter does, and the only question the app asks is
the one it cannot take back.
