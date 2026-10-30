#!/usr/bin/env bash
# Run PwrDisk's Linux checks in a Sandboxer VM. See docs/sandboxer.md.
set -uo pipefail

usage() {
  echo "usage: $0 {setup|lint|test [cargo test args]|build|e2e|all}" >&2
}

step="${1:-}"
(( $# )) && shift
case "$step" in
  setup|lint|build|e2e|all) (( $# == 0 )) || { usage; exit 2; } ;;
  test) ;;
  *) usage; exit 2 ;;
esac
if [[ "$(uname -s)" != Linux ]]; then
  echo "run this script in the Linux sandbox; see docs/sandboxer.md" >&2
  exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root" || exit 1
out_dir="$repo_root/.local/sandboxer"
mkdir -p "$out_dir"
# The checkout, target directory and status file belong to one run.
exec 9> "$out_dir/lock"
if ! flock -n 9; then
  echo "another verification run holds $out_dir/lock" >&2
  exit 3
fi
if [[ -n "$(git status --porcelain)" ]]; then
  echo "uncommitted changes: commit before remote verification" >&2
  exit 2
fi
export PATH="$HOME/.cargo/bin:$HOME/.bun/bin:$PATH"
export CARGO_TERM_COLOR=never
# Keep the VM's cache smaller. These profiles differ from a local build.
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
unset GH_TOKEN GITHUB_TOKEN DISKTREE_PROFILE_ROOT

setup() {
  # GPUI libraries match .github/workflows/ci.yml. SSH checks need Bun,
  # OpenSSH and both static scanner targets used by xtask portable.
  local packages=(
    build-essential pkg-config curl ca-certificates git python3 unzip
    util-linux openssh-client openssh-server
    libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev
    libxcb1-dev libfontconfig-dev libfreetype-dev libvulkan-dev
    libasound2-dev libssl-dev libzstd-dev
  )
  if ! dpkg -s "${packages[@]}" > /dev/null 2>&1; then
    sudo env DEBIAN_FRONTEND=noninteractive apt-get update -qq || return 1
    sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
      --no-install-recommends "${packages[@]}" || return 1
  fi
  if ! command -v rustup > /dev/null; then
    curl -fsSL https://sh.rustup.rs | sh -s -- \
      -y --profile minimal --default-toolchain none || return 1
  fi
  # Read the channel and components from this commit's rust-toolchain.toml.
  rustup toolchain install || return 1
  rustup target add x86_64-unknown-linux-musl \
    aarch64-unknown-linux-musl || return 1
  if ! command -v bun > /dev/null; then
    curl -fsSL https://bun.sh/install | bash || return 1
  fi
  # Ubuntu's sshd requires this directory even for the unprivileged test.
  sudo install -d -m 0755 /run/sshd || return 1
  cargo -V && rustc -V && bun -v && cargo fetch --locked
}

run_step() {
  local name="$1" start code
  shift
  start="$(date +%s)"
  case "$name" in
    setup) setup ;;
    lint) cargo xtask lint ;;
    test)
      if (( $# )); then
        cargo test --locked "$@"
      else
        cargo xtask test
      fi
      ;;
    build) cargo xtask portable ;;
    e2e) bun scripts/test-ssh.ts ;;
  esac > "$out_dir/$name.log" 2>&1
  code=$?
  echo "$name exit=$code secs=$(($(date +%s) - start))" \
    | tee -a "$out_dir/status"
  return "$code"
}

# Tie every status file to its source. Never check out while this runs.
git rev-parse HEAD > "$out_dir/status" || exit 1
if [[ "$step" == all ]]; then
  failed=0
  if run_step setup; then
    run_step lint || failed=1
    run_step test || failed=1
    run_step build || failed=1
    run_step e2e || failed=1
  else
    failed=1
  fi
  echo "all exit=$failed" | tee -a "$out_dir/status"
  exit "$failed"
fi
run_step "$step" "$@"
