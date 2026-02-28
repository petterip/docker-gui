param(
  [Parameter(Mandatory = $true)]
  [string]$HelperPath,
  [Parameter(Mandatory = $true)]
  [string]$InstallerPath,
  [string]$LogDir = 'C:\docker-gui-logs',
  [string]$CheckpointTimeoutMinutes = 10,
  [switch]$RunRepair
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Write-Log {
  param([string]$Message)
  $timestamp = Get-Date -Format o
  $entry = \"[$timestamp] $Message\"
  Write-Host $entry
  if (!(Test-Path $LogDir)) {
    New-Item -ItemType Directory -Path $LogDir -Force | Out-Null
  }
  Add-Content -Path (Join-Path $LogDir 'provisioning-ci.log') -Value $entry
}

function Get-ProviderState {
  $providersJson = Join-Path $env:APPDATA 'docker-gui\engine_providers.json'
  if (-not (Test-Path $providersJson)) {
    return $null
  }
  try {
    return Get-Content $providersJson -Raw | ConvertFrom-Json
  } catch {
    Write-Log \"Failed to parse engine_providers.json: $_\"
    return $null
  }
}

function Wait-Checkpoint {
  param(
    [string]$ExpectedCheckpoint,
    [double]$TimeoutMinutes = 10
  )
  $deadline = (Get-Date).AddMinutes($TimeoutMinutes)
  while ((Get-Date) -lt $deadline) {
    $state = Get-ProviderState
    if ($null -eq $state) {
      Start-Sleep -Seconds 10
      continue
    }
    if ($ExpectedCheckpoint -eq $null) {
      if ([string]::IsNullOrEmpty($state.resume_checkpoint)) {
        Write-Log 'Checkpoint cleared.'
        return
      }
    } elseif ($state.resume_checkpoint -eq $ExpectedCheckpoint) {
      Write-Log \"Reached checkpoint $ExpectedCheckpoint\"
      return
    }
    Start-Sleep -Seconds 10
  }
  throw \"Timeout waiting for checkpoint '$ExpectedCheckpoint'\"
}

Write-Log 'Starting Windows WSL provisioning CI harness'
Write-Log \"Installer: $InstallerPath\"
Write-Log \"Helper: $HelperPath\"

if (-not (Test-Path $InstallerPath)) {
  throw \"Installer path not found: $InstallerPath\"
}
if (-not (Test-Path $HelperPath)) {
  throw \"Helper path not found: $HelperPath\"
}

New-Item -ItemType Directory -Path $LogDir -Force | Out-Null

Write-Log 'Running installer'
Start-Process -FilePath $InstallerPath -ArgumentList '/quiet', \"/log\", (Join-Path $LogDir 'installer.log') -Wait

Write-Log 'Waiting for provisioning checkpoint before_distro'
Wait-Checkpoint -ExpectedCheckpoint 'before_distro' -TimeoutMinutes $CheckpointTimeoutMinutes

Write-Log 'Waiting for provisioning completion'
Wait-Checkpoint -ExpectedCheckpoint $null -TimeoutMinutes $CheckpointTimeoutMinutes

if ($RunRepair) {
  Write-Log 'Triggering Fix automatically (relay registration)'
  $targetJson = '{\"provider\":\"wsl_engine\",\"distro\":\"docker-gui-engine\",\"relay_pipe\":\"npipe:////./pipe/docker_gui_engine\"}'
  & $HelperPath run-action --action wsl_relay_register --target-json $targetJson --app-data-dir $env:APPDATA | Tee-Object -FilePath (Join-Path $LogDir 'repair.log')
}

Write-Log 'Collecting diagnostics'
$diagDir = Join-Path $LogDir 'appdata'
if (Test-Path $diagDir) {
  Remove-Item $diagDir -Recurse -Force
}
Copy-Item -Path (Join-Path $env:APPDATA 'docker-gui\*') -Destination $diagDir -Recurse -Force

Write-Log 'Windows WSL provisioning CI harness completed'
