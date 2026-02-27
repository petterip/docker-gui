---
name: docker-gui-dev-cycle
description: Use when you need to build, test, or run the docker-gui app locally (Angular frontend + Tauri desktop shell).
---

# Docker GUI Dev Cycle

Use this skill for local build/test/run tasks in this repository.

## Preconditions

- Run from repository root.
- Node version is from `.nvmrc`.
- `pnpm` is available.
- For desktop run: Rust toolchain and Tauri prerequisites are installed.

## Canonical commands

Prefer the script wrapper for consistency:

```bash
scripts/dev-cycle.sh install
scripts/dev-cycle.sh build
scripts/dev-cycle.sh test
scripts/dev-cycle.sh run-web
scripts/dev-cycle.sh run-tauri
scripts/dev-cycle.sh check
```

## Execution policy

- Use `build` for compile validation.
- Use `test` for unit tests in CI-like non-watch mode.
- Use `run-web` for fast frontend iteration.
- Use `run-tauri` when validating full desktop behavior.
- For quick validation before handoff, run `check`.

## Troubleshooting

- If `pnpm` is missing: install via Corepack or npm.
- If Tauri run fails: verify Rust and platform prerequisites, then retry `run-tauri`.
