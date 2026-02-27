# Architecture

## Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                        Tauri App Process                         │
│                                                                  │
│  ┌────────────────────────────────┐   ┌───────────────────────┐ │
│  │       WebView (Frontend)        │   │    Rust Core (IPC)    │ │
│  │                                │◄──►                       │ │
│  │  Angular 21 + TypeScript       │   │  Tauri Commands       │ │
│  │  TanStack Angular Query        │   │  bollard (Docker API) │ │
│  │  Angular Signals (state)       │   │  compose subprocess   │ │
│  │  Spartan UI + Tailwind         │   │  socket resolver      │ │
│  └────────────────────────────────┘   └──────────┬────────────┘ │
│                                                   │              │
└───────────────────────────────────────────────────┼─────────────┘
                                                    │ Unix socket / named pipe
                                         ┌──────────▼──────────┐
                                         │   Docker Engine      │
                                         │  (REST API v1.47+)   │
                                         └─────────────────────┘
```

---

## Component Breakdown

### 1. Rust Core (`src-tauri/src/`)

#### 1a. Socket Resolver (`config.rs`)

Determines the Docker socket path at startup, in priority order:

```
1. DOCKER_HOST env var (user override)
2. Platform defaults:
   - Linux:  /var/run/docker.sock
   - macOS:  $HOME/.colima/default/docker.sock
             then /var/run/docker.sock (fallback)
   - WSL 2:  /var/run/docker.sock (inside WSL)
3. Error dialog if no socket found — guide user to start Colima / Docker Engine
```

The resolved path is stored in a `Mutex<AppState>` managed by Tauri and passed into every `bollard::Docker` client construction.

#### 1b. Docker API Layer (inline in `commands/`)

**KISS**: No separate `docker/` abstraction layer for MVP. Bollard is called directly inside each Tauri command handler. Adding an intermediate layer would be premature — commands and their bollard calls are 1:1 at this scope. Extract a shared layer only when a real second consumer appears.

Key patterns:
- All bollard calls are `async` on the tokio runtime embedded by Tauri.
- Log streaming uses Tauri **events** (`app.emit`) rather than a single command response, so the frontend receives lines incrementally.
- Pull progress similarly streams events back keyed by image ref.

#### 1c. Compose Module (`commands/compose.rs`)

Uses `std::process::Command` to shell out to the Compose CLI. At startup the binary is resolved once and cached in `AppState`:

```rust
fn resolve_compose_binary() -> Result<String, AppError> {
    // Prefer docker compose v2 plugin
    if Command::new("docker").args(["compose", "version"]).output().is_ok() {
        return Ok("docker".into()); // invoked as: docker compose …
    }
    // Fall back to standalone v1 binary
    if Command::new("docker-compose").arg("version").output().is_ok() {
        return Ok("docker-compose".into());
    }
    Err(AppError::ComposeNotFound)
}
```

If neither is found, the Compose tab is disabled in the UI and a settings warning is shown.

Once resolved, CLI calls are:

```
<binary> [-f <path>] -p <project> up -d
<binary> [-f <path>] -p <project> down [--volumes]
<binary> [-f <path>] -p <project> ps --format json
<binary> [-f <path>] -p <project> logs --tail 200 [service]
```

The compose file path is stored in the user-facing "Stacks" database (see below).

#### 1d. Stacks Registry (`stacks_registry.rs`)

A simple JSON file (`$APP_DATA/stacks.json`) stores user-registered compose projects:

```json
[
  {
    "id": "uuid",
    "name": "my-app",
    "compose_file": "/home/user/projects/my-app/docker-compose.yml",
    "added_at": "2026-02-27T10:00:00Z"
  }
]
```

The registry struct is wrapped in a `tokio::sync::Mutex` and managed by Tauri state to prevent concurrent read-modify-write races across async commands:

```rust
pub struct StacksRegistry(pub Mutex<Vec<Stack>>);
```

On every write the full JSON is flushed atomically (write to a `.tmp` file, then `fs::rename`). On load, entries whose `compose_file` path no longer exists on disk are flagged as `missing: true` and surfaced with a warning badge in the UI — they are not silently deleted.

Auto-discovery of running compose projects is also performed via the `com.docker.compose.project` container label.

#### 1e. Tauri Commands (`commands/`)

Commands are the IPC bridge — called from the frontend via `invoke()`:

```
commands/
├── containers.rs   — list_containers, start_container, stop_container,
│                     remove_container, get_logs (event stream)
├── images.rs       — list_images, remove_image, pull_image (event stream)
├── volumes.rs      — list_volumes, create_volume, remove_volume
├── compose.rs      — list_stacks, stack_up, stack_down,
│                     stack_restart, stack_logs, register_stack
└── system.rs       — get_docker_info, check_connection
```

#### 1f. AppError Type (`error.rs`)

All Tauri commands return `Result<T, AppError>`. A shared, serialisable enum maps every failure category to a stable discriminant the frontend can pattern-match on:

```rust
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(tag = "kind", content = "message")]
pub enum AppError {
    #[error("Docker socket not found: {0}")]
    SocketNotFound(String),

