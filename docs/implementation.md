# docker-gui — Complete Implementation Record

A full account of what was built, every decision made, all bugs found and fixed, and the lessons that emerged. Written retrospectively after the M0 milestone was completed and audited.

---

## Table of Contents

1. [Project Objective](#1-project-objective)
2. [Technology Stack — Choices and Rationale](#2-technology-stack--choices-and-rationale)
3. [Repository Structure](#3-repository-structure)
4. [Rust Backend — Implementation](#4-rust-backend--implementation)
5. [Angular Frontend — Implementation](#5-angular-frontend--implementation)
6. [IPC Layer — The Bridge](#6-ipc-layer--the-bridge)
7. [Build & CI Pipeline](#7-build--ci-pipeline)
8. [Bugs Found in Audit and How They Were Fixed](#8-bugs-found-in-audit-and-how-they-were-fixed)
9. [Lessons Learned](#9-lessons-learned)
10. [What M1 Should Address](#10-what-m1-should-address)

---

## 1. Project Objective

Build a native desktop GUI for Docker that feels like a first-class application, not a web app wrapped in a browser tab. The primary audience is developers who work with Docker on macOS (Colima), Linux, and WSL 2. The design targets:

- Zero Chromium overhead — use the OS WebView (Tauri).
- No Docker CLI required for read operations — talk to the socket directly (bollard).
- Clean, dark-native UI with a sidebar layout matching OS design conventions.
- Feature parity with basic Docker Desktop views: containers, images, volumes, compose stacks, settings.

The M0 milestone is the walking skeleton: every view is functional for basic CRUD, the app ships as a single binary, and CI produces signed artifacts for all three platforms.

---

## 2. Technology Stack — Choices and Rationale

### 2.1 Tauri v2

Tauri packages a Rust binary and an OS WebView into a single distributable. The key advantages over Electron:

| Concern | Electron | Tauri v2 |
|---------|----------|----------|
| Bundle size | ~120 MB+ (ships Chromium) | ~10–15 MB (uses OS WebView) |
| Memory at idle | ~200 MB | ~30–50 MB |
| Native API layer | Node.js | Rust — full `std`, direct syscalls |
| Unix socket access | `net.createConnection` | bollard on tokio — zero overhead |
| macOS notarization | Complex third-party tools | First-class in Tauri toolchain |

Tauri v2 (not v1) was chosen specifically because v2 stabilised:
- A proper permission and capability system for IPC.
- Multi-window support.
- First-class `tauri-plugin-shell`, `tauri-plugin-fs`, `tauri-plugin-dialog`.

### 2.2 Rust + bollard

`bollard` speaks the Docker Engine HTTP/JSON API directly over the Unix socket. This means:

- No `docker` CLI binary in PATH required for listing, inspecting, starting, or stopping.
- Log and pull-progress streams are native async Rust futures — no buffering artifacts.
- Fully typed responses — `bollard::models::ContainerSummary`, `ImageSummary`, `Volume`, etc.

The tradeoff: Compose operations (`up`, `down`, `restart`) cannot go through bollard because the Compose engine is a CLI plugin, not part of the Engine API. Those calls shell out via `std::process::Command` and stream stdout/stderr back to the frontend via Tauri events.

**bollard 0.18 breaking change** (learned the hard way): many fields that were `String` in 0.17x became `Option<String>` in 0.18. `ImageSummary::repo_tags` is `Option<Vec<String>>`. `Volume::created_at` is `Option<BollardDate>`. Callers must `.unwrap_or_default()` or handle `None` explicitly — the compiler enforces this.

### 2.3 Angular 21

Angular 21 with fully standalone components (no NgModules) was chosen over Vue or React for these reasons:

- **Signal-based reactivity** is built in (stable since v17, fully mature in v21). No external store library is needed. `signal()`, `computed()`, and `effect()` cover all state management at M0 scope.
- **Strict template type-checking** catches bugs at compile time that would surface only at runtime in loosely typed JSX.
- **RxJS** (already a peer dependency) maps directly onto Tauri event streams: `listen('container-log-{id}', handler)` turns Docker log lines into an Observable, which is exactly what `LogStreamService` does.
- **Angular CLI** with esbuild (default in v17+) gives fast builds with zero Webpack config.

### 2.4 @tanstack/angular-query v5

TanStack Angular Query was chosen for server state (Docker resource listings) instead of raw RxJS intervals because:

- Automatic background refetching at a configurable `refetchInterval`.
- Query invalidation on mutation success flushes stale data.
- Deduplication — multiple components subscribing to `['containers']` result in one backend call.
- The `injectQuery()` / `injectMutation()` pattern works cleanly inside Angular signal-based components.

The experimental package `@tanstack/angular-query-experimental` is the current distribution name; this will change when the Angular adapter exits experimental status.

**Important constraint**: `injectQuery()` and `injectMutation()` must be called in the constructor (or field initialiser context), not in lifecycle hooks or methods. This is because they use Angular's `inject()` internally, which is only valid during construction.

### 2.5 Tailwind CSS v4

Tailwind v4 is imported as a single PostCSS directive: `@import "tailwindcss"` in `styles.css`. No `tailwind.config.js` file is needed. The new oxide engine scans templates automatically. This simplified the setup significantly compared to v3.

The custom design system (CSS variables for `--bg`, `--text`, `--border`, `--surface`, `--accent`, etc.) lives in `styles.css` and is toggled by a `data-theme="dark"` attribute on `<html>`, managed by `UiStore`.

---

## 3. Repository Structure

```
docker-gui/
├── src/                        Angular source root
│   └── app/
│       ├── app.ts              Root component (Angular 21 naming convention)
│       ├── app.html            Root template
│       ├── app.config.ts       ApplicationConfig — bootstraps QueryClient, router
│       ├── app.routes.ts       Lazy-loaded route definitions
│       ├── components/         Shared UI components
│       │   ├── confirm-row.component.ts/.html
│       │   ├── toast.service.ts
│       │   └── toast-container.component.ts/.html
│       ├── lib/
│       │   ├── models.ts       All shared TypeScript interfaces
│       │   ├── tauri.ts        Typed invoke wrapper + errorMessage helper
│       │   └── log-stream.service.ts   Tauri event → RxJS Observable bridge
│       ├── stores/
│       │   ├── connection.store.ts    Docker connection status (signal store)
│       │   ├── ui.store.ts            Theme toggle (signal store)
│       │   └── log.store.ts           Per-component streaming log buffer
│       └── views/
│           ├── containers/     Container list + detail + log viewer
│           ├── images/         Image list + pull + inspect
│           ├── volumes/        Volume list + create + inspect
│           ├── compose/        Compose stacks + service table + log viewer
│           └── settings/       Docker info + theme + reconnect
│
├── src-tauri/
│   ├── src/
│   │   ├── lib.rs              App bootstrap — registers state, plugins, commands
│   │   ├── config.rs           AppState: socket resolver, Docker client, compose binary
│   │   ├── error.rs            AppError enum — serde-tagged for IPC transport
│   │   ├── registry.rs         Persistent JSON store for registered compose stacks
│   │   └── commands/
│   │       ├── containers.rs
│   │       ├── images.rs
│   │       ├── volumes.rs
│   │       ├── compose.rs
│   │       └── system.rs
│   ├── capabilities/default.json   Tauri v2 permissions
│   ├── tauri.conf.json
│   └── Cargo.toml
│
├── docs/
│   └── plans/                  Original planning documents
│
├── .github/workflows/build.yml Multi-platform CI
├── .cargo/config.toml          Linker overrides for cross-compilation
├── rust-toolchain.toml         Pinned stable toolchain
├── postcss.config.js           Tailwind v4 PostCSS plugin
└── package.json
```

---

## 4. Rust Backend — Implementation

### 4.1 AppState and Startup (`config.rs`, `lib.rs`)

`AppState` holds:
- `docker: Arc<Mutex<Option<Docker>>>` — `None` means Docker socket not found.
- `socket_path: String` — the resolved path, stored for subprocess calls to compose.
- `compose_binary: ComposeBinary` — either `V2` (docker compose plugin), `V1(path)` (standalone docker-compose), or `NotFound`.

Socket resolution order at startup:
1. `DOCKER_HOST` env var (unix:// prefix stripped).
2. macOS only: `$HOME/.colima/default/docker.sock` (Colima default socket).
3. `/var/run/docker.sock` (Linux, WSL 2, macOS Docker Desktop).
4. If nothing found: `AppState` is created with `docker: None`. The app starts in a "disconnected" state rather than panicking.

The startup non-panic design was a deliberate fix from the initial implementation (see §8.5). `block_on(AppState::new()).expect(...)` is a common pattern but wrong for a desktop app — Docker not being open when the app launches must be a graceful state, not a crash.

The `StacksRegistry` is a `Mutex<Vec<Stack>>` backed by a JSON file in Tauri's `app_data_dir`. If the registry JSON is corrupt or the directory is not writable, the app falls back to an empty registry rather than panicking (added in the audit phase).

### 4.2 Error Type (`error.rs`)

```rust
#[derive(Debug, Error, Serialize)]
#[serde(tag = "kind", content = "message")]
pub enum AppError {
    SocketNotFound(String),
    DockerApi(String),
    PermissionDenied(String),
    ComposeError { code: i32, stderr: String },
    ComposeNotFound,
    StackNotFound(String),
    RegistryError(String),
    Io(String),
    InvalidArgument(String),
}
```

The `#[serde(tag = "kind", content = "message")]` adjacently-tagged representation serialises to:

```json
// Newtype variant:
{ "kind": "SocketNotFound", "message": "No Docker socket found..." }

// Struct variant:
{ "kind": "ComposeError", "message": { "code": 1, "stderr": "service not found" } }

// Unit variant:
{ "kind": "ComposeNotFound" }
```

The frontend `AppError` TypeScript interface was updated to match all three shapes. The initial `message: string` type was wrong for `ComposeError` and absent for `ComposeNotFound`.

### 4.3 Container Commands (`containers.rs`)

All container operations use bollard:
- `list_containers` — `bollard::Docker::list_containers` with `all: Some(true)`.
- `start_container`, `stop_container`, `restart_container`, `remove_container` — direct bollard calls with Tauri-bound `id: String` and optional flags.
- `get_container_logs` — uses `bollard::Docker::logs` with `follow: true`. Each `LogOutput` variant is converted to a `LogLine { stream, text }` and emitted as a Tauri event `container-log-{id}`. The frontend subscribes to this event channel via `LogStreamService`. Accepts `tail: Option<u32>` (defaults to 200).
- `inspect_container` — returns the full `ContainerInspectResponse` as `serde_json::Value` to avoid maintaining a parallel TypeScript model.

### 4.4 Image Commands (`images.rs`)

- `list_images` — `list_images(None)` returns an `ImageSummary` list. In bollard 0.18, `repo_tags` field is `Option<Vec<String>>` — callers must handle `None`.
- `remove_image` — `remove_image(&reference, None)`.
- `pull_image(name: String)` — uses `create_image` with `CreateImageOptions { from_image: name.clone(), ... }`. Emits `pull-progress-{name}` events per chunk so the frontend shows a live progress bar. The Tauri command parameter is `name` (matching the frontend `invoke('pull_image', { name })`).
- `inspect_image` — returns raw JSON.

### 4.5 Volume Commands (`volumes.rs`)

- `list_volumes` — uses `tokio::try_join!` to fetch volumes and prune information in parallel for a single round-trip feel.
- `create_volume(name: Option<String>)` — Docker generates a random name if `None`.
- `remove_volume(name: String)` — bollard's `remove_volume`.
- `inspect_volume` — raw JSON.

### 4.6 Compose Commands (`compose.rs`)

This module is more complex because Compose is a CLI tool, not an API:

#### Stack Discovery

`list_stacks` does four things:
1. Reads the `StacksRegistry` (stacks manually registered by the user).
2. Issues `docker ps --format json` to find running containers.
3. Groups running containers by their `com.docker.compose.project` label to discover stacks that weren't registered (e.g., started from a terminal).
4. Merges both lists. Auto-discovered stacks get synthetic IDs (`auto-{project}`) and an empty `compose_file`.

#### Stack Control

`stack_up`, `stack_down`, `stack_restart` all call the internal `run_compose` helper, which:
1. Resolves the binary (V2 plugin or standalone v1).
2. Builds the argument list: `["-f", compose_file, "up", "-d"]` etc.
3. Runs the subprocess in `tokio::task::spawn_blocking` (blocking call) to avoid blocking the async runtime.
4. Streams every line of stdout/stderr back to the frontend via `app.emit("compose-log-{id}", line)`.

`stack_down` accepts `remove_volumes: Option<bool>` (made optional so the frontend can omit it — defaults to `false`).

#### Auto-Discovered Stack Limitation

Auto-discovered stacks (those with `id.startsWith('auto-')`) have no `compose_file` in the registry and therefore cannot be controlled — `registry.get_by_id` returns `None` for them, producing `StackNotFound`. The UI disables the Up/Down/Restart buttons for these stacks, showing them as read-only observations.

### 4.7 System Commands (`system.rs`)

- `get_docker_info` — calls `docker.info()`, returns a trimmed `DockerInfo` struct with server version, container counts, OS, architecture.
- `check_connection` — lightweight ping to re-attempt socket connection. Called by the frontend's `ConnectionStore` on a timer and on user click of the "Reconnect" button.

---

## 5. Angular Frontend — Implementation

### 5.1 App Shell (`app.ts` / `app.html`)

The root component is `app.ts` (not `app.component.ts`) — this is the Angular 21 naming convention. The template is a two-column flexbox: a fixed sidebar on the left and a `<router-outlet>` on the right.

The sidebar contains navigation links styled as active/inactive based on `routerLinkActive`. The app shell also mounts `<app-toast-container>` for global toast notifications.

On init, `AppComponent` starts the connection health check loop via `ConnectionStore`.

### 5.2 Signal Stores

Three singleton signal stores, all using `providedIn: 'root'`:

**`ConnectionStore`** — holds `status: Signal<'connected' | 'disconnected' | 'checking'>`. Polls `check_connection` every 10 seconds. On disconnect, all views show a banner instead of their data.

**`UiStore`** — holds `theme: Signal<'dark' | 'light'>`. On change, toggles `data-theme` on `document.documentElement`. Theme is persisted to `localStorage`.

**`LogStore`** — per-component (not root-scoped, provided in the component's `providers` array). Holds a bounded ring buffer of `LogLine[]`, an auto-scroll flag, and a `clear()` method. Used by container detail and compose views.

### 5.3 Data Fetching Pattern

Every view uses the same pattern:

```typescript
resources = injectQuery(() => ({
  queryKey: ['containers'],
  queryFn: () => invoke<ContainerItem[]>('list_containers'),
  refetchInterval: 5_000,
}));
```

Mutations follow:

```typescript
start = injectMutation(() => ({
  mutationFn: (id: string) => invoke('start_container', { id }),
  onSuccess: () => this.queryClient.invalidateQueries({ queryKey: ['containers'] }),
  onError: (e: unknown) => this.toast.error(errorMessage(e)),
}));
```

`injectQuery` and `injectMutation` return signal-based accessors: `.data()`, `.isPending()`, `.isError()`, `.isFetching()`. These compose naturally with Angular's `@if` and `@for` template syntax.

### 5.4 IPC Layer (`lib/tauri.ts`)

A thin typed wrapper around `@tauri-apps/api/core`'s `invoke`:

```typescript
export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return tauriInvoke<T>(cmd, args);
}
```

The `errorMessage(e: unknown): string` helper extracts a human-readable string from any error shape — `AppError` (Rust), `Error` (JS), or unknown primitive:

```typescript
export function errorMessage(e: unknown): string {
  if (isAppError(e)) {
    const msg = e.message;
    if (msg === undefined) return e.kind;                         // unit variant
    if (typeof msg === 'string') return msg;                      // newtype variant
    if (typeof msg === 'object' && 'stderr' in msg) return msg.stderr || e.kind; // struct variant
    return e.kind;
  }
  if (e instanceof Error) return e.message;
  return String(e);
}
```

### 5.5 Log Streaming (`lib/log-stream.service.ts`)

Bridges Tauri's event system to RxJS:

```typescript
containerLogs$(id: string): Observable<LogLine> {
  return new Observable(observer => {
    let unlisten: (() => void) | undefined;
    listen<LogLine>(`container-log-${id}`, event => observer.next(event.payload))
      .then(fn => (unlisten = fn));
    return () => unlisten?.();
  });
}
```

The same pattern handles `composeLogs$`. The subscriber unlistens on unsubscribe, preventing memory leaks when navigating away.

### 5.6 Toast Service

`ToastService` keeps a `signal<Toast[]>()`. Each toast has an `id`, `type` (`success`|`error`|`info`), `message`, and auto-removes itself after a configurable duration via `setTimeout`. `ToastContainerComponent` renders the list using `@for`.

### 5.7 Confirm-Row Pattern

`ConfirmRowComponent` is a small inline confirmation widget. When a destructive action (remove container, force-remove image, delete volume) is triggered, the row expands to show "Confirm?" with Yes/No buttons instead of navigating to a modal dialog. This avoids modal z-index issues and keeps context visible. The confirmed callback is passed as an `@Input` function.

### 5.8 Views Summary

| View | Key features |
|------|-------------|
| **Containers** | List with status badges; Start/Stop/Restart/Remove inline; click-through to detail |
| **Container Detail** | Stats panel; live log tail with auto-scroll; inspect JSON accordion |
| **Images** | List with size and tag; Remove with force option; Pull with live progress bar; Inspect JSON |
| **Volumes** | List with driver and mount point; Create (named or anonymous); Remove; Inspect JSON |
| **Compose** | Stack list with service expansion; Up/Down/Restart; streaming log panel; auto-discovered stacks shown read-only |
| **Settings** | Docker info card; theme toggle; reconnect button |

---

## 6. IPC Layer — The Bridge

### 6.1 Tauri Command Parameter Binding

This is the most important implementation detail to understand:

When the frontend calls:

```typescript
invoke('stack_up', { stack_id: id })
```

Tauri's JS-to-Rust bridge deserialises the second argument as a JSON object and maps each key to a Rust function parameter by **exact name match**. The Rust function:

```rust
#[tauri::command]
pub async fn stack_up(id: String, ...) -> Result<(), AppError>
```

must have a parameter named `id` (matching the key in the JS object). If the JS sends `{ stack_id: id }` but Rust has `id: String`, Tauri returns a runtime deserialization error. There is no compile-time check for this mismatch — the only feedback is a runtime error in the JavaScript console.

This rule caused **5 out of 10 bugs found in the audit** (see §8).

### 6.2 Event Streaming

Tauri events flow one-way: Rust → JavaScript. The pattern used throughout:

```rust
// Rust side
let _ = app.emit(&format!("container-log-{}", id), &line);

// TypeScript side
listen<LogLine>(`container-log-${id}`, event => handler(event.payload))
```

The `listen` call returns a promise that resolves to an `unlisten` function. This must be called when the component is destroyed. `LogStreamService` handles this by returning an Observable whose teardown logic calls `unlisten`.

### 6.3 AppError Serialisation Shape

Rust's `#[serde(tag = "kind", content = "message")]` adjacently-tagged enum produces three different JSON shapes depending on the variant type:

```
Unit variant   ComposeNotFound   → { "kind": "ComposeNotFound" }
Newtype variant SocketNotFound   → { "kind": "SocketNotFound", "message": "..." }
Struct variant  ComposeError     → { "kind": "ComposeError", "message": { "code": 1, "stderr": "..." } }
```

The TypeScript interface must reflect this:

```typescript
export interface AppError {
  kind: string;
  message?: string | { code: number; stderr: string };
}
```

The initial implementation had `message: string` — wrong for struct variants (produces an object) and wrong for unit variants (the field is absent).

---

## 7. Build & CI Pipeline

### 7.1 Local Build

```
npm run tauri build
```

This runs `ng build` (esbuild, ~3 seconds) then `cargo build --release` (~2 minutes first time, incremental thereafter). The output is a platform-native installer in `src-tauri/target/release/bundle/`.

### 7.2 GitHub Actions (`.github/workflows/build.yml`)

Three-platform matrix: `ubuntu-latest`, `windows-latest`, `macos-latest`. Each job:

1. Installs the Rust toolchain from `rust-toolchain.toml`.
2. Installs Node 22 with npm cache.
3. Installs Linux system dependencies (webkit2gtk, libayatana-appindicator) on the Ubuntu runner.
4. Runs `cargo check` and `ng build` independently as fast-fail steps.
5. Runs `npm run tauri build`.
6. Uploads the bundle artifacts (DMG, MSI/NSIS, AppImage/deb).

### 7.3 Rust Toolchain

`rust-toolchain.toml` pins to `stable` (current: 1.84.x). No nightly features are used.

`.cargo/config.toml` contains linker override for macOS cross-compilation and strips debug symbols in release builds to reduce binary size.

---

## 8. Bugs Found in Audit and How They Were Fixed

The audit systematically cross-referenced every `invoke(cmd, args)` call in the frontend with the matching Rust `#[tauri::command]` function signature. Ten issues were found.

---

### 8.1 `pull_image` — Parameter Name Mismatch (CRITICAL)

**Bug**: Rust `pub async fn pull_image(reference: String, ...)` — frontend called `invoke('pull_image', { name })`.

**Effect**: Tauri could not deserialise the call. Every image pull silently failed at runtime with a deserialization error.

**Fix**: Renamed the Rust parameter from `reference` to `name` to match the frontend. Renaming on the Rust side was cleaner than renaming in the frontend because `name` is the more natural term for a Docker image reference in context.

---

### 8.2 All Compose Stack Commands — `id` vs `stack_id` (CRITICAL)

**Bug**: `stack_up`, `stack_down`, `stack_restart`, `stack_logs` all used `stack_id: String` as the Rust parameter name. The frontend called all four with `{ id }` or `{ id: stack.id }`.

**Effect**: Every compose stack action failed with a deserialization error. None of Up/Down/Restart/Logs ever worked.

**Fix**: Renamed all four Rust parameters from `stack_id` to `id`.

**Root cause**: The Rust naming was chosen for clarity (`stack_id` is more self-documenting than `id` inside the function body). But Tauri's binding is mechanical — it does not know the semantic intent, only the name. Clarity must be achieved via comments or local variable renaming inside the function, not via the parameter name itself.

---

### 8.3 `stack_down` — Missing Required Parameter (CRITICAL)

**Bug**: `stack_down(stack_id: String, remove_volumes: bool, ...)` — `remove_volumes` was `bool`, not `Option<bool>`. The frontend called `invoke('stack_down', { id })` without passing `remove_volumes`.

**Effect**: Tauri rejected the call because a required field was absent during deserialisation. Stack down never worked.

**Fix**: Changed to `remove_volumes: Option<bool>` with `unwrap_or(false)` inside the body. The frontend passes `remove_volumes: false` explicitly as well. The `Option` type future-proofs callers that may want to omit it.

---

### 8.4 Auto-Discovered Stacks — Control Actions Always Fail (CRITICAL)

**Bug**: `list_stacks` auto-discovers running compose stacks from Docker container labels and gives them synthetic IDs: `format!("auto-{}", project_name)`. `stack_up`, `stack_down`, and `stack_restart` call `registry.get_by_id(&id)` which looks up stacks in the JSON registry — auto-discovered stacks are not in the registry.

**Effect**: Clicking Up/Down/Restart on any auto-discovered stack always returned `AppError::StackNotFound`. The Rust code was correct; the architecture made it structurally impossible for auto-discovered stacks to be controlled.

**Fix**: In the Angular component, added `isAutoDiscovered(id: string): boolean` which returns `true` for `id.startsWith('auto-')`. The Up/Down/Restart buttons are `[disabled]="isAutoDiscovered(stack.id)"`. These stacks are shown as read-only observations. A tooltip could explain this in M1.

**Longer-term fix** (M1): Auto-discovered stacks could be promoted to the registry by locating the compose file via the container's `com.docker.compose.project.config_files` label.

---

### 8.5 `lib.rs` — Panic on Startup Without Docker (CRITICAL)

**Bug**:
```rust
let app_state = tauri::async_runtime::block_on(AppState::new())
    .expect("Failed to initialise app state");
```

`AppState::new()` returned `Err(AppError::SocketNotFound(...))` if no Docker socket was found. `.expect()` panicked. The Tauri app exited immediately with a crash dialog.

**Effect**: The app was completely unusable on machines where Docker is not running when the app opens — a very common scenario on macOS with Colima (which must be started manually).

**Fix**: Changed `AppState::new()` to return `Self` directly (infallible). When no socket is found, it constructs `AppState { docker: Arc::new(Mutex::new(None)), socket_path: String::new(), ... }`. Every `get_docker()` call on this state returns `Err(AppError::SocketNotFound(...))`. The frontend's `ConnectionStore` polls `check_connection` and displays a banner when disconnected.

Similarly, `StacksRegistry::load().expect(...)` was changed to `.unwrap_or_else(|_| StacksRegistry::empty())`, adding `StacksRegistry::empty()` as a constructor.

---

### 8.6 `AppError` TypeScript Type — `message: string` (MEDIUM)

**Bug**: The TypeScript interface had `message: string`, which is wrong for two cases:
- `ComposeError` struct variant: `message` is `{ code: number; stderr: string }`.
- `ComposeNotFound` unit variant: `message` is absent entirely.

**Effect**: `errorMessage()` called `e.message` and got `[object Object]` for compose errors and `undefined` for compose-not-found. Error toasts showed useless text.

**Fix**: Updated the interface to `message?: string | { code: number; stderr: string }` and updated `errorMessage()` to branch on all three cases.

---

### 8.7 Redundant Providers in `app.config.ts` (LOW)

**Bug**: `ConnectionStore`, `UiStore`, and `ToastService` all have `providedIn: 'root'`. They were also listed explicitly in `appConfig.providers`.

**Effect**: Angular created a second instance of each service at the `appConfig` scope, shadowing the root-scoped instances. Components using `inject(ConnectionStore)` inside the `AppComponent` provider tree got the local instance; components outside got the root instance. These could diverge in state.

**Fix**: Removed the explicit listings from `appConfig.providers`. Services with `providedIn: 'root'` must not be re-listed in providers arrays — doing so creates shadowing instances.

---

### 8.8 Orphaned `app.component.ts` / `app.component.html` (LOW)

**Bug**: Two files existed from the initial Angular CLI scaffold: `app.component.ts` (a re-export shim exporting `App as AppComponent`) and `app.component.html` (empty). Nothing imported them.

**Effect**: Confusion for any developer reading the codebase who expected Angular's conventional entry component. Dead code that could cause future accidental imports.

**Fix**: Deleted both files. The Angular 21 naming convention (`app.ts`/`app.html`) is the source of truth.

---

### 8.9 Unused `computed` Import in `compose.component.ts` (LOW)

**Bug**: `computed` was imported from `@angular/core` but never used in the component.

**Effect**: No runtime impact; dead import, noise in the module graph.

**Fix**: Removed from the import list.

---

### 8.10 `get_container_logs` Ignored `tail` Parameter (INFO)

**Bug**: The frontend called `invoke('get_container_logs', { id, tail: 200 })` but the Rust function signature was `get_container_logs(id: String, app: AppHandle, state: State<...>)` — no `tail` parameter. Tauri silently ignored the extra `tail` key. The function hardcoded `tail: "500"`.

**Effect**: The frontend UI intent (200 lines) was ignored. Extra unknown parameters to Tauri commands are silently discarded — no error, no warning.

**Fix**: Added `tail: Option<u32>` to the Rust function. `unwrap_or(200)` makes 200 the effective default, matching the frontend expectation.

---

## 9. Lessons Learned

### 9.1 Tauri IPC Parameter Names Are a Silent Contract

The single biggest class of bugs in this project (5 of 10) came from mismatches between Rust parameter names and JavaScript object keys in `invoke()` calls. Tauri deserialises the args object into the Rust function parameters by name, and **extra unknown keys are silently discarded, missing required keys cause a runtime deserialization error**.

**What to do**:
- Treat Tauri `#[tauri::command]` parameter names as a public API contract, identical to a REST endpoint's field names.
- Write an integration test or at minimum a manual test checklist that exercises every `invoke()` call once before marking a feature complete.
- In M1, consider generating TypeScript bindings from the Rust command signatures using `tauri-specta`. This would catch name mismatches at compile time.

### 9.2 Never `.expect()` in App Startup Code

`block_on(async_operation()).expect("message")` is appropriate in tests and CLI tools. In a GUI application, panicking on startup produces a crash dialog that users find alarming and is often triggered by routine conditions (Docker not open). The correct pattern is always:

```
try → succeed → normal state
try → fail  → degraded/disconnected state
degrade gracefully, provide UI to retry
```

`expect` is fine for truly unrecoverable programmer errors (e.g., building the Tauri application object itself). It is never appropriate for I/O operations that can fail for environmental reasons.

### 9.3 Serde's Adjacently-Tagged Enum Needs a Matching TypeScript Type

Rust's `#[serde(tag = "kind", content = "message")]` enum notation is elegant in Rust. The output JSON is not uniform — unit, newtype, and struct variants each produce different shapes. If you define a TypeScript interface against this, you must account for all three shapes: `message` absent, `message: string`, and `message: { ... }`.

Write the TypeScript interface by reading the Rust enum and the serde documentation together, not by inspection of a single example response.

### 9.4 `providedIn: 'root'` and Explicit Provider Listing Are Mutually Exclusive

Listing a service that already has `providedIn: 'root'` in a component or application `providers` array creates a second, scoped instance. Angular will not warn about this. The two instances are independent. This silently breaks shared state (e.g., a toast service that was notified but whose instance is not the one the toast container is watching).

Rule: pick one. For app-wide singletons, use `providedIn: 'root'` and never list them in any providers array.

### 9.5 bollard 0.18 Fields Are More Optional Than 0.17

When upgrading bollard, expect `Option<T>` to appear where `T` existed before. The compiler will catch these — they are not silent. But the volume of changes can be surprising. Important ones:

- `ContainerSummary.image` → `Option<String>`
- `ImageSummary.repo_tags` → `Option<Vec<String>>`
- `Volume.created_at` → `Option<BollardDate>` (not `Option<String>`)

Pattern: always `.unwrap_or_default()` or provide sensible display fallbacks (`"<none>"`, `"unknown"`).

### 9.6 `injectQuery` Must Be Called in Constructor Context

TanStack Angular Query's `injectQuery()` calls Angular's `inject()` internally. Angular only permits `inject()` during the construction phase of a component or service. Calling it in `ngOnInit`, an event handler, or any other lifecycle hook throws a runtime error. This is not surfaced as a TypeScript type error — only a console error at runtime.

Pattern: declare all `injectQuery` and `injectMutation` calls as class field initialisers:

```typescript
class MyComponent {
  // ✓ — field initialiser, runs during construction
  data = injectQuery(() => ({ ... }));

  ngOnInit() {
    // ✗ — too late, inject() context has closed
    this.data = injectQuery(() => ({ ... }));
  }
}
```

### 9.7 Compose is a CLI, Not an API

It is tempting to think of docker-compose as an API you can talk to. It is not. It is a Python/Go CLI tool that itself calls the Docker Engine API. The correct approach for compose control is:
1. Shell out to `docker compose` / `docker-compose`.
2. Capture stdout/stderr.
3. Stream them back to the UI.

There is no stable programmatic interface. Any attempt to re-implement compose logic (parsing YAML, calling the engine API directly for each service) will break on edge cases the compose devs have already solved.

### 9.8 Auto-Discovered Resources Need Explicit Read-Only Treatment

`list_stacks` auto-discovers running compose projects from container labels. This is great for observability but creates a subtle contract violation: the user sees a stack in the UI, assumes they can control it, clicks "Up", and gets an error. The fix is to classify resources by their control surface at the data model level:

- Does the resource have a registered compose file? → **controllable**.
- Discovered from runtime state only? → **read-only / observable**.

Surface this as a distinct visual treatment in the UI. Disable control buttons. Add a tooltip explaining how to register the stack.

### 9.9 Angular 21 Naming Convention: `app.ts` not `app.component.ts`

Angular 21 defaults to the shorter naming convention (`app.ts`, `app.html`, `app.routes.ts`) without the `.component` infix for the root component. The Angular CLI scaffold creates these files, but anything that used the old convention (`app.component.ts`) as a re-export shim is dead weight. Delete it early.

### 9.10 Tauri Extra Parameters Are Silently Discarded

When the frontend passes an extra key in the invoke args object that has no matching Rust parameter, Tauri ignores it with no warning. This means stale or wrong code that was once correct (e.g., `tail: 200` before the Rust side added the param) fails invisibly. The result is a feature that appears to work but does not.

Write assertion tests or use `tauri-specta` to generate typed bindings. Without generated bindings, review every invoke call whenever the Rust command signature changes.

---

## 10. What M1 Should Address

In order of priority:

1. **tauri-specta** — generate TypeScript bindings from Rust command signatures. Eliminates the entire class of parameter name mismatches at compile time.

2. **Auto-stack promotion** — when an auto-discovered stack has the `com.docker.compose.project.config_files` label set, offer a one-click "Register" button to add it to the registry.

3. **Container stats polling** — `bollard::Docker::stats` provides CPU and memory metrics as a stream. `ContainerDetailComponent` should display a mini chart (recharts or d3 over the WebView canvas).

4. **Image tag filtering** — the image list grows long. Add a search/filter input. Dangling images (`<none>:<none>`) should be visually distinct.

5. **Context switching** — support multiple Docker contexts (local socket, SSH remote, Docker Desktop). `DOCKER_CONTEXT` or `DOCKER_HOST` switching via the Settings view.

6. **Pull conflict handling** — if a pull starts for an image that is in use, surface a clear error rather than a raw bollard error string.

7. **Volumes in-use protection** — remove volume should check if any container is using it before calling the API, and if so, offer to stop the container first.

8. **Log export** — add a "Copy all" or "Save to file" action to the log panel using `tauri-plugin-fs` and `tauri-plugin-dialog`.

9. **E2E tests** — `tauri-driver` with WebDriver for end-to-end testing. At minimum: connect, list containers, start one, verify status badge changes.

10. **Windows Docker Desktop socket** — `npipe:////./pipe/docker_engine` support via bollard's named pipe connector. Currently untested on Windows Docker Desktop.

---

*End of document.*
