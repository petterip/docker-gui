# UI/UX Design Plan

## Design Philosophy

- **Familiar first** — mirror Docker Desktop's information architecture so users migrate without relearning.
- **Dense but not cramped** — tables show maximum information; panels collapse cleanly.
- **Dark-mode by default** — developers live in dark terminals; light mode is available.
- **No modals for destructive actions** — use inline confirmation rows or slide-over panels to keep context visible.
- **Status is always visible** — connection health, container counts, and active operations appear in persistent UI chrome.

---

## Application Shell

```
┌──────────────────────────────────────────────────────────────────────┐
│  ●  docker-gui        [_][□][×]  (or macOS traffic lights)           │
├────────────┬─────────────────────────────────────────────────────────┤
│            │  [Search / filter…]                    [⚙ Settings]     │
│  SIDEBAR   │─────────────────────────────────────────────────────────│
│            │                                                          │
│ ▶ Containers│              MAIN CONTENT AREA                         │
│   Images   │                                                          │
│   Volumes  │                                                          │
│   Compose  │                                                          │
│            │                                                          │
│            │                                                          │
├────────────┴─────────────────────────────────────────────────────────┤
│  ● Connected  Docker 27.0.1 (API 1.47)         [0 running] [5 images]│
└──────────────────────────────────────────────────────────────────────┘
```

### Sidebar

- Width: 200 px (collapsible to icon-only at 48 px).
- Items: icon + label. Active item has a coloured left border accent.
- Section badges show live counts: `Containers (3 running)`, `Images (12)`, `Volumes (5)`.
- Compose item shows stack count.

### Top Bar — Per-view Filter

The `[Search / filter…]` bar is a **per-view filter** applied to the currently visible table. It filters by name/ID client-side (substring, case-insensitive). It is **not** a global cross-view search — that is post-MVP. Each view also exposes a status filter dropdown when relevant (e.g. `[Status ▾]` on the Containers view).

### Status Bar (bottom)

- Left: connection dot + Docker version.
- Right: summary counts (running containers, total images).
- Clicking the connection dot opens the connection settings popover.

---

## Containers View

```
┌──────────────────────────────────────────────────────────────────────┐
│  Containers                            [Filter▾]  [Status▾]  │
├──────────────┬──────────────────┬──────────┬────────┬───────────────┤
│ Name         │ Image            │ Status   │ Ports  │ Actions       │
├──────────────┼──────────────────┼──────────┼────────┼───────────────┤
│ web-1        │ nginx:alpine      │ ● Running│ 80→8080 │ [■] [↺] [⋯]  │
│ api-1        │ node:20-slim      │ ● Running│        │ [■] [↺] [⋯]  │
│ db-1         │ postgres:16       │ ○ Exited │ 5432   │ [▶] [🗑] [⋯] │
└──────────────┴──────────────────┴──────────┴────────┴───────────────┘
```

**Note**: "Start all" / "Stop all" header buttons are removed from MVP. They provide minimal daily value and risk accidental mass-stop with no confirmation. Post-MVP.

- Status badges: green `● Running`, grey `○ Exited`, yellow `◐ Paused`, blue `↻ Restarting`.
- `[⋯]` opens a context menu: **Logs**, **Inspect**, **Copy ID**. (Rename is post-MVP — not included to avoid shipping a disabled/greyed item that creates questions.)
- Row click opens the **Container Detail** slide-over panel from the right.

### Container Detail (Slide-over)

```
┌────────────────────────────────────────────┐
│ web-1                            [■ Stop] × │
│ nginx:alpine  ● Running  2 hours ago        │
├─────────────────┬──────────┬───────────────┤
│ Overview        │ Logs     │ Inspect       │
├─────────────────┴──────────┴───────────────┤
│ PORTS                                       │
│   0.0.0.0:8080 → 80/tcp                    │
│                                             │
│ ENVIRONMENT                                 │
│   NGINX_HOST=localhost                      │
│   NGINX_PORT=80                             │
│                                             │
│ MOUNTS                                      │
│   /usr/share/nginx/html → ./html (bind)     │
└─────────────────────────────────────────────┘
```

### Log Viewer (in detail panel or full-screen toggle)

```
┌─────────────────────────────────────────────────────────────┐
│ Logs — web-1          [🔍 Filter…]  [auto-scroll ✓]  [copy] │
├─────────────────────────────────────────────────────────────┤
│ 2026-02-27 10:01:22 172.18.0.3 - "GET / HTTP/1.1" 200       │
│ 2026-02-27 10:01:23 172.18.0.3 - "GET /api HTTP/1.1" 404    │
│ ...                                                          │
└─────────────────────────────────────────────────────────────┘
```
**Auto-scroll behaviour:** auto-scroll is on by default. It **automatically disables** the moment the user manually scrolls upward — the toggle reflects this. Re-enable by clicking the toggle or scrolling to the bottom.
---

## Images View

