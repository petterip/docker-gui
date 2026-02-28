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

macOS users: Homebrew is the recommended package manager for installing prerequisites.

### Install Rust/Cargo

`cargo` is installed as part of the Rust toolchain.

Windows (WSL):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
cargo --version
```

macOS (Homebrew-first):

```bash
brew install node pnpm rustup-init
rustup-init -y
source "$HOME/.cargo/env"
cargo --version
```

Also install Apple command line build tools if missing:

```bash
xcode-select --install
```

Ubuntu:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
cargo --version
```

If you installed Rust in a previous shell, open a new terminal (or run `source "$HOME/.cargo/env"`) before running `pnpm tauri build`.

### Linux system dependencies for Tauri (Ubuntu/WSL)

Install required native packages before running Tauri build/dev commands:

```bash
sudo apt-get update
sudo apt-get install -y \
  libgtk-3-dev \
  libwebkit2gtk-4.1-dev \
  libsoup-3.0-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  patchelf
```

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

## Tauri config troubleshooting

- Tauri v2 validates `src-tauri/tauri.conf.json` against the v2 schema.
- `app.windows[].icon` is not valid in v2 and causes:
  - `Additional properties are not allowed ('icon' was unexpected)`
- Define app icons under `bundle.icon` instead (already configured in this repo).
