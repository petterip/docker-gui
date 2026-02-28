#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/dev-cycle.sh <command>

Commands:
  install    Install JS deps
  build      Build frontend bundle
  test       Run unit tests once (non-watch)
  run-web    Run Angular dev server
  run-tauri  Run full Tauri desktop app
  check      Run build + test
USAGE
}

cmd="${1:-}"

if [[ -z "$cmd" ]]; then
  usage
  exit 1
fi

case "$cmd" in
  install)
    pnpm install
    ;;
  build)
    pnpm run build
    ;;
  test)
    pnpm run test --watch=false
    ;;
  run-web)
    pnpm run start
    ;;
  run-tauri)
    pnpm run tauri:dev
    ;;
  check)
    pnpm run build
    pnpm run test --watch=false
    ;;
  *)
    usage
    exit 2
    ;;
esac