    #[error("Docker API error: {0}")]
    DockerApi(String),          // maps from bollard::errors::Error

    #[error("Permission denied on socket: {0}")]
    PermissionDenied(String),

    #[error("Compose CLI error (exit {code}): {stderr}")]
    ComposeError { code: i32, stderr: String },

    #[error("Compose CLI not found in PATH")]
    ComposeNotFound,

    #[error("Stack registry IO error: {0}")]
    RegistryError(String),

    #[error("IO error: {0}")]
    Io(String),
}

impl From<bollard::errors::Error> for AppError { /* … */ }
impl From<std::io::Error>         for AppError { /* … */ }
```

The `kind` discriminant (e.g. `"PermissionDenied"`) is read by the Angular frontend to display contextual help banners rather than raw error strings.

---

### 2. Angular Frontend (`src/`)

#### 2a. Views (standalone components)

```
views/
├── containers/
│   ├── containers.component.ts    — table + action toolbar
│   └── container-detail.component.ts — tabs: overview, logs, inspect
├── images/
│   └── images.component.ts        — table + pull dialog
├── volumes/
│   └── volumes.component.ts       — table + create dialog
├── compose/
│   └── compose.component.ts       — stacks list + service breakdown
└── settings/
    └── settings.component.ts      — socket path, theme, update check
```

#### 2b. Data Fetching (TanStack Angular Query in components)

**KISS**: `injectQuery()` and `injectMutation()` are called directly inside component constructors, where Angular's injection context is naturally available. No separate service wrapper is needed — the query result is component state.

```typescript
// containers.component.ts
@Component({ /* … */ })
export class ContainersComponent {
  private queryClient = injectQueryClient();

  containers = injectQuery(() => ({
    queryKey: ['containers'],
    queryFn:  () => invoke<Container[]>('list_containers'),
    refetchInterval: 3_000,
  }));

  start = injectMutation(() => ({
    mutationFn: (id: string) => invoke('start_container', { id }),
    onSuccess:  () => this.queryClient.invalidateQueries({ queryKey: ['containers'] }),
  }));
}
```

A shared `invoke` helper module (`src/app/lib/tauri.ts`) provides typed wrappers with the `AppError` shape, used directly by each component. Mutations surface errors as toast notifications.

#### 2c. Real-time Streaming — Container Logs

Container log streams use Tauri's event system bridged to RxJS Observables. Pull-image progress uses the same pattern — see §2e.

```typescript
// log-stream.service.ts
containerLogs$(id: string): Observable<LogLine> {
  return new Observable(subscriber => {
    const unlistenPromise = listen<LogLine>(`container-log-${id}`, e =>
      subscriber.next(e.payload)
    );
    return () => unlistenPromise.then(fn => fn());
  });
}
```

Log lines are held in a `signal<LogLine[]>` (capped at 5 000 lines, see `LogStore` in §2d) and rendered via `@tanstack/angular-virtual` (virtualised list).

#### 2d. Angular Signals State

No external state library. Component-level and app-level state uses Angular's built-in Signals:

```typescript
// app state (provided in root)
@Injectable({ providedIn: 'root' })
export class ConnectionStore {
  private queryClient = inject(QueryClient);

