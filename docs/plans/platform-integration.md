# Platform Integration

This document covers every platform-specific concern: socket discovery, installation, auto-launch, and known caveats.

---

## macOS — Colima

### Docker Socket Location

Colima exposes the Docker socket at:

```
$HOME/.colima/default/docker.sock
```

The app uses the default profile socket only. Users with non-default profiles (e.g. `colima start --profile arm`) must set `DOCKER_HOST` explicitly — a multi-profile picker is post-MVP (YAGNI).

Fallback: `/var/run/docker.sock` (works if `DOCKER_HOST` points there or Docker for Mac is installed).

### Detection Logic

The full socket resolution code lives in `src-tauri/src/config.rs` — see [architecture.md](architecture.md) (section 1a, Socket Resolver). The priority order is:

1. `DOCKER_HOST` env var (user override)
2. `~/.colima/default/docker.sock` (Colima default profile)
3. `/var/run/docker.sock` (system fallback)
4. Error banner if nothing found

### App Distribution (macOS)

- Tauri produces a signed `.dmg` (requires Apple Developer ID).
- For unsigned local builds: `xattr -dr com.apple.quarantine docker-gui.app`
- Auto-update via Tauri's built-in updater (GitHub Releases as update server).

### Tray Icon

The app ships a menu bar / system tray icon using Tauri's `tray-icon` feature, showing:
- Connection status dot (connected / disconnected)
- Open main window
- Quit

"Start Colima" / "Stop Colima" tray actions are **post-MVP** — they require adding `colima` to the `shell:allow-execute` capability scope and designing a resource-config UX (CPU / memory / disk). Deferred (YAGNI).

### "Start Colima" Integration — POST-MVP

Running `colima start` from the GUI requires:
1. Adding `colima` to the `shell:allow-execute` capability scope.
2. A resource-configuration form (CPU / memory / disk) in Settings.

Both are deferred. For MVP, users run `colima start` from the terminal before launching the app. The No-Docker banner shows the detected socket path and links to Settings for manual override.

---

## Linux — Ubuntu (and other systemd distros)

### Docker Socket Location

Standard path: `/var/run/docker.sock`

The systemd socket unit `docker.socket` activates on first connection — no need for the service to be pre-started.

### Permissions

By default the socket is owned by `root:docker`. The user must be in the `docker` group:

```
sudo usermod -aG docker $USER
newgrp docker   # or log out/in
```

The app detects a permission error on socket connect and shows a banner:

> "Permission denied on /var/run/docker.sock.  
> Run: `sudo usermod -aG docker $USER` then log out and back in."

### App Distribution (Linux)

Tauri produces:
- `.AppImage` — portable, no install needed (`chmod +x docker-gui.AppImage && ./docker-gui.AppImage`)
- `.deb` — for Ubuntu/Debian (`sudo dpkg -i docker-gui.deb`)

Desktop entry (`.desktop` file) is created automatically by the `.deb` package and by AppImage launcher.

### Autostart (Linux)

Optional: write an XDG autostart entry to `~/.config/autostart/docker-gui.desktop`.
Tauri provides `autostart` plugin support (`tauri-plugin-autostart`).

---

## Windows — WSL 2

### Architecture

The application **runs inside WSL 2** (a Linux process). Windows users launch it through:

1. **WSLg** (Windows 11 / Windows 10 with WSL2 update) — native Wayland/X11 forwarding, zero config, the app window appears as a normal Windows window.
2. A **Windows shortcut** that runs `wsl.exe docker-gui` (the distro-installed binary).

The `.exe` installer produced by Tauri for a future "Windows-native" version (WebView2-based) is a separate target — not in MVP scope. MVP ships the Linux binary, distributed via a WSL-friendly `.deb` or script.

### Docker Socket in WSL 2

Docker Engine runs inside WSL 2 at the standard path:

```
/var/run/docker.sock
```

The app uses the exact same binary as Linux — no WSL-specific code paths needed in the Rust core.

### Prerequisites

| Requirement | Install |
|-------------|---------|
| WSL 2 with Ubuntu 22.04+ | `wsl --install` |
| Docker Engine in WSL 2 | `apt install docker.io` or [Docker's official script](https://get.docker.com) |
| WSLg | Windows 11 built-in; Windows 10: WSL update from Microsoft Store |
| User in docker group | `sudo usermod -aG docker $USER` |

### Installation in WSL 2

```bash
# Option A — .deb
wget https://github.com/user/docker-gui/releases/latest/download/docker-gui.deb
sudo dpkg -i docker-gui.deb
docker-gui &

# Option B — AppImage
wget https://github.com/user/docker-gui/releases/latest/download/docker-gui.AppImage
chmod +x docker-gui.AppImage
./docker-gui.AppImage
```

### Windows Start Menu Integration

**POST-MVP.** Creating a Windows `.lnk` shortcut via PowerShell is polish, not a blocker. Users launch the app with `wsl.exe docker-gui &` or via the WSL terminal in the meantime.

### Known WSL Caveats

| Issue | Mitigation |
|-------|------------|
| WSLg may not be available on older Windows 10 | Document X410 / VcXsrv as alternatives |
| High-DPI scaling in WSLg can be imperfect | Honour GTK_SCALE / GDK_SCALE env vars; allow DPI override in Settings |
| Docker Engine not auto-started on WSL boot | Show informational banner: "Run `sudo service docker start` in your WSL terminal." A clickable Start button is post-MVP (requires `sudo` capability scope — YAGNI at MVP scale). |
| Slow socket under load | Inherent WSL 2 Unix socket I/O; acceptable for a management GUI |

---

## Cross-Platform Environment Variable Override

The user can override socket detection by setting `DOCKER_HOST` (Unix sockets only in MVP):

```bash
export DOCKER_HOST=unix:///var/run/docker.sock
export DOCKER_HOST=unix:///home/user/.colima/default/docker.sock
```

This is surfaced in Settings → "Docker socket path" with a text field that pre-fills from `DOCKER_HOST` if set.

TCP remote host (`DOCKER_HOST=tcp://…`) is **post-MVP**.

---

## Dependency Matrix

| Feature | macOS (Colima) | Linux | WSL 2 |
|---------|----------------|-------|-------|
| Containers/Images/Volumes | `colima start` | `dockerd` running | `dockerd` running in WSL |
| Compose stacks | `<compose-binary>` — v2 plugin or v1 binary, auto-detected at startup (see architecture.md §1c) | same | same |
| System tray | ✅ macOS tray | ✅ via AppIndicator/libayatana | ✅ via WSLg tray |
| Auto-start on login | ✅ LaunchAgent | ✅ XDG autostart | ✅ Windows Task Scheduler (via helper, post-MVP) |
| Auto-update | post-MVP | post-MVP | post-MVP |
