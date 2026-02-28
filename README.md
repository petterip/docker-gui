# docker-gui

`docker-gui` is a local desktop UI for Docker Engine (containers, images, volumes, and compose stacks).

It is intended for teams that want a Docker Desktop-style daily workflow while connecting directly to a local Docker Engine daemon.

## What this replaces (and what it does not)

`docker-gui` can replace common day-to-day Docker Desktop UI tasks:
- Browse and manage containers
- Pull/remove images
- Create/remove volumes
- View and operate compose stacks
- Stream logs

`docker-gui` does not replace all Docker Desktop platform features. It is a focused Docker Engine UI.

## Architecture

- Frontend: Angular
- Desktop shell: Tauri
- Backend runtime: Rust + Docker Engine API (`bollard`)
- Compose operations: executed via local `docker compose` / `docker-compose` CLI

## Prerequisites

- Docker Engine available locally (for example via Colima on macOS, Docker Engine on Linux, or WSL Docker Engine on Windows)
- Node.js version from `.nvmrc`
- `pnpm`
- Rust toolchain (for Tauri desktop runs/builds)

## Quick start

1. Install dependencies

```bash
pnpm install
```

2. Run web UI (frontend only)

```bash
pnpm run start
```

3. Run desktop app (recommended for real usage)

```bash
pnpm run tauri:dev
```

## Build and test

Build:

```bash
pnpm run build
```

Unit tests:

```bash
pnpm run test -- --watch=false
```

Or use the unified helper:

```bash
scripts/dev-cycle.sh check
```

## How to use as a Docker Desktop alternative (Engine-first)

1. Start your Docker Engine (example: `colima start` on macOS).
2. Start `docker-gui` (`pnpm run tauri:dev` for local development).
3. Open **Settings** and verify detected Docker connection/socket path.
4. Use **Containers / Images / Volumes / Compose** views for normal operations.
5. Keep CLI workflows (`docker`, `docker compose`) for advanced or unsupported scenarios.

## Repository structure

- `src/`: Angular UI
- `src-tauri/`: Rust/Tauri backend and Docker command integrations
- `docs/`: architecture, implementation notes, and roadmap plans

## Notes

- Compose control requires a working compose binary in `PATH`.
- If Docker is not reachable, the app will show disconnected status and command failures until the engine is available.
