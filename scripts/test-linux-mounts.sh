#!/bin/sh
# Pass the compiled disktree-core unit test binary as the first argument.
# All mounts and writes live in an isolated, temporary /run.
set -eu
exec unshare --user --map-root-user --mount sh -eu -c '
    mount --make-rprivate /
    mount -t tmpfs disktree-test-run /run
    mkdir -p /run/media/disktree-test /run/disktree-scan
    mount -t tmpfs tmpfs /run/media/disktree-test
    mkdir /run/media/disktree-test/nested
    mount -t tmpfs tmpfs /run/media/disktree-test/nested
    for dir in original alias one two; do mkdir /run/disktree-scan/$dir; done
    mkdir /run/disktree-scan/original/nested
    mount --bind /run/disktree-scan/original /run/disktree-scan/alias
    for dir in one two; do mount -t tmpfs tmpfs /run/disktree-scan/$dir; done
    for dir in original one two; do printf data > /run/disktree-scan/$dir/file; done
    mkdir -p /run/disktree-layers/older /run/disktree-layers/newer /run/disktree-layers/peer
    mount -t tmpfs "$(printf "disktree-layer-\\200")" /run/disktree-layers/older
    mount -t tmpfs hidden-layer /run/disktree-layers/newer
    mkdir /run/disktree-layers/newer/hidden
    mount -t tmpfs hidden-child /run/disktree-layers/newer/hidden
    mount -t tmpfs "$(printf "disktree-layer-\\201")" /run/disktree-layers/peer
    mount --move /run/disktree-layers/older /run/disktree-layers/newer
    "$1" --exact space::tests::moved_layers_and_source_bytes_match_the_kernel --ignored --nocapture
    "$1" --exact removal::tests::mounted_media_and_repeated_views --ignored --nocapture
' sh "$1"
