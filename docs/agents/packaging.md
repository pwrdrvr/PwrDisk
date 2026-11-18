# Installing, bundling and releasing

<!-- Moved verbatim from AGENTS.md, which keeps a summary and says when
     to read this. -->

Read this before changing installation, the desktop entry, the macOS
bundle or its icon, release packaging, or how a release is cut.

`make install` is the supported way to put this on a machine: it installs the
release binary, `packaging/pwrdisk.desktop.in` (rendered with the real install
prefix and the crate version) and `assets/disktree.svg`. Keep the desktop
entry's `Categories` to a single main category plus additional ones, or
`desktop-file-validate` complains.

On macOS `make install` installs `target/bundle/PwrDisk.app` into
`~/Applications` instead, built by `cargo xtask bundle` from
`packaging/macos/` (the `Info.plist` template and the icon's SVG). The bundle
identifier there keys every macOS permission grant; do not change it lightly.

The macOS icon has two sources in `packaging/macos/`, and each macOS reads
the one made for it. `AppIcon.svg` is the mark on Apple's legacy 824-in-1024
tile; the bundle draws `AppIcon.icns` from it, which macOS 15 and earlier
draw literally. `AppIcon.icon` with `AppGlyph.svg` is the Icon Composer
package: when Xcode 26's actool is at hand `cargo xtask bundle` compiles it
into `Assets.car` and names it with `CFBundleIconName`, and macOS 26 draws
that with its own shape, glass edge and Dark, Clear and Tinted looks. Handed
only the `.icns`, macOS 26 normalizes it instead, and 26.6.2 has drawn such a
padded icns on a light plate, so a bundle without the catalog says so. A run
outside the bundle (`make run`, `cargo run`) has no icon for macOS to draw,
so `dock_icon.rs` paints one: `DockIcon.png`, or on macOS 26
`DockIcon-glass.png`, the shipped icon as that macOS itself draws it, since
Liquid Glass is never applied to an image set at run time. `make dock-icons`
redraws both on a macOS 26 with Xcode 26; run it after either SVG changes.

Releases are cut by `release.yml` from a `vX.Y.Z` tag that matches the
workspace version and a `## vX.Y.Z` section of `CHANGELOG.md` (`cargo xtask
release-check`); `docs/releasing.md` is the procedure and
`.github/workflows/README.md` the design. macOS ships a DMG and Windows a
Velopack Setup.exe, MSI and portable zip, both of which update themselves
from GitHub Releases. The `velopack` crate pin and `VPK_VERSION` in
`scripts/release/install-vpk.sh` move together.
