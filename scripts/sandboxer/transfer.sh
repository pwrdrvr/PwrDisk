#!/usr/bin/env bash
# Runs on the laptop. Talks to scripts/sandboxer/transfer.py in a Sandboxer
# sandbox through the URL the MCP's host_port returned
# (https://<host>?_token=<token>). The procedure is in
# docs/sandboxer.md.
#
#   scripts/sandboxer/transfer.sh send URL        bundle HEAD, upload it as <sha>.bundle,
#                                                 print the sha
#   scripts/sandboxer/transfer.sh get URL NAME DEST
#                                                 download ~/pwrdisk/files/NAME
#
# Git bundles include unpublished commits without a GitHub token.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

usage() {
  sed -n '2,13p' "$0" >&2
  exit 2
}

# https://host?_token=t + name -> https://host/name?_token=t
file_url() {
  local base="${1%%\?*}" query=""
  [[ "$1" == *\?* ]] && query="?${1#*\?}"
  printf '%s/%s%s' "${base%/}" "$2" "$query"
}

case "${1:-}" in
  send)
    [[ $# -eq 2 ]] || usage
    if [[ -n "$(git status --porcelain)" ]]; then
      echo "uncommitted changes: the sandbox verifies HEAD, not the working tree" >&2
      exit 1
    fi
    sha="$(git rev-parse HEAD)"
    mkdir -p .local/sandboxer
    bundle=".local/sandboxer/$sha.bundle"
    trap 'rm -f "$bundle"' EXIT
    git bundle create -q "$bundle" HEAD
    code="$(curl -fsS --retry 3 -o /dev/null -w '%{http_code}' -T "$bundle" "$(file_url "$2" "$sha.bundle")")"
    if [[ "$code" != 201 ]]; then
      echo "upload answered HTTP $code, not 201" >&2
      exit 1
    fi
    echo "$sha"
    ;;
  get)
    [[ $# -eq 4 ]] || usage
    [[ "$3" =~ ^[a-zA-Z0-9][a-zA-Z0-9._-]*$ ]] || usage
    tmp="$(mktemp "$4.partial.XXXXXX")"
    trap 'rm -f "$tmp"' EXIT
    # The proxy answers a GET carrying _token with a redirect that sets an
    # auth cookie: follow it with the cookie engine on, holding nothing on disk.
    if ! curl -fsSL -b /dev/null --retry 3 -o "$tmp" "$(file_url "$2" "$3")"; then
      rm -f "$tmp"
      exit 1
    fi
    mv "$tmp" "$4"
    ;;
  *) usage ;;
esac
