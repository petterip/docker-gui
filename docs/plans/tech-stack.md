# Tech Stack

## Decision Summary

| Layer | Choice | Alternatives considered |
|-------|--------|------------------------|
| Desktop shell | **Tauri v2** | Electron, Flutter, NW.js |
| Frontend framework | **Angular 21 + TypeScript** | Vue 3, Svelte, React |
| UI component library | **Spartan UI + Tailwind CSS v4** | Angular Material, PrimeNG |
| Build / bundler | **Angular CLI 21** (esbuild + Vite dev server) | Webpack, analog.js |
| Backend / Rust core | **Tauri commands + bollard** | Raw subprocess calls to Docker CLI |
| Docker API client | **bollard** (async Rust) | dockerode (Node), direct fetch over socket |
| Compose orchestration | **`docker compose` CLI subprocess** | libyaml parse + custom REST calls |
| State management | **Angular Signals** (built-in) | NgRx, NGXS |
| Data fetching / polling | **`@tanstack/angular-query` v5** | RxJS interval + toSignal, SWR |
| Testing — frontend | **Jest + jest-preset-angular** | Karma/Jasmine, Vitest |
| Testing — Rust | **built-in `#[cfg(test)]` + mockall** | — |
| Linting / formatting | **ESLint v9 flat config + Prettier, clippy** | Biome |
| Packaging / signing | **Tauri bundler** (MSI/NSIS, AppImage/deb, dmg) | electron-builder |

---

## Why Tauri v2 (not Electron)

| Concern | Electron | Tauri |
|---------|----------|-------|
| Bundle size | ~120 MB+ (ships Chromium) | ~10–15 MB (uses OS WebView) |
| Memory footprint | ~200 MB idle | ~30–50 MB idle |
| Native APIs | Node.js child_process / IPC | Rust — full `std`, direct syscalls |
| Unix socket access | Node.js `net.createConnection` | `bollard` on tokio — zero overhead |
| WSL 2 support | Works but heavy | Works via WSLg or X11 forwarding, same binary |
| macOS notarization | Complex | First-class in Tauri toolchain |
| Cross-compile | Windows needs VM | `cross` crate or GitHub Actions matrix |

Tauri v2 ships a stable multi-window API and an improved permissions model that fits this project well.

---

## Why bollard (not Docker CLI subprocess)

`bollard` speaks the Docker Engine HTTP API directly over the Unix socket (`/var/run/docker.sock`).

Advantages:
- No dependency on `docker` CLI being in `PATH` — only the socket is needed.
- Streaming log lines via async Rust futures — no child-process stdout buffering.
- Structured JSON responses, fully typed via generated structs.
- Works identically on Linux, macOS (Colima socket), and WSL 2.

Compose operations (up / down / restart) **do** shell out to `docker compose` because the Compose engine is not exposed via the Engine API — it lives in the CLI plugin. This is acceptable for MVP.

---

## Why Angular 21 + Spartan UI

- **Angular Signals** (stable since Angular 17, fully mature in v21) replace external state libraries entirely — `signal()`, `computed()`, and `effect()` cover everything Zustand did, with zero extra dependencies.
- **Spartan UI** is the Angular equivalent of shadcn/ui: copy-owned primitives built on Tailwind CSS, no opaque library upgrade path.
- **Tailwind CSS v4** works natively with Angular CLI — just add the PostCSS plugin.
- **RxJS** (included with Angular) maps directly onto Tauri event streams: `fromEventPattern(() => listen('container-log', cb))` turns Docker log lines into an Observable — cleaner than React's `useEffect` cleanup pattern.
- **`@tanstack/angular-query`** provides the same polling and mutation semantics as the React version, with Angular-idiomatic signal-based result accessors.
- Angular's **strict template type-checking** and **standalone components** (default in v21) give strong compile-time guarantees across the whole UI.

---

## Monorepo Layout

```
docker-gui/
├── src-tauri/              # Rust Tauri backend
│   ├── src/
│   │   ├── main.rs
│   │   ├── commands/       # Tauri command handlers — one file per domain:
│   │   │   ├── containers.rs   #   list, start, stop, restart, remove
│   │   │   ├── images.rs       #   list, pull (streaming), remove
│   │   │   ├── volumes.rs      #   list, create, remove
│   │   │   └── compose.rs      #   list stacks, up, down, restart, logs
│   │   ├── config.rs       # socket resolution + compose binary detection → AppState
│   │   ├── error.rs        # AppError enum (thiserror + serde discriminant tag)
│   │   └── registry.rs     # StacksRegistry (Mutex<Vec<Stack>>, atomic JSON flush)
│   ├── Cargo.toml
│   └── tauri.conf.json
├── src/                    # Angular frontend
│   ├── main.ts
│   ├── app/
│   │   ├── app.component.ts
│   │   ├── app.routes.ts
│   │   ├── components/     # Shared UI: ConfirmRow, RelativeTime, Spartan UI wrappers
│   │   ├── views/          # Page-level standalone components (one per sidebar item)
│   │   ├── stores/         # Angular Signal stores: ConnectionStore, UiStore, LogStore
│   │   └── lib/            # log-stream.service.ts, typed models, tauri invoke helpers
│   └── styles.css          # Tailwind CSS v4 entry
├── docs/
│   └── plans/
├── package.json
├── angular.json            # Angular CLI config (build target: esbuild)
├── tailwind.config.ts
└── tsconfig.json
```

---

## Runtime Requirements (end-user machine)

| Platform | Required |
|----------|----------|
| Windows (WSL 2) | WSL 2 with Ubuntu, Docker Engine installed inside WSL, WSLg for GUI |
| Ubuntu | Docker Engine (`docker.socket`), optional: user in `docker` group |
| macOS | Colima started (`colima start`), socket at `~/.colima/default/docker.sock` |

The app binary itself ships with no runtime dependencies beyond the OS WebView (WebView2 on Windows, WebKitGTK on Linux, WKWebView on macOS).
