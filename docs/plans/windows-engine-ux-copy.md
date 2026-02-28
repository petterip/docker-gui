# Windows Engine Provisioning UX Copy

Last updated: 2026-02-28

This deck captures the canonical customer-facing strings for the Windows container engine setup, repair, and limited-mode flows. Copy must avoid jargon (`npipe`, `context`, `WSL`) except inside `View details` diagnostics.

## 1. Installer – Container Engine Setup

| Element | Copy | Notes |
| --- | --- | --- |
| Title | `Container engine setup` | Title case only. |
| Body | `We can install and configure the WSL Engine for you. This takes about 5–10 minutes and may reboot your PC.` | Sets expectation about reboot/time. |
| Primary Action | `Set up automatically` | Default focused button. |
| Secondary Action | `Continue with limited mode` | Leaves provisioning pending but opens app. |
| Link | `Learn what’s installed` | Opens `View details` drawer with distro + Docker package list. |

## 2. Live Provisioning Progress

| Stage | Status Text | Failure hint |
| --- | --- | --- |
| Checking prerequisites | `Enabling Windows features…` | `Windows needs Virtual Machine Platform and WSL enabled. You can retry after reboot.` |
| Preparing WSL distro | `Importing docker-gui-engine WSL distro…` | `We couldn’t finish downloading or importing Ubuntu. Retry or check your network.` |
| Installing engine packages | `Installing Docker Engine and Compose inside WSL…` | `Package install failed. Retry to reinstall automatically.` |
| Registering relay endpoint | `Creating secure Docker pipe…` | `Relay service did not start. Retry to reinstall the relay.` |
| Running health checks | `Verifying Docker is ready…` | `Docker did not respond. Select Fix automatically to retry.` |

Progress footer (always visible):

- `You can keep working while we finish. We’ll notify you when Docker is ready.`

## 3. Failure Modal

| Element | Copy |
| --- | --- |
| Title | `We couldn’t finish setting up Docker` |
| Body (dynamic) | `Docker Engine install failed during “Installing engine packages”.` |
| Primary Action | `Fix automatically` |
| Secondary Action | `Retry` |
| Inline Link | `Continue with limited mode` |
| Details Toggle | `View details` |

Details body template:

```
Stage: Installing engine packages
Reason: apt exit code 100 (package mirror timeout)
Recommended action: Fix automatically
```

## 4. Guided Disconnected Mode Banner

| Element | Copy |
| --- | --- |
| Title | `Docker isn’t connected yet` |
| Body | `Select Fix automatically and we’ll resume setup from where it stopped.` |
| Primary Action | `Fix automatically` |
| Secondary Action | `View details` |

## 5. Settings > Engine Cards

### Active Provider Card (WSL Engine)

- Title: `WSL Engine`
- Status Pill (Ready): `Ready`
- Status Pill (Needs repair): `Needs repair`
- Status Pill (Not installed): `Not installed`
- Body text when Ready: `Docker runs inside the managed docker-gui-engine WSL distro.`
- Body text when Needs repair: `We’ll reinstall packages and restart Docker inside WSL.`
- Primary button when not installed: `Install WSL Engine`
- Primary button when unhealthy: `Fix automatically`

### Host Engine Card

- Title: `Host Engine`
- Body: `Connect to an existing Docker or Podman host.`
- Primary button: `Switch to Host Engine`
- Empty state footnote: `You need to install the host yourself. We’ll detect it automatically.`

### Install Another Engine Flow

| Step | Copy |
| --- | --- |
| Wizard intro title | `Choose an engine to install` |
| Wizard intro body | `We recommend WSL Engine for this PC. You can switch providers at any time in Settings.` |
| Option 1 label | `WSL Engine (recommended)` |
| Option 1 body | `Installs Docker Engine inside WSL with automatic updates.` |
| Option 2 label | `Host Engine` |
| Option 2 body | `Connect to an existing Docker or Podman installation.` |

## 6. Limited Mode Banner

- Title: `You’re in limited mode`
- Body: `Docker commands are disabled until a container engine is connected.`
- Button: `Fix automatically`

## 7. Telemetry Consent Copy

When helper requires elevation consent:

- Title: `Allow Docker GUI to make changes?`
- Body: `We’ll enable Windows features, install a WSL distro, and download Docker Engine from the official repository.`
- Checkbox: `Remember my choice for automatic repair`
- Buttons: `Allow`, `Cancel`

## Localization Notes

1. Avoid abbreviations except `WSL`, which is industry standard. Expand once in tooltips (`Windows Subsystem for Linux`).
2. Keep imperative verbs (`Set up`, `Fix`) consistent so automation references remain accurate.
3. Strings above are canonical; any new copy must be added here before implementation to keep product + documentation aligned.

