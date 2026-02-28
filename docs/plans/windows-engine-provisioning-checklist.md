# Windows Engine Provisioning Checklist

Last updated: 2026-02-28

## Status Legend

- `[x]` Done
- `[~]` In progress / partial
- `[ ]` Not started

## Current Snapshot

- Phase readiness: **M1 partial**, **M2 partial**, **M3 not started**
- MVP currently has:
  - provider model + persistence
  - provider switch with rollback behavior
  - guided disconnected mode with fail-safe actions
  - checkpointed provisioning state machine scaffold + UI progress
- MVP currently does **not** yet have:
  - privileged bootstrapper/helper implementation
  - real WSL install/feature enablement/reboot-resume execution
  - lightweight WSL-host (`podman_wsl_api`) provider path

## Checklist by Plan Area

## 1) Product Requirement: Fail-Safe by Default

- `[~]` Default path one click (`Set up automatically`)
  - Disconnected guidance banner now invokes setup orchestration directly (`Set up automatically`) with provisioning start/retry + live stage status polling; installer bootstrapper integration remains pending.
- `[x]` No hard dead-end screen (Retry/Fix automatically/Continue limited mode)
- `[x]` No terminal commands required in primary UI flow
- `[~]` Provisioning idempotent/resumable/reboot-safe
  - Checkpoints, run status, retry backoff, and stage-resume from checkpoint exist; privileged auto-resume is now consent-gated via persisted resume permission state.
- `[x]` Guided disconnected mode with one-click repair

## 2) Decision Lock (v1)

- `[x]` WSL Engine as phase-1 install target in code paths
- `[~]` Host Engine semantics defined
  - Detect/connect existing endpoint is present; lightweight WSL-host provider (`podman_wsl_api`) is planned but not yet implemented.
- `[x]` No binary redistribution in app installer logic
- `[ ]` Privileged actions delegated to signed helper (not app process)

## 3) Architecture

### 3.1 Bootstrapper + Privilege Boundary

- `[ ]` Installer bootstrapper (elevated, signed)
- `[~]` Provisioning helper process for repair/install
  - Added `docker-gui-provisioning-helper` binary implementing the current action contract; signing/elevation packaging remains pending. Helper is now built via `pnpm run build:helper`, embedded under `resources/bin/win32`, and auto-discovered by the app/relay without manual installation.
- `[~]` App requests actions through structured commands
  - Provisioning stages now execute through a typed privileged-action dispatcher with structured action IDs/events; helper IPC boundary split is still pending.
- `[x]` Bootstrapper/helper state machine spec documented (`docs/plans/windows-engine-provisioning-state-machine.md`)

### 3.2 Engine Provider Model

- `[x]` `WslEngine`, `HostEngine`, `CustomHost` provider schema
- `[x]` Provider persistence (`active`, `previous`, checkpoint, provisioning state)
- `[x]` Provider switch rollback to previous healthy provider
- `[x]` Provider registry schema documented (`docs/plans/windows-engine-provider-schema.md`)

### 3.3 Endpoint Contract

- `[~]` WSL relay endpoint contract represented (`npipe:////./pipe/docker_gui_engine`)
  - Relay registration + lifecycle state metadata are now persisted with health validation and repair-time revalidation; fallback viability checks are in place while relay process start/ensure-running management is still pending.
- `[x]` Host endpoint resolution path
- `[x]` No insecure `0.0.0.0:2375` fallback

### 3.4 First-Run Orchestrator

- `[x]` Startup reconnect + health check loop
- `[x]` User-facing fix action when disconnected
- `[~]` Strict provider-first startup path
  - App now attempts provider endpoint reconnect during startup; installer-first orchestration is still pending.
- `[x]` UX copy deck finalized for installer/progress/failure/settings flows (`docs/plans/windows-engine-ux-copy.md`)

## 4) WSL Provisioning Design (Phase 1)

### A. Prerequisites

- `[~]` Capability checks scaffolded
  - Windows prerequisite checks now include auto-enable attempts and post-enable revalidation.