```
┌──────────────────┬───────────────┬──────────┬────────┬──────────────┐
│ Repository:Tag   │ Image ID      │ Size     │ Created│ Actions      │
├──────────────────┼───────────────┼──────────┼────────┼──────────────┤
│ nginx:alpine     │ a1b2c3d4e5    │ 42 MB    │ 3d ago │ [🗑] [⋯]     │
│ postgres:16      │ f6g7h8i9j0    │ 390 MB   │ 1w ago │ [🗑] [⋯]     │
│ <none>:<none>    │ k1l2m3n4o5 ⚠ │ 210 MB   │ 2w ago │ [🗑]         │
└──────────────────┴───────────────┴──────────┴────────┴──────────────┘
[+ Pull image]
```

### Pull Image Dialog

```
┌──────────────────────────────────────────┐
│  Pull Image                            × │
│                                          │
│  Image reference:                        │
│  [ubuntu:24.04                        ]  │
│  ← inline error if format is invalid    │
│                                          │
│  [Cancel]                  [Pull ▶]      │
└──────────────────────────────────────────┘
```

- **Pull** is disabled until the input matches a valid OCI image reference (`name` or `name:tag` or `registry/name:tag`).
- Invalid input shows an inline error immediately: e.g. `ubuntu-24:04` → *"Use a colon to separate name and tag: ubuntu:24.04"*.

After clicking Pull, the dialog transforms into a progress panel:

```
│ Pulling ubuntu:24.04                       │
│                                            │
│ Layer a1b2c3d4  Download  ████████░░  78%  │
│ Layer e5f6g7h8  Download  ██████████ 100% ✓│
│ Layer i9j0k1l2  Extracting ████░░░░░  40%  │
│                                            │
│                           [Cancel]         │
```

---

## Volumes View

```
┌──────────────────┬─────────┬─────────────────────────────┬────────┬──────────────┐
│ Name             │ Driver  │ Mount Point                  │ In Use │ Actions      │
├──────────────────┼─────────┼─────────────────────────────┼────────┼──────────────┤
│ postgres_data    │ local   │ /var/lib/docker/volumes/…   │ ● yes  │ [⋯] [🗑]     │
│ redis_cache      │ local   │ /var/lib/docker/volumes/…   │ ○ no   │ [⋯] [🗑]     │
└──────────────────┴─────────┴─────────────────────────────┴────────┴──────────────┘
[+ Create volume]
```

- 🗑 is disabled (with tooltip listing blocking containers) when In Use = yes.

---

## Compose View

```
┌──────────────────┬──────────┬───────────────┬──────────────────────┐
│ Project          │ Services │ Status        │ Actions              │
├──────────────────┼──────────┼───────────────┼──────────────────────┤
│ ▶ my-app         │ 3 / 3 ↑  │ ● All running │ [↺] [■ Down] [logs]  │
│   web            │          │ ● Running     │ [■] [▶] [logs]       │
│   api            │          │ ● Running     │ [■] [▶] [logs]       │
│   db             │          │ ● Running     │ [■] [▶] [logs]       │
├──────────────────┼──────────┼───────────────┼──────────────────────┤
│ ▶ staging        │ 1 / 2 ↑  │ ◐ Partial     │ [▶ Up] [■ Down]      │
└──────────────────┴──────────┴───────────────┴──────────────────────┘
[+ Open compose file]
```

- Rows are collapsible/expandable.
- Partial status (some services stopped) shown in amber.

---

## Colour System

Using CSS custom properties, switchable via `[data-theme="dark"|"light"]` on `<html>`.

### Dark Theme (default)

| Token | Value | Usage |
|-------|-------|-------|
| `--bg-base` | `#1a1b1e` | App background |
| `--bg-surface` | `#25262b` | Cards, sidebar |
| `--bg-elevated` | `#2c2e33` | Table rows, inputs |
| `--border` | `#373a40` | Dividers, table lines |
| `--text-primary` | `#c1c2c5` | Body text |
| `--text-secondary` | `#868e96` | Muted labels |
| `--accent` | `#228be6` | Links, active states, primary buttons |
| `--success` | `#2f9e44` | Running status |
| `--warning` | `#f08c00` | Partial/paused |
| `--danger` | `#c92a2a` | Error, destructive actions |

Inspired by Mantine's dark colour scheme — professional, low eye strain.

### Status Badge Colours

| Status | Background | Text |
|--------|-----------|------|
| Running | `#2f9e44` | white |
| Exited | `#373a40` | `#868e96` |
| Paused | `#e67700` | white |
| Restarting | `#1971c2` | white |
| Dead | `#c92a2a` | white |

---

## Typography

- Font: **Inter** (variable), loaded from bundled assets (no CDN).
- Monospace (logs, IDs): **JetBrains Mono** or system mono fallback.
- Base size: 14 px.
- Log viewer: 12 px / 1.6 line height.

---

## Timestamps

