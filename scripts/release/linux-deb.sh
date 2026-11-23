#!/usr/bin/env bash
# Package a native cargo xtask portable build on Debian or Ubuntu.
set -euo pipefail

if [[ $# -ne 3 ]]; then
    echo "Usage: $0 VERSION PORTABLE_BINARY OUTPUT_DIR" >&2
    exit 2
fi

version=$1
# Debian treats ~ as older than the final release with the same version.
deb_version=${version/-/\~}
dpkg --validate-version "$deb_version"
binary=$(realpath "$2")
output_dir=$(realpath -m "$3")
repo=$(cd "$(dirname "$0")/../.." && pwd)
arch=$(dpkg --print-architecture)
case "$arch" in
    amd64|arm64) ;;
    *) echo "Unsupported Debian architecture: $arch" >&2; exit 2 ;;
esac

stage=$(mktemp -d)
trap 'rm -r -- "$stage"' EXIT
cd "$stage"
pkg=debian/pwrdisk
mkdir -p "$pkg/DEBIAN"

# dpkg-shlibdeps needs a source stanza to look up the binary's linked
# libraries and their minimum versions on the native release runner.
cat > debian/control <<'EOF'
Source: pwrdisk
Section: utils

Package: pwrdisk
Architecture: any
EOF
install -Dm755 "$binary" "$pkg/usr/bin/pwrdisk"
depends=$(dpkg-shlibdeps -O -e"$pkg/usr/bin/pwrdisk")
depends=${depends#shlibs:Depends=}

install -Dm644 "$repo/assets/disktree.svg" \
    "$pkg/usr/share/icons/hicolor/scalable/apps/pwrdisk.svg"
install -Dm644 "$repo/LICENSE" "$pkg/usr/share/doc/pwrdisk/copyright"
install -Dm644 "$repo/THIRD_PARTY_LICENSES" \
    "$pkg/usr/share/doc/pwrdisk/THIRD_PARTY_LICENSES"
mkdir -p "$pkg/usr/share/applications"
sed -e 's|@BINDIR@|/usr/bin|g' -e "s|@VERSION@|$version|g" \
    "$repo/packaging/pwrdisk.desktop.in" \
    > "$pkg/usr/share/applications/pwrdisk.desktop"

# The graphics backend opens Vulkan, EGL and Wayland at runtime, so
# dpkg-shlibdeps cannot detect these libraries.
cat > "$pkg/DEBIAN/control" <<EOF
Package: pwrdisk
Version: $deb_version
Architecture: $arch
Section: utils
Priority: optional
Maintainer: PwrDrvr LLC <harold@pwrdrvr.com>
Installed-Size: $(du -sk "$pkg/usr" | cut -f1)
Depends: $depends, libegl1, libvulkan1, libwayland-client0, libwayland-egl1
Recommends: mesa-vulkan-drivers | vulkan-icd, openssh-client
Homepage: https://github.com/pwrdrvr/PwrDisk
Description: Explore disk usage with an interactive treemap
 Find what is using a volume, then mark and review files to remove.
EOF

mkdir -p "$output_dir"
dpkg-deb --root-owner-group --build "$pkg" \
    "$output_dir/PwrDisk-$version-linux-$arch.deb"
