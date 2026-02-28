# Windows Engine Provisioning – Bootstrapper/Helper State Machine

Last updated: 2026-02-28

This document defines the concrete state machine that the elevated installer bootstrapper and provisioning helper must implement for the Windows WSL Engine provisioning flow. The goal is to make resumable, idempotent behavior observable in both components and in the persisted `engine_providers.json` state so the desktop app can recover after reboots or crashes.

## Actors and Responsibilities

| Actor | Responsibility |
| --- | --- |
| **Installer Bootstrapper** | Elevated process that runs during first install, launches helper, and writes signed helper bits + integrity manifest. Owns Windows Feature enablement and reboot prompts. |
| **Provisioning Helper** | Elevated child process invoked from installer or app settings via IPC. Runs discrete privileged actions (enablement, distro import, package install, relay registration) and reports structured status codes. |
| **docker-gui App** | Non-elevated UI process that orchestrates stages, persists checkpoints via `EngineRegistry`, and requests privileged actions through helper contracts. |

## Persisted Context

All actors MUST agree on the serialized context stored at `%APPDATA%/docker-gui/engine_providers.json`:

- `run_id` – UUID for the active provisioning run.
- `target_provider_id` – always `wsl_engine` for Phase 1.
- `resume_checkpoint` – most recent checkpoint ID (see table below).
- `resume_privileged_allowed` – persisted consent flag the helper must read before executing privileged actions after reboot.
- `provisioning.stages[]` – stage metadata exposed to the UI.

The helper reads `resume_checkpoint` on startup and emits the first pending checkpoint ID when signalling readiness so the app/bootstrapper can resume at the correct stage.

## Checkpoint IDs and Stage Mapping

| Checkpoint ID | Stage Label | Helper Responsibility | Exit Conditions |
| --- | --- | --- | --- |
| `before_windows_features` | Checking prerequisites | Detect WSL capability, enable `Microsoft-Windows-Subsystem-Linux` + `VirtualMachinePlatform` via DISM/PowerShell, ensure `wsl.exe --status` returns OK. | Success → next checkpoint; Missing capability → `prereq_missing`; DISM reboot response → `reboot_required`. |
| `before_distro` | Preparing WSL distro | Discover managed distro `docker-gui-engine`; if absent, run `wsl.exe --import` (managed tarball) or `wsl.exe --install -d Ubuntu`. Persist distro metadata. | Success → `before_engine_install`; distro failure → `distro_install_failed`; reboot flag → `reboot_required`. |
| `before_engine_install` | Installing engine packages | Run non-interactive script inside distro to install Docker Engine + Compose, create docker group, enable service, smoke test `docker version/info`. | Success → `before_relay_registration`; apt errors → `engine_install_failed`; script exit non-zero → `engine_install_failed`. |
| `before_relay_registration` | Registering relay endpoint | Install/start managed Windows named-pipe relay, register pipe `npipe:////./pipe/docker_gui_engine`, ensure WSL user has permissions, persist relay PID + pipe path. | Success → `health_check`; relay start failure → `relay_failed`. |
| `health_check` | Running health checks | Invoke `docker version`, `docker info`, `docker ps` via relay to confirm host connectivity. | Success → Finish run; health failures → `connectivity_failed`. |

Each checkpoint corresponds 1:1 with the stage IDs defined in `provisioning_stage_specs()` (`src-tauri/src/commands/engine.rs`).

## State Machine

