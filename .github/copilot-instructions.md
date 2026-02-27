# docker-gui Copilot instructions

Use the repo script `scripts/dev-cycle.sh` for build/test/run workflows.

Preferred command mapping:
- Install deps: `scripts/dev-cycle.sh install`
- Build: `scripts/dev-cycle.sh build`
- Test (non-watch): `scripts/dev-cycle.sh test`
- Run web app: `scripts/dev-cycle.sh run-web`
- Run desktop app: `scripts/dev-cycle.sh run-tauri`
- Pre-handoff validation: `scripts/dev-cycle.sh check`

When changing behavior, update or add tests where practical.
Keep commands non-interactive unless the user explicitly asks for interactive mode.
