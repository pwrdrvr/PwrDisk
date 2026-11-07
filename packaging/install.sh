#!/bin/sh
# Install a PwrDisk release: the binary, a desktop entry and an icon.
#
#   ./install.sh                  # into ~/.local, no root needed
#   sudo PREFIX=/usr/local ./install.sh
#
# The same three files `make install` puts in place, for a machine without a
# Rust toolchain.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
PREFIX=${PREFIX:-"$HOME/.local"}
BINDIR=${BINDIR:-"$PREFIX/bin"}
APPDIR=${APPDIR:-"$PREFIX/share/applications"}
ICONDIR=${ICONDIR:-"$PREFIX/share/icons/hicolor/scalable/apps"}
VERSION=$(cat "$here/VERSION")

install -d "$BINDIR" "$APPDIR" "$ICONDIR"
install -m755 "$here/pwrdisk" "$BINDIR/pwrdisk"
install -m755 "$here/pwrdisk-tui" "$BINDIR/pwrdisk-tui"
install -m644 "$here/disktree.svg" "$ICONDIR/pwrdisk.svg"
sed -e "s|@BINDIR@|$BINDIR|" -e "s|@VERSION@|$VERSION|" \
    "$here/pwrdisk.desktop.in" > "$APPDIR/pwrdisk.desktop"
chmod 644 "$APPDIR/pwrdisk.desktop"
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APPDIR" 2>/dev/null || true
fi

echo "installed PwrDisk $VERSION:"
echo "  $BINDIR/pwrdisk"
echo "  $BINDIR/pwrdisk-tui"
echo "  $APPDIR/pwrdisk.desktop"
echo "  $ICONDIR/pwrdisk.svg"
resolved=$(command -v pwrdisk 2>/dev/null || true)
if [ -z "$resolved" ] || ! [ "$resolved" -ef "$BINDIR/pwrdisk" ]; then
    echo
    if [ -n "$resolved" ]; then
        echo "warning: pwrdisk still resolves to $resolved"
        echo "         that hides the build in $BINDIR"
    else
        echo "warning: $BINDIR is not on PATH"
    fi
    printf '%s\n' "Run in your shell: export PATH=\"$BINDIR:\$PATH\""
    echo "Then add that PATH setting to your shell configuration."
fi
