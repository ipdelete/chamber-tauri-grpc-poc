#!/usr/bin/env bash
set -euo pipefail

kind="${1:-}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$root"
if [[ -n "$kind" ]]; then
  export CHAMBER_SIDECAR_KIND="$kind"
fi

npm run tauri -- build --no-bundle

if [[ "$kind" == "go" ]]; then
  cd "$root/packaging/arch-go"
elif [[ "$kind" == "rust" ]]; then
  cd "$root/packaging/arch-rust"
else
  cd "$root/packaging/arch"
fi

makepkg --cleanbuild --force --nodeps

