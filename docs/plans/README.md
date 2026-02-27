# docker-gui — Planning Overview

> A lightweight, cross-platform Docker Desktop replacement focused on simplicity,
> native platform integration, and a clean developer experience.

## Goal

Build an open-source Docker GUI that works seamlessly on:

| Platform         | Docker runtime           |
|------------------|--------------------------|
| Windows (WSL 2)  | Docker Engine in WSL 2   |
| Linux (Ubuntu)   | Docker Engine (native)   |
| macOS            | Colima (or Rancher Desktop / OrbStack socket) |

The app is packaged as a native desktop application on every platform — no browser required, no extra daemon, no subscription.

## Documents in this folder

| File | Description |
|------|-------------|
| [tech-stack.md](tech-stack.md) | Technology choices and rationale |
| [architecture.md](architecture.md) | System and component architecture |
| [mvp-features.md](mvp-features.md) | MVP feature specifications |
| [platform-integration.md](platform-integration.md) | Per-platform setup and socket paths |
| [ui-ux-design.md](ui-ux-design.md) | UI/UX structure and visual design plan |

## Guiding Principles

1. **Single codebase** — one repo, one build pipeline, three platforms.
2. **Zero runtime dependencies for the user** — ship as a self-contained binary + webview bundle.
3. **Talk to Docker directly** — use the Docker Engine REST API over the Unix socket (or named pipe on WSL). No Docker CLI subprocess required for core operations.
4. **Compose is first-class** — `docker compose` workflows are not an afterthought.
5. **Familiar look** — mirror Docker Desktop's mental model (Containers / Images / Volumes / Compose stacks) so existing users feel at home.
6. **Extensible foundation** — MVP is deliberately narrow; architecture must not block adding volumes, networks, registries, build logs later.

## High-Level Milestones

```
M0  — Project scaffolding, CI, cross-platform build pipeline
M1  — Containers view: list, start, stop, remove, logs
M2  — Images view: list, pull, remove, inspect
M3  — Volumes view: list, create, remove, inspect
M4  — Compose stacks: list projects, up/down/restart per service
M5  — Polish: tray icon, auto-connect, settings, theme
```
