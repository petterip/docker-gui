# Windows Engine Provisioning Plan

Implementation checklist: see `docs/plans/windows-engine-provisioning-checklist.md`.

## Objective

Enable the Win64 installer to provision and run a supported local container engine when Docker Desktop is not installed, then connect `docker-gui` automatically.

## Product Requirement: Fail-Safe by Default

This flow must work for users who do not understand Docker, WSL, contexts, sockets, or services.

Non-negotiable behavior:
1. Default path is one click: `Set up automatically`.
2. No hard dead-end screens: every failure screen has `Retry`, `Fix automatically`, and `Continue with limited mode`.
3. No terminal commands are required for normal success.
4. Provisioning is idempotent, resumable, and reboot-safe.
5. If setup cannot complete, app still launches in guided disconnected mode with one-click repair.

## Decision Lock (v1)

Concrete decisions for implementation:
1. Phase 1 engine install target is **WSL Engine** only.
2. `Host Engine` in Settings means:
   - install **Rancher Desktop (Moby mode)** using official distribution channel, or
   - detect and connect to an already-installed compatible host provider.
3. No engine binaries are redistributed inside `docker-gui` installer.
4. All privileged actions are executed by a signed bootstrapper helper, not by the app process.

## Scope

In scope:
- Installer-time detection of existing engine/runtime.
- Guided WSL engine provisioning and first-run auto-connect.
- Settings-driven install/switch between `WSL Engine` and `Host Engine`.
- Safe rollback/resume across failure and reboot.

Out of scope (initial release):
- Enterprise fleet/GPO templates.
- Remote Docker hosts as default onboarding path.
- Silent elevation without explicit consent.

## Constraints and Assumptions

1. Admin rights are required for enabling WSL features and installing system components.
2. Docker Desktop binaries/licenses cannot be redistributed.
3. Third-party installers are fetched from official channels with integrity checks.
4. Existing Docker Desktop/WSL environments must remain untouched unless user explicitly changes provider.

## Target UX

First install when no reachable engine is found:
1. Installer shows `Container Engine Setup`.
2. Primary action: `Set up automatically` (recommended).
3. Installer performs all required steps and reboots/resumes if needed.
4. App opens in connected state without extra configuration.

Failure path:
1. Screen uses plain-language cause and action text.
2. User can choose `Retry`, `Fix automatically`, or `Continue with limited mode`.
3. Limited mode always includes a prominent `Fix automatically` button.

Settings UX (`Settings > Engine`):
1. Show current engine provider and health (`Ready`, `Needs repair`, `Not installed`).
2. Button: `Install another engine`.
3. Wizard choices:
   - `WSL Engine (recommended)`
   - `Host Engine`
4. On install success, ask `Switch now`.
5. Keep previous provider as fallback by default.
6. No transport jargon (`npipe`, `socket`, `context`) in primary UI.

## UX Copy Rules (Required)

1. Use user-facing provider names only: `WSL Engine`, `Host Engine`.
2. Advanced diagnostics are hidden behind `View details`.
3. No raw command text in primary error messages.
4. Each error message includes exactly one recommended primary action.

## Architecture

## 1. Bootstrapper + Privilege Boundary

Components:
1. `Installer Bootstrapper` (elevated, signed).
2. `Provisioning Helper` (elevated child process for repair/install from Settings).
3. `docker-gui app` (non-elevated).

Rules:
1. App never performs privileged OS changes directly.
2. App requests privileged actions via IPC to helper.
3. Helper returns structured status codes and plain-language mapped messages.

## 2. Engine Provider Model

Provider enum:
- `Provider::WslEngine { distro, relay_pipe }`
- `Provider::HostEngine { kind, endpoint }`
- `Provider::CustomHost` (advanced/manual only)

Where `HostEngine.kind` (v1 allowed values):
- `rancher_desktop_moby`
- `existing_compatible_host`

Provider responsibilities:
1. Install (if supported).
2. Start/ensure-running.
3. Resolve endpoint.
4. Health check (`ping`, `version`).
5. Repair.

## 3. Endpoint Contract (No Ambiguity)

Canonical endpoint behavior:
1. `WSL Engine`: app talks to a managed local named pipe relay (example: `npipe:////./pipe/docker_gui_engine`) that proxies to WSL Unix socket.
2. `Host Engine`: app talks to provider-discovered endpoint (named pipe or explicit host URL).
3. `CustomHost`: explicit user-managed endpoint.

Prohibited default behavior:
- No insecure `0.0.0.0:2375` fallback.

## 4. First-Run Orchestrator

On app startup/reconnect:
1. Read persisted active provider.
2. Ensure provider runtime is running.
3. Resolve endpoint from provider contract.
4. Connect and health check.
5. On failure, offer single action `Fix connection`.

## WSL Provisioning Design (Phase 1)

## A. Prerequisites

1. Verify WSL capability and version.
2. Enable required Windows features if missing.
3. Handle reboot-required state with persisted resume checkpoint.

## B. Distro strategy (No user mechanics by default)

Default algorithm:
1. If managed distro exists (`docker-gui-engine`), reuse it.
2. Else if supported Ubuntu distro exists, pick it automatically.
3. Else install managed distro automatically.

Advanced option (`View details`) may allow manual distro selection, but not on default path.