All tables show **relative time** ("3d ago"). Every relative time cell has a **tooltip** showing the absolute ISO 8601 timestamp on hover. The log viewer shows absolute timestamps directly. This pattern is applied consistently — one shared `RelativeTime` component renders both.

---

## Search / Filter Architecture

Two tiers, each a distinct component:

1. **Per-view filter bar** (MVP): each view has a local text input that performs client-side substring/case-insensitive match on the visible table. Containers also exposes a status filter chip (`Running` / `Exited` toggle).
2. **Global cross-view search** (`Ctrl+K` command palette): **post-MVP** (YAGNI — adds a separate query layer with little MVP value).

---

## Loading States

First load and re-fetch show **skeleton rows** (animated shimmer) rather than a spinner or empty table, so layout is stable:

```
│ ████████     █████████████     ███████   ████   ₈₈₈₈₈₈₈₈  │
│ ██████████  █████████████     ███████   ████   ₈₈₈₈₈₈₈₈  │
```

Number of skeleton rows matches the last known row count (or 5 on first load).

---

## Empty States

Each view has a contextual empty state when there is no data:

| View | Empty state message |
|------|---------------------|
| Containers | "No containers found. Pull an image and run it to get started." + [Pull image] button |
| Images | "No images. Pull one to get started." + [Pull image] button |
| Volumes | "No volumes. Volumes are created automatically when containers need them." |
| Compose | "No stacks registered. Open a docker-compose.yml to add one." + [Open compose file] button |

If Docker is **not connected**, all views show a single full-area banner instead of an empty table:

```
┌──────────────────────────────────────────────────┐
│  ⚠  Cannot connect to Docker                          │
│                                                        │
│  Socket: /home/user/.colima/default/docker.sock        │
│  Error:  Permission denied                             │
│                                                        │
│  Run: sudo usermod -aG docker $USER  then log out/in   │
│                                                        │
│  [Open Settings]                                       │
└──────────────────────────────────────────────────┘
```

The message text is derived from the `AppError.kind` discriminant returned by `check_connection`.

---

## Keyboard Navigation

| Key | Action |
|-----|--------|
| `↑` / `↓` or `j` / `k` | Navigate table rows |
| `Enter` | Open detail panel for focused row |
| `Escape` | Close slide-over / cancel inline confirmation |
| `Space` | Toggle primary action on focused row (start/stop) |
| `Ctrl+F` | Focus per-view filter input |
| `Ctrl+L` | Focus log search input (when log viewer is open) |
| `Tab` | Move between interactive elements |

Focus ring visible in both themes (no `outline: none` without a visible replacement).

---

## Motion & Transitions

- Table row appear/disappear: fade + height transition (150 ms ease-out).
- Slide-over panel: translate X from right (200 ms ease-out).
- Status badges: colour cross-fade (300 ms).
- No decorative animations — performance over polish.
- Respect `prefers-reduced-motion`.

---

## Accessibility

- All interactive elements reachable via keyboard Tab order.
- Status indicators use both colour AND icon/text (never colour alone).
- ARIA roles on tables (`role="grid"`), dialogs, and status regions.
- Focus ring visible in both themes (no `outline: none` without replacement).
- Keyboard shortcuts are defined in the **Keyboard Navigation** section above.

---

## Responsive / Window Sizing

Minimum window: **900 × 600 px**  
Default window: **1200 × 750 px**  
Sidebar collapses below: **1000 px** wide

The app is NOT intended for mobile — it is a desktop utility.

---

## Settings Page

```
┌─────────────────────────────────────────────────────────┐
│ Settings                                                  │
│                                                          │
│  DOCKER CONNECTION                                               │
│  Detected socket:  /home/user/.colima/default/docker.sock        │
│                    (read-only — set by auto-detection on startup) │
│  Override (DOCKER_HOST):  [                                 ]    │
│  [Test connection]  ← shows latency and Docker version          │
│                                                          │
│  APPEARANCE                                               │
│  Theme:  [● Dark  ○ Light  ○ System]                     │
│                                                          │
│  BEHAVIOUR                                                │
│  ☑ Start minimised to tray                              │
│  ☑ Launch on login                                       │
│  Polling interval (containers): [3] seconds              │
│                                                          │
│  COMPOSE                                                  │
│  docker compose path: [docker compose]  (auto-detected)  │
│                                                          │
│  ABOUT                                                    │
│  docker-gui v0.1.0                                        │
└─────────────────────────────────────────────────────────┘
```

- **Detected socket** is read-only; it reflects what `resolve_socket_path()` found at startup.
- **Override (DOCKER_HOST)** pre-fills from the `DOCKER_HOST` env var if set. Changing it and clicking Test applies a temporary override without an app restart; persisting it writes the value to app config so it takes precedence over auto-detection on next launch.
- The Compose path shown under **COMPOSE** is also read-only — it reflects what `resolve_compose_binary()` detected at startup.