- `[~]` Enable missing features (real execution)
  - Stage runner now attempts `wsl --install --no-distribution` and DISM feature enablement.
- `[~]` Reboot-required handling + resume execution
  - Reboot-required failures are classified and persisted; app startup calls resume when checkpoint exists, but privileged WSL resume is blocked unless prior explicit consent was recorded.

### B. Distro Strategy

- `[~]` Managed distro reuse/install logic
  - Reuse path is implemented; when no suitable distro exists, setup now attempts `wsl --install -d Ubuntu` with explicit reboot/permission failure mapping.
- `[~]` Auto-pick supported Ubuntu distro logic
  - Startup/provider selection now prefers managed distro when present, otherwise picks an installed Ubuntu distro.
- `[~]` Optional advanced manual selection
  - `View details` now exposes supported WSL distro list and lets users set a persisted distro preference.

### C. Engine Install in Distro

- `[~]` Non-interactive WSL provisioning script execution
  - `before_engine_install` now executes a root non-interactive WSL script via `wsl -d <distro> -u root`.
- `[~]` Engine + Compose install
  - Script now installs `docker.io` and compose plugin/binary with apt-based fallback handling.
- `[~]` Permissions, service enable/start
  - Script now configures docker group membership for UID 1000 user and performs best-effort service enable/start.
- `[~]` Smoke checks from provisioned distro context
  - Script now runs `docker version`, `docker info`, and compose version checks inside the distro.

### D. Recovery Model

- `[x]` Stage checkpoints are persisted
- `[~]` Failure class mapping
  - Expanded mapping exists for provisioning stages; full matrix coverage pending.
- `[x]` Retry with backoff

## 5) Settings: Install/Switch/Repair/Remove

- `[x]` Install WSL Engine action
- `[~]` Install Host Engine action
  - Host Engine install entry points are now policy-blocked; flow is detect/use existing compatible host only (switch/repair/health checks remain available).
- `[x]` Switch active engine
- `[x]` Repair active engine
- `[x]` Remove managed engine (explicit destructive action)
  - Settings action now supports managed WSL distro unregister with explicit user confirmation + consent gating.
- `[x]` Provisioning progress card with stage-level statuses and retry

## 6) Host Engine Plan (Phase 2)

- `[ ]` Implement lightweight WSL-host provider (`podman_wsl_api`)
  - Install/configure Podman API service in WSL (no desktop UI installer path).
- `[ ]` Configure/verify Podman Docker API compatibility mode
- `[~]` Compatibility gate for RW operations + compose behavior
  - Host provisioning now runs compatibility probes (ping/version/info + create/remove volume + compose availability).
- `[ ]` Reliability SLO validation

## 7) Security and Compliance

- `[~]` Explicit elevation consent wiring
  - Privileged install/repair/retry/remove flows now require explicit consent and persisted consent state gates privileged auto-resume.
- `[~]` Artifact signature/checksum verification pipeline
  - Helper dispatch now supports SHA-256 integrity verification (`DOCKER_GUI_HELPER_SHA256` or helper sidecar `.sha256`), with optional enforcement via `DOCKER_GUI_HELPER_ENFORCE_CHECKSUM`; signed artifact pipeline integration is still pending.
- `[ ]` Least-privilege split (non-admin app + elevated helper) enforced
- `[ ]` SBOM updates for installed components
- `[ ]` Legal/provider review checklist integration
- `[ ]` Credential Manager storage path for provider credentials

## 8) Telemetry and Diagnostics

- `[~]` Structured failure classes in provisioning model
  - Provisioning stage/state failure classes are now normalized to the canonical recovery set, while telemetry preserves raw failure class detail for diagnostics.
- `[x]` Stage started/completed/failed telemetry emission
- `[~]` Provider installed/switched/repaired event telemetry
  - Switch/rollback/repair + reconnect + provisioning events are emitted; provider install success/failure and retry-request telemetry now includes normalized source tags.