  status     = signal<'connected' | 'connecting' | 'disconnected'>('connecting');
  version    = signal<string | null>(null);
  socketPath = signal<string>('');

  isConnected = computed(() => this.status() === 'connected');

  // Call this whenever Docker reconnects after a disconnect.
  // Stale query data from before the outage must be invalidated immediately
  // rather than waiting for the next poll tick.
  onReconnect(): void {
    this.status.set('connected');
    this.queryClient.invalidateQueries();  // invalidate all keys
  }
}

// UI / theme state — the only truly global UI signal
// YAGNI notes:
//   activeView  → removed: Angular Router is the single source of truth for the active view.
//   activeModal → removed: each component owns its own open/close boolean signal;
//                          a global modal manager is unneeded at MVP scale.
@Injectable({ providedIn: 'root' })
export class UiStore {
  theme = signal<'dark' | 'light' | 'system'>('dark');
}

// Per-container log buffer
// KISS + SRP: LogStore is component-scoped (provided in ContainerDetailComponent),
// not global. It is created when the detail panel opens and destroyed when it closes,
// so cleanup is free. No global service needed for data that only one component reads.
@Injectable()   // no providedIn — declared in ContainerDetailComponent.providers
export class LogStore {
  private readonly MAX_LINES = 5_000;
  readonly lines = signal<LogLine[]>([]);

  append(line: LogLine): void {
    this.lines.update(buf => {
      const next = [...buf, line];
      return next.length > this.MAX_LINES ? next.slice(-this.MAX_LINES) : next;
    });
  }
}
```

#### 2e. Pull-Image Progress Streaming

Pull-image progress from bollard is a stream of JSON events. The pattern mirrors §2c but events are emitted to a dedicated channel and consumed only by the open Pull dialog — not persisted to any store.

**Rust side** (`commands/images.rs`):

```rust
#[tauri::command]
pub async fn pull_image(app: AppHandle, reference: String) -> Result<(), AppError> {
    let docker = get_docker(&app).await?;
    let mut stream = docker.create_image(
        Some(CreateImageOptions { from_image: reference.clone(), ..Default::default() }),
        None, None,
    );
    while let Some(event) = stream.next().await {
        let msg = event.map_err(AppError::from)?;
        app.emit("image-pull-progress", &msg)?;
    }
    Ok(())
}
```

**Angular side** (inside `PullImageDialogComponent` constructor):

```typescript
// Scoped to dialog lifetime — unlisten fires on ngOnDestroy.
private pullProgress$ = new Observable<CreateImageInfo>(subscriber => {
  const p = listen<CreateImageInfo>('image-pull-progress', e =>
    subscriber.next(e.payload)
  );
  return () => p.then(fn => fn());
});

// subscribe in constructor after mutation starts
this.pullProgress$.subscribe(event =>
  this.progressLines.update(lines => [...lines, event])
);
```

Key decisions:

- Event name: `"image-pull-progress"` (no container ID suffix — only one pull runs at a time in MVP).
- `CreateImageInfo` is bollard's type; Tauri's serde deserialises it automatically.
- On bollard error the `?` operator converts to `AppError::DockerApi`; Angular's `injectMutation` `onError` handler shows an inline message in the dialog.
- On success: `injectQueryClient().invalidateQueries(['images'])` in `onSuccess`.

---

## Data Flow: Start a Container

```
User clicks "Start"
  │
  ▼
injectMutation.mutate(id) → invoke('start_container', { id })
  │
  ▼  (IPC boundary — serialised JSON)
  │
Tauri command: start_container
  │
  ▼
bollard: Docker::start_container(id, None).await
  │
  ▼
Docker Engine API  POST /containers/{id}/start
  │
  ◄── 204 No Content
  │
Command returns Ok(())
  │  (IPC boundary)
  ▼
onSuccess → queryClient.invalidateQueries(['containers'])
  │
  ▼
injectQuery re-fetches → Angular Signals update → template re-renders
```

---

## Data Flow: Compose Stack Up

```
User clicks "Up" on stack "my-app"
  │
  ▼
