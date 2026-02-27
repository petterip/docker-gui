# MVP Feature Specifications

## Scope Statement

The MVP covers exactly the operations a developer needs day-to-day when working with a local Docker environment. Nothing more. Advanced features (registry management, network topology, resource limits, Dev Environments) are post-MVP.

---

## Destructive Action Pattern (canonical — applied everywhere)

All destructive operations (stop, remove, down, …) use **inline row confirmation** — not a modal dialog — to keep context visible, consistent with the UX philosophy.

When the user triggers a destructive action the table row expands inline:

```
│ web-1  nginx:alpine  ● Running  80→8080  │
│  ⚠ Stop container web-1?  [Confirm]  [Cancel]   │
```

- A single optional **checkbox** for additive options ("Also remove volumes", "Force remove") appears on the same inline row.
- Pressing **Escape** or clicking elsewhere cancels.
- Only one row may be in confirmation state at a time.
- This component is implemented once (`ConfirmRow`) and reused across all views.

---

## F1 — Containers

### F1.1 List Containers

- Display all containers (running + stopped) in a table.
- Columns: **Name**, **Image**, **Status** (colour-coded badge), **Ports**, **Created**, **Actions**.
- Toggle: "Show stopped containers" (default: on).
- Auto-refresh every **3 seconds**.
- Click row → open Container Detail panel.

### F1.2 Start Container

- Action button per row (and in detail panel).
- Available when container status is `exited` / `created` / `paused`.
- Calls `POST /containers/{id}/start`.
- Optimistic UI: badge switches to `starting…` immediately.

### F1.3 Stop Container

- Available when status is `running` / `restarting`.
- Calls `POST /containers/{id}/stop` (10 s grace period).
- Uses **Destructive Action Pattern** (inline row confirmation): "Stop container **{name}**?"

### F1.4 Restart Container

- Available when running or stopped.
- Calls `POST /containers/{id}/restart`.

### F1.5 Remove Container

- Available for any non-running container (auto-stop option for running ones).
- Uses **Destructive Action Pattern** with checkbox **"Also remove anonymous volumes"**.
- Calls `DELETE /containers/{id}?v={bool}`.

### F1.6 Container Logs

- Side panel or full-page log viewer.
- Streams via `GET /containers/{id}/logs?follow=true&stdout=1&stderr=1&tail=500`.
- Features: **auto-scroll toggle**, **copy to clipboard**, **filter/search** (client-side regex).
- **Auto-scroll behaviour**: auto-scroll is ON by default. It disables automatically when the user scrolls up manually; a "Jump to latest" button appears at the bottom to re-enable it. This matches terminal/IDE conventions and prevents the view jumping away while the user is reading.
- ANSI colour codes rendered.
- Line cap: last 5 000 lines in memory.

### F1.7 Container Detail (Inspect)

- Tabs: **Overview** (ports, env vars, mounts), **Logs**, **Inspect** (raw JSON collapsible tree).

---

## F2 — Images

### F2.1 List Images

- Columns: **Repository:Tag**, **Image ID** (short), **Size**, **Created**, **Actions**.
- Show `<none>:<none>` dangling images with a warning badge.
- Auto-refresh every **10 seconds**.

### F2.2 Remove Image

- Uses **Destructive Action Pattern** with checkbox **"Force remove"** (adds `force=true`).
- Calls `DELETE /images/{id}`.
- If image is in use, show the blocking container name and disable Confirm (force checkbox overrides the block).

### F2.3 Pull Image

- "Pull image" button opens a dialog: text field for `image:tag`.
- Client-side validation against OCI image reference format before enabling Pull (inline error, e.g. "Use `name:tag` — colons separate name and tag").
- Streams pull progress per layer (download / extract / complete) in a progress panel.
- Uses Tauri event stream: `image-pull-progress` events.

### F2.4 Inspect Image

- Expandable JSON tree (same component as container inspect).
- Shows layers, env, entrypoint, exposed ports.

---

## F3 — Volumes

### F3.1 List Volumes

- Columns: **Name**, **Driver**, **Mount Point**, **Created**, **In Use** (badge), **Actions**.
- Derive "In Use" by checking if any running container mounts this volume.
- Auto-refresh every **10 seconds**.

### F3.2 Create Volume

- "New volume" button: simple dialog with a single Name field (optional — Docker auto-names if blank).
- Driver is always `local` (MVP). No driver dropdown.
- Calls `POST /volumes/create`.

### F3.3 Remove Volume

- Uses **Destructive Action Pattern**.
- Disabled (with tooltip naming blocking containers) if volume is in use.
- Calls `DELETE /volumes/{name}`.

### F3.4 Inspect Volume

- Show driver options, mount point, labels, creation time.

---

## F4 — Compose Stacks

### F4.1 List Stacks

- Auto-discovers running stacks from container labels (`com.docker.compose.project`).
- Merges with user-registered stacks (stacks registry JSON).
- Columns: **Project name**, **Services** (count), **Status** (all running / partial / stopped), **Compose file path**, **Actions**.

### F4.2 Register a Stack

- "Open compose file" button → native file picker → add to stacks registry.
- Validates that the file is a valid YAML before registering.

### F4.3 Stack Up

- Runs: `<compose-binary> -f <file> -p <project> up -d` (binary resolved at startup — see architecture).
- Streams stdout/stderr to log panel.
- Disables action buttons while running.

### F4.4 Stack Down

- Uses **Destructive Action Pattern** with checkbox **"Remove volumes"**.
- Runs: `<compose-binary> -f <file> -p <project> down [--volumes]`

### F4.5 Stack Restart

- Runs: `<compose-binary> -f <file> -p <project> restart`

### F4.6 Service List

- Expanding a stack row shows individual services.
- Per-service: **Name**, **Image**, **Status**, **Replicas**, **Ports**.
- Per-service actions: stop, start, restart.

### F4.7 Compose Logs

- Log viewer for entire stack (all services merged) or per-service.
- Runs: `<compose-binary> -f <file> -p <project> logs -f --tail 200 [service]`

---

## F5 — Connection / Status Bar

- Global status bar indicator: **Connected** (green) / **Connecting…** (yellow) / **Not connected** (red).
- On "Not connected": shows actionable message ("Start Colima: `colima start`" / "Start Docker Engine").
- Shows Docker version and API version.
- Settings: manual override of socket path.

---

## Out of Scope for MVP

- Build image from Dockerfile
- Registry login / push
- Network management
- Resource usage graphs (CPU/RAM per container)
- Dev Environments / volume bind-mounts wizard
- Multi-context / remote Docker hosts
- Image vulnerability scanning
- Kubernetes integration