- `[~]` Local log paths for bootstrapper/helper/reconnect flow
  - Settings `View diagnostics` now exposes concrete bootstrapper/helper/reconnect/event/relay file paths with presence status from app data logs/state.

## 9) Acceptance Criteria Tracking

- `[ ]` Fresh machine success >=95% without terminal
- `[ ]` Median clicks <=5 from setup screen
- `[ ]` Median time <=12 minutes (excluding reboot wait)
- `[ ]` Recovery success >=90% for top failure classes
- `[ ]` Provider switch success >=95% with guaranteed fallback

## Extended Plan (Next Implementation Waves)

## Wave A: Privileged Execution Foundation

- `[~]` Define helper command protocol (JSON contract + versioning)
  - Contract now exposes typed action specs (`id`, description, elevation requirement) and helper identity metadata; execution remains in-process until helper split.
- `[~]` Implement signed elevated helper executable
  - Helper executable scaffold now exists in `src-tauri/src/bin`; code signing and elevated installer integration are still pending.
- `[~]` Route provisioning commands through helper instead of app process
  - Stage runner routes privileged operations via centralized `run_privileged_action` dispatch and attempts `docker-gui-provisioning-helper` first with helper path resolution + integrity precheck; on Windows, helper-only mode is now the default (`DOCKER_GUI_HELPER_STRICT` override, `DOCKER_GUI_ALLOW_IN_PROCESS_FALLBACK=1` opt-out).
- `[~]` Add consent/elevation UX for install/repair actions
  - Settings and guided repair flows require explicit user confirmation before WSL privileged install/repair/retry commands are invoked; privileged auto-resume is blocked without recorded consent. Helper-driven UAC/elevation handoff is still pending.

## Wave B: Real WSL Provisioning

- `[~]` Implement prerequisite enablement (`wsl`, required Windows features)
- `[~]` Implement reboot-required checkpoint + automatic resume
- `[~]` Implement distro selection strategy and managed distro lifecycle
  - Managed distro reuse/auto-install is implemented and explicit managed-distro removal is now wired via `Remove managed engine`; advanced selection and import lifecycle are still pending.
- `[~]` Run non-interactive install script in distro
  - WSL stage now executes a non-interactive root install script (`wsl -d <distro> -u root -- bash -lc ...`) with package/service/smoke-check steps; distro diversity hardening remains pending.
- `[~]` Implement relay registration and lifecycle
  - Registration + health/lifecycle state tracking are implemented; WSL relay checks now ensure Docker runtime is started inside distro and start/ensure managed relay via built-in helper `run-relay` mode by default (with env/sidecar fallbacks available).
- `[~]` Convert stage runner from simulated to real command execution
  - Stage runner no longer uses per-stage artificial delay; remaining simulated behavior is limited to helper-less in-process execution.

## Wave C: Host Engine Support

- `[ ]` Implement `podman_wsl_api` install/repair adapter (WSL-host only)
- `[~]` Add host provider health compatibility checks
  - Host stage includes API, RW volume, and compose compatibility checks.
- `[~]` Add host repair flow and diagnostics
  - Repair path now runs host compatibility diagnostics before reconnect.

## Wave D: Hardening + Release Readiness

- `[ ]` Add VM-based CI scenarios (clean, WSL-only, Docker Desktop preinstalled, Win10 baseline)
- `[~]` GitHub Actions workflow + Azure VM plan drafted (`.github/workflows/windows-wsl-provisioning.yml`, `docs/ci/windows-wsl-provisioning-ci.md`)
- `[ ]` Add resume/reboot/crash recovery integration tests
- `[~]` Add telemetry and log export tooling
  - Added `export_engine_diagnostics` command and Settings `Export diagnostics` action that writes a timestamped diagnostics snapshot JSON under app data diagnostics.
- `[ ]` Run usability pass for non-technical users

## Notes

- The current provisioning runner intentionally uses staged scaffolding with checkpoint persistence and UI polling to stabilize UX/state transitions before privileged installer integration.
- This checklist should be updated per merged PR to keep milestone readiness explicit.