invoke('stack_up', { stack_id })
  │
  ▼
Rust: look up compose_file from stacks registry
  │
  ▼
spawn: docker compose -f <path> -p <project> up -d
  │
  ▼ stdout/stderr lines
emit('compose-log', line) → frontend log panel
  │
  ▼ process exit
return Ok(exit_code) or Err(message)
```

---

## Error Handling

| Scenario | Behaviour |
|----------|-----------|
| Docker socket not found | Splash/banner: "Docker not running — start Colima / Docker Engine" |
| Container operation fails | Toast notification with Docker error message |
| Compose CLI not in PATH | Settings warning; graceful disable of Compose tab |
| Stream disconnect (Docker restart) | Query auto-retries with exponential backoff via TanStack Query |
| Permission denied on socket | Clear message: "Add user to docker group" or "check Colima status" |

---

## Security Model (Tauri v2 Permissions)

Tauri v2 requires explicit capability declarations. The app requests:

```json
// src-tauri/capabilities/default.json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "windows": ["main"],
  "permissions": [
    "core:default",
    {
      "identifier": "shell:allow-execute",
      "allow": [
        {
          "name": "docker",
          "cmd": "docker",
          "args": { "validator": ".+" }
        },
        {
          "name": "docker-compose",
          "cmd": "docker-compose",
          "args": { "validator": ".+" }
        }
      ]
    },
    "fs:allow-read-text-files",     // for reading compose files
    "fs:allow-write-text-files"     // for stacks registry JSON
    // process:allow-exit is NOT requested — the frontend never needs to kill the process directly.
    // shell:allow-execute is intentionally scoped to "docker" and "docker-compose" only.
    // "colima" is NOT listed — launching 'colima start' from the app is post-MVP (YAGNI:
    //   requires a resource-config UI and separate capability surface).
  ]
}
```

No network permissions are needed — all Docker communication is over the local Unix socket.

---

## Build Pipeline

```
GitHub Actions matrix:
  # MVP targets — Linux binary is also used for WSL 2 (see note below)
  os: [ubuntu-22.04, macos-14]

  # Windows-native (WebView2 .exe) is post-MVP; the WSL 2 workflow uses
  # the linux .deb/.AppImage launched via WSLg — no windows-latest runner needed.

Toolchain pinning (committed to repo):
  rust-toolchain.toml  → channel = "stable", version = "1.82"
  .nvmrc               → 22
  package.json         → "packageManager": "pnpm@9.x"

steps:
  1. actions/setup-node (version from .nvmrc) + pnpm install
     with: cache: pnpm
  2. rustup (reads rust-toolchain.toml automatically)
     Swatinem/rust-cache@v2  workspaces: "src-tauri -> target"
  3. pnpm tauri build
  4. upload-artifact (installer per OS)

macOS code-signing (required env secrets):
  APPLE_CERTIFICATE          # base64-encoded .p12
  APPLE_CERTIFICATE_PASSWORD
  APPLE_SIGNING_IDENTITY     # "Developer ID Application: …"
  APPLE_ID                   # Apple ID email
  APPLE_PASSWORD             # app-specific password
  APPLE_TEAM_ID
  → tauri.conf.json: macOS.signingIdentity = $APPLE_SIGNING_IDENTITY
  → Notarization runs automatically via tauri-action on tagged releases.

Auto-updater: **POST-MVP (M5 Polish)**
  Users download new releases manually until then. When implemented, use tauri-plugin-updater
  with a GitHub Releases JSON manifest endpoint.

Release versioning (KISS — no scripts or automation in MVP):
  - Single source of truth: version in src-tauri/Cargo.toml
  - Bump package.json version manually before tagging
  - Tagged release: git tag vX.Y.Z && git push --tags
  - GPG artifact signing, auto-updater manifest, and CHANGELOG automation — post-MVP (M5)

Artefacts produced:
  Linux   → docker-gui_x.y.z_amd64.AppImage + docker-gui_x.y.z_amd64.deb
  macOS   → docker-gui_x.y.z_aarch64.dmg (Apple Silicon), _x64.dmg (Intel) — notarized
```
