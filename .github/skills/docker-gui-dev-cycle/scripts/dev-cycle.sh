#!/usr/bin/env bash
set -euo pipefail

# Delegate to the canonical repo script.
exec "$(git rev-parse --show-toplevel)/scripts/dev-cycle.sh" "$@"