## C. Engine install in distro

Execute non-interactive provisioning script via WSL.

Script tasks:
1. Install Docker Engine + Compose plugin.
2. Configure user/group permissions.
3. Start/enable service.
4. Validate with smoke checks (`docker version`, `docker info`).

## D. Recovery model (Mandatory)

1. Retries with backoff for transient failures.
2. Checkpoints at:
   - before Windows feature enablement
   - before distro creation/import
   - before engine package install
   - before relay registration
3. Resume token persisted locally.
4. Recovery entry points:
   - installer: `Repair setup`
   - app settings: `Fix automatically`

## Rollback Policy (Concrete)

Rollback behavior by stage:
1. Windows features enabled:
   - do not auto-disable; mark as completed prerequisite.
2. Distro created:
   - keep by default; remove only on explicit user action `Remove managed engine`.
3. Engine packages installed:
   - keep by default; mark as installed but unhealthy if validation fails.
4. Provider switch:
   - atomic switch record; if new provider fails health check, revert active provider to previous healthy one.

## Settings: Engine Installer and Switching

Supported user actions:
1. `Install WSL Engine`
2. `Install Host Engine`
3. `Switch active engine`
4. `Repair active engine`
5. `Remove managed engine` (explicit destructive action with confirmation)

Switch rules:
1. Switch requires successful health check of target provider.
2. Failure auto-rolls back to last healthy provider.
3. Existing provider stays installed unless user explicitly removes it.

## Host Engine Plan (Phase 2)

Initial supported host path:
1. Install Rancher Desktop from official channel (`winget` preferred) with verification.
2. Configure Moby mode and validate Docker API compatibility.

Policy override:
1. Rancher Desktop UI installation is disabled and must not be implemented in `docker-gui`.
2. Host Engine flow must only detect/use existing compatible host providers.

Compatibility gate before marking host provider healthy:
1. Containers/images/volumes read-write flows pass.
2. Compose flows pass or are explicitly gated with clear UI messaging.
3. Startup/reconnect reliability meets SLO.

## Security and Compliance

1. Explicit consent for all downloads/installs.
2. Verify artifact integrity (signature/checksum).
3. Least privilege: app non-admin, helper elevated only when needed.
4. SBOM entry for installed components and versions.
5. Legal review for each supported host provider path.
6. Secrets policy:
   - local provider credentials (if any) stored in Windows Credential Manager,
   - no plaintext credential files.

## Telemetry and Diagnostics

Opt-in events:
- stage started/completed/failed
- provider installed/switched/repaired
- rollback executed

Failure classes:
- `prereq_missing`
- `reboot_required`
- `distro_install_failed`
- `engine_install_failed`
- `engine_start_failed`
- `relay_failed`
- `connectivity_failed`

Local logs:
- bootstrapper log path
- helper log path
- app reconnect log path

## Measurable Acceptance Criteria

Fail-safe UX metrics:
1. Fresh machine success rate (no Docker/WSL): >= 95% without terminal.
2. Median clicks to connected state: <= 5 from installer setup screen.
3. Median time to connected state on clean machine: <= 12 minutes (excluding reboot wait).
4. Recovery success for top 5 failure classes: >= 90% using `Fix automatically`.
5. Provider switch success rate: >= 95%; failed switch must preserve previous provider connectivity.

## Rollout Plan

## Milestone 0: RFC + Threat Model

Deliverables:
- bootstrapper/helper architecture
- provider contract
- security/legal sign-off

Exit criteria:
- approved technical spec with command set and rollback matrix

## Milestone 1: WSL Engine MVP

Deliverables:
- one-click WSL provisioning
- resume across reboot
- managed relay endpoint
- guided disconnected mode with repair

Exit criteria:
- acceptance metrics met for WSL path

## Milestone 2: Settings Engine Installer

Deliverables:
- Settings install/repair/switch flows
- provider fallback and rollback handling
- UX copy finalized (no-jargon primary flow)

Exit criteria:
- user can install second provider and switch without terminal

## Milestone 3: Host Engine Support

Deliverables:
- Rancher Desktop host provider integration
- compatibility tests and repair flow

Exit criteria:
- host provider passes compatibility and reliability gates

## Test Strategy

Test environments:
1. Windows 11 fresh machine (no WSL, no Docker).
2. Windows 11 with WSL only.
3. Windows 11 with Docker Desktop preinstalled.
4. Windows 10 supported baseline.

Automated tests:
1. Detection parser unit tests.
2. Provisioning integration tests in VM snapshots.
3. Resume tests (crash/reboot mid-flow).
4. Recovery tests for top failure classes.
5. Provider-switch tests with forced failure rollback.
6. API smoke tests (`check_connection`, `get_docker_info`, core resource lists).

Manual acceptance:
1. Non-technical usability run: complete setup without docs/terminal.
2. Verify uninstall behavior does not remove engines unless explicitly requested.
3. Verify advanced diagnostics are available but not required.

## Immediate Next Steps

1. Write bootstrapper/helper state machine spec (states, transitions, checkpoint IDs).
2. Define provider data schema in backend config.
3. Draft plain-language UX copy set for install, repair, and switch flows.
4. Create VM-based CI pipeline for install/reboot/resume/repair scenarios.