```
IDLE
  └─(user clicks Set up automatically)→ CONSENT_PENDING?
CONSENT_PENDING (no persisted consent)
  ├─(user grants)→ PRECHECK
  └─(user declines)→ IDLE (status failed + failure_class=permission_denied)
PRECHECK (checkpoint=before_windows_features)
  ├─(prereqs satisfied)→ DISTRO (checkpoint=before_distro)
  ├─(reboot required)→ REBOOT_PENDING
  └─(fatal failure)→ FAILED(before_windows_features)
REBOOT_PENDING
  ├─(helper registers run + consent)→ REBOOT_WAITING
REBOOT_WAITING
  ├─(system reboot)→ RESUME_SIGNAL (bootstrapper auto-runs helper at logon)
RESUME_SIGNAL
  ├─(helper sees resume_checkpoint)→ resume stage (DISTRO/ENGINE/RELAY/HEALTH)
DISTRO (checkpoint=before_distro)
  ├─(success)→ ENGINE_INSTALL (checkpoint=before_engine_install)
  ├─(reboot)→ REBOOT_PENDING
  └─(failure)→ FAILED(before_distro)
ENGINE_INSTALL (checkpoint=before_engine_install)
  ├─(success)→ RELAY (checkpoint=before_relay_registration)
  ├─(transient failure retriable)→ ENGINE_INSTALL (retries up to 3)
  └─(fatal)→ FAILED(before_engine_install)
RELAY (checkpoint=before_relay_registration)
  ├─(success)→ HEALTH (checkpoint=health_check)
  └─(failure)→ FAILED(before_relay_registration)
HEALTH (checkpoint=health_check)
  ├─(success)→ SUCCEEDED
  └─(failure)→ FAILED(health_check)
SUCCEEDED
  └─(persist active provider, clear checkpoint)
FAILED(stage_id)
  └─(persist checkpoint=stage_id, emit error, user may Retry/Fix automatically)
```

## Transition Rules

1. **Consent gating** – Provisioning may only progress past `CONSENT_PENDING` if `resume_privileged_allowed` is true. If the helper is invoked after reboot without this flag it must exit with `permission_denied`, leaving the UI in connected-limited mode.
2. **Reboot handling** – When a stage signals `reboot_required`, the helper writes `resume_checkpoint` to that stage ID, requests reboot via bootstrapper UX, and exits with status `pending_reboot`. After reboot, the bootstrapper re-launches helper with `--resume` so it can continue from the stored checkpoint.
3. **Stage retries** – Each stage execution is wrapped by `execute_stage_with_backoff` (three attempts, 500ms exponential backoff). Helper responses must set `retriable=true` for transient errors; otherwise the dispatcher marks the stage Failed immediately.
4. **Failure classification** – Helper responses must populate `failure_class` with canonical values (`prereq_missing`, `reboot_required`, `distro_install_failed`, `engine_install_failed`, `engine_start_failed`, `relay_failed`, `connectivity_failed`, `permission_denied`). The app will map unknown values to `connectivity_failed` for UX copy.
5. **Idempotency** – Each stage must be safe to rerun:
   - Windows feature enablement checks for existing enablement before running DISM.
   - Distro install reuses `docker-gui-engine` if already registered.
   - Engine install script is guarded by dpkg status checks.
   - Relay registration verifies pipe/listener before re-creating.
6. **Completion** – After `HEALTH` succeeds, helper notifies the app, which calls `set_active_provider(Provider::WslEngine)` and clears the checkpoint.

## Helper Event Protocol

| Event | Description | Payload |
| --- | --- | --- |
| `provisioning_stage_started` | Helper acknowledges stage execution. | `{ "stage_id": "before_distro" }` |
| `provisioning_stage_completed` | Stage succeeded. | `{ "stage_id": "before_distro" }` |
| `provisioning_stage_failed` | Stage failed with canonical class + message. | `{ "stage_id": "before_distro", "failure_class": "distro_install_failed", "message": "wsl.exe exited 0x80070005" }` |
| `provisioning_resume_needed` | Helper requires reboot/resume from checkpoint. | `{ "stage_id": "before_distro", "reason": "reboot_required" }` |

Bootstrapper wiring: when receiving `provisioning_resume_needed` with `reason=reboot_required`, the installer shows the reboot prompt, persists the helper command line (`--resume --checkpoint before_distro`), and registers a RunOnce entry to relaunch after reboot.

## Recovery Paths

- **Retry** – When user clicks `Retry` or `Fix automatically`, the app launches helper starting at `resume_checkpoint`.
- **Repair** – Settings > Repair uses the same helper but sets `source=settings_engine_retry` so telemetry distinguishes flows.
- **Limited mode** – If provisioning fails repeatedly, the UI surfaces `Continue with limited mode` but keeps `resume_checkpoint` so helper can resume later without losing progress.

## Open Items

1. Signed binary + integrity manifest handshake (helper advertises SHA256 to app) – tracked separately in security checklist.
2. Command-line contract for bootstrapper (`--resume --checkpoint <id> --run-id <uuid>`). Implementation stub lives in `src-tauri/src/bin/docker-gui-provisioning-helper.rs`.

