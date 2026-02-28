# Windows Engine Provider Data Schema

Last updated: 2026-02-28

This document defines the serialized shape of `engine_providers.json`, the registry file that coordinates provider state between the desktop app, provisioning helper, and bootstrapper. All components MUST treat this schema as stable and backwards-compatible; additive changes require version gating.

## File Location

- Windows: `%APPDATA%/docker-gui/engine_providers.json`
- macOS/Linux/WSL: `$XDG_CONFIG_HOME/docker-gui/engine_providers.json` (falls back to `$HOME/.config/...`)

The file is written atomically via `EngineRegistry::flush_atomic` and should never be modified by external tools.

## Top-Level Schema

```jsonc
{
  "active_provider": Provider | null,
  "previous_provider": Provider | null,
  "preferred_wsl_distro": string | null,
  "resume_checkpoint": string | null,
  "resume_privileged_allowed": boolean,
  "provisioning": ProvisioningState | null
}
```

### Provider Variants

```jsonc
// Provider::WslEngine
{
  "provider": "wsl_engine",
  "distro": "docker-gui-engine",
  "relay_pipe": "npipe:////./pipe/docker_gui_engine"
}

// Provider::HostEngine
{
  "provider": "host_engine",
  "kind": "existing_compatible_host", // future: "podman_wsl_api"
  "endpoint": "npipe:////./pipe/docker_engine"
}

// Provider::CustomHost
{
  "provider": "custom_host",
  "endpoint": "tcp://192.168.1.10:2375"
}
```

### ProvisioningState

```jsonc
{
  "run_id": "uuid",
  "target_provider_id": "wsl_engine",
  "status": "running" | "succeeded" | "failed",
  "stages": [ProvisioningStage],
  "started_at": "RFC3339",
  "updated_at": "RFC3339",
  "finished_at": "RFC3339" | null
}
```

### ProvisioningStage

```jsonc
{
  "id": "before_distro",
  "label": "Preparing WSL distro",
  "status": "pending" | "in_progress" | "completed" | "failed",
  "failure_class": "distro_install_failed" | null,
  "message": "human friendly status" | null
}
```

## Contract Rules

1. **Idempotent updates** – Writers MUST lock, mutate, and flush atomically. Never partially update nested objects.
2. **Provider identity** – `provider` strings are canonical IDs surfaced directly to the UI. Adding a new provider requires updating UI copy, telemetry, and schema docs.
3. **Relay pipe normalization** – `relay_pipe` must always be a fully-qualified named pipe string to avoid ambiguity when reconstructed by the app.
4. **Preferred distro** – When unset, the app infers the managed distro or autodetects Ubuntu. Helpers should not populate this field directly; it is user preference.
5. **Resume consent** – `resume_privileged_allowed` is set by `set_resume_checkpoint_with_privilege`. Helpers MUST read this flag before executing operations after reboot.
6. **Stage alignment** – `stages[].id` must match `provisioning_stage_specs()` entries so UI progress bars stay consistent with helper checkpoints.
7. **Compatibility** – Unknown fields must be ignored to allow forward-compatible extensions, but helpers should log a warning when encountering keys they do not understand.

## Sample File

```json
{
  "active_provider": {
    "provider": "wsl_engine",
    "distro": "docker-gui-engine",
    "relay_pipe": "npipe:////./pipe/docker_gui_engine"
  },
  "previous_provider": {
    "provider": "host_engine",
    "kind": "existing_compatible_host",
    "endpoint": "npipe:////./pipe/docker_engine"
  },
  "preferred_wsl_distro": "docker-gui-engine",
  "resume_checkpoint": null,
  "resume_privileged_allowed": false,
  "provisioning": null
}
```

