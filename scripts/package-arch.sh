#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$root"
npm run tauri -- build --no-bundle

cd "$root/packaging/arch"
makepkg --cleanbuild --force --nodeps
