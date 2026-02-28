# Windows WSL Provisioning CI Pipeline

Last updated: 2026-02-28

This document describes the GitHub Actions workflow and VM harness used to validate the Windows WSL Engine provisioning flow. The pipeline must verify install → reboot → resume → repair scenarios on a clean Windows 11 VM before we run manual builds.

## Goals

1. Prove that one-click provisioning succeeds on a fresh Windows 11 image with no Docker/WSL installed.
2. Exercise reboot/resume checkpoints by forcing DISM to require a reboot and validating the helper restarts automatically.
3. Validate repair flows (Fix automatically) after injecting failures mid-run.
4. Export helper/bootstrapper logs as workflow artifacts for debugging.

## High-Level Architecture

```
GitHub Actions runner (ubuntu-latest)
  └─ uses azure/login to connect to provisioning subscription
      └─ deploys ephemeral Windows 11 Gen2 VM via Bicep template
          ├─ installs latest docker-gui installer via WinGet
          ├─ runs bootstrapper via PowerShell remoting
          ├─ forces reboot mid-run (optional scenario)
          ├─ polls helper logs + provisioning checkpoints
          ├─ exports event logs, relay logs, engine_providers.json
          └─ deletes VM at end of job
```

## Workflow Outline

Create `.github/workflows/windows-wsl-provisioning.yml` with the following jobs:

1. `build-helper` (runs on `windows-latest`)
   - Checks out repo
   - Runs `scripts/dev-cycle.sh install`
   - Builds Tauri helper binary (`cargo build --bin docker-gui-provisioning-helper --release`)
   - Uploads artifact `provisioning-helper` for reuse

2. `provisioning-e2e` (runs on `ubuntu-latest`)
   - Needs `build-helper`
   - Logs into Azure using federated credentials
   - Deploys Bicep/ARM template `infra/windows-wsl-lab.bicep` (to be added) that provisions:
     - Windows 11 Pro Gen2 VM
     - Bootstrap script extension that downloads the helper artifact + unsigned installer build
   - Executes PowerShell script `scripts/ci/run-windows-wsl-tests.ps1` that:
     1. Enables script logging and remoting
     2. Copies helper + app package to VM
     3. Runs the installer with `/quiet /log installer.log`
     4. Waits for `engine_providers.json` to show `status="running"`
     5. Forces reboot by invoking `Restart-Computer -Force` when checkpoint=`before_distro`
     6. After reboot, waits for provisioning to succeed, then triggers `Fix automatically` via helper CLI to validate repair path
     7. Collects logs into `C:\docker-gui-logs`
   - Downloads logs via `az vm run-command invoke` + `az storage` upload
   - Uploads logs as workflow artifact
   - Destroys VM (even on failure) using `az group delete --yes --no-wait`

## scripts/ci/run-windows-wsl-tests.ps1 (outline)

```powershell
param(
  [string]$HelperPath,
  [string]$InstallerPath,
  [string]$LogDir = 'C:\\docker-gui-logs'
)

New-Item -ItemType Directory -Path $LogDir -Force | Out-Null

Start-Process -FilePath $InstallerPath -ArgumentList '/quiet', '/log', "$LogDir\\installer.log" -Wait

$providersJson = "$env:APPDATA\\docker-gui\\engine_providers.json"

function Wait-Checkpoint($checkpoint) {
  for ($i=0; $i -lt 30; $i++) {
    if (Test-Path $providersJson) {
      $state = Get-Content $providersJson | ConvertFrom-Json
      if ($state.resume_checkpoint -eq $checkpoint) {
        return $true
      }
    }
    Start-Sleep -Seconds 10
  }
  throw "Timeout waiting for checkpoint $checkpoint"
}

Wait-Checkpoint 'before_distro'
Restart-Computer -Force

Wait-Checkpoint $null

& $HelperPath run-action --action wsl_relay_register --target-json '{"provider":"wsl_engine","distro":"docker-gui-engine","relay_pipe":"npipe:////./pipe/docker_gui_engine"}' --app-data-dir $env:APPDATA | Out-File "$LogDir\\repair.log"

Compress-Archive -Path $env:APPDATA\\docker-gui\\* -DestinationPath "$LogDir\\appdata.zip" -Force
```

## Reporting

- Workflow summary must include:
  - Duration of provisioning
  - Stage where reboot was triggered
  - Any failure_class values encountered
- Logs are attached for consumption during manual debugging sessions.

## Follow-Ups

1. Add `infra/windows-wsl-lab.bicep` with parameters for SKU, admin username, and diagnostics storage.
2. Gate the workflow behind a `workflow_dispatch` with inputs (`run_repair`, `retain_vm`) so engineers can opt-in during investigations.
3. Integrate results with the acceptance criteria dashboard once telemetry endpoints exist.

