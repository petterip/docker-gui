# Agent Instructions

## Build, test, run

Use `scripts/dev-cycle.sh` from repo root:

- `scripts/dev-cycle.sh install`
- `scripts/dev-cycle.sh build`
- `scripts/dev-cycle.sh test`
- `scripts/dev-cycle.sh run-web`
- `scripts/dev-cycle.sh run-tauri`
- `scripts/dev-cycle.sh check`

## Workflow

- Prefer `check` before handoff.
- Keep command usage non-interactive unless explicitly requested.
- If a command fails, report the first actionable root cause and retry after fix.
