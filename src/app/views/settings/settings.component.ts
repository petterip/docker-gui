import { Component, inject, OnDestroy, OnInit, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { UiStore } from '../../stores/ui.store';
import { ConnectionStore } from '../../stores/connection.store';
import { invoke, errorMessage } from '../../lib/tauri';
import { ToastService } from '../../components/toast.service';
import { DockerInfo, EngineProviderStatus, EngineStatus, ProvisioningState } from '../../lib/models';

@Component({
  selector: 'app-settings',
  standalone: true,
  imports: [CommonModule],
  templateUrl: './settings.component.html',
})
export class SettingsComponent implements OnInit, OnDestroy {
  ui = inject(UiStore);
  connection = inject(ConnectionStore);
  private toast = inject(ToastService);

  info = signal<DockerInfo | null>(null);
  infoError = signal<string | null>(null);
  engine = signal<EngineStatus | null>(null);
  provisioning = signal<ProvisioningState | null>(null);
  engineError = signal<string | null>(null);
  engineBusy = signal(false);
  loading = signal(false);
  private provisioningPoll: ReturnType<typeof setInterval> | null = null;

  ngOnInit(): void { this.load(); }
  ngOnDestroy(): void { this.stopProvisioningPoll(); }

  async load(): Promise<void> {
    this.loading.set(true);
    this.infoError.set(null);
    this.engineError.set(null);
    try {
      const engine = await invoke<EngineStatus>('get_engine_status');
      this.syncEngineStatus(engine);
    } catch (e) {
      this.engineError.set(errorMessage(e));
    }

    try {
      const info = await invoke<DockerInfo>('get_docker_info');
      this.info.set(info);
    } catch (e) {
      this.infoError.set(errorMessage(e));
    } finally {
      this.loading.set(false);
    }
  }

  async reconnect(): Promise<void> {
    const needsConsent = this.engine()?.active_provider_id === 'wsl_engine';
    const consent =
      !needsConsent ||
      window.confirm(
        'Fix automatically may request administrator permission to repair WSL engine setup. Continue?',
      );
    if (!consent) {
      return;
    }

    this.loading.set(true);
    try {
      await invoke('repair_active_engine', { consent });
      await this.load();
      this.toast.success('Reconnected to Docker');
    } catch (e) {
      this.toast.error(errorMessage(e));
    } finally {
      this.loading.set(false);
    }
  }

  healthLabel(health: EngineProviderStatus['health']): string {
    if (health === 'ready') return 'Ready';
    if (health === 'needs_repair') return 'Needs repair';
    return 'Not installed';
  }

  async installProvider(provider: 'wsl_engine' | 'host_engine'): Promise<void> {
    const needsConsent = provider === 'wsl_engine';
    const consent =
      !needsConsent ||
      window.confirm(
        'Installing WSL Engine may request administrator permission to enable Windows features and install engine components. Continue?',
      );
    if (!consent) {
      return;
    }

    this.engineBusy.set(true);
    this.engineError.set(null);
    try {
      const p = await invoke<ProvisioningState>('start_engine_provisioning', { provider, consent });
      this.provisioning.set(p);
      this.startProvisioningPoll();
      this.toast.success('Provisioning started');
    } catch (e) {
      this.engineError.set(errorMessage(e));
      this.toast.error(errorMessage(e));
    } finally {
      this.engineBusy.set(false);
    }
  }

  async switchProvider(provider: 'wsl_engine' | 'host_engine'): Promise<void> {
    this.engineBusy.set(true);
    this.engineError.set(null);
    try {
      const status = await invoke<EngineStatus>('switch_active_engine', { provider });
      this.syncEngineStatus(status);
      await this.load();
      this.toast.success(provider === 'wsl_engine' ? 'Switched to WSL Engine' : 'Switched to Host Engine');
    } catch (e) {
      this.engineError.set(errorMessage(e));
      this.toast.error(errorMessage(e));
    } finally {
      this.engineBusy.set(false);
    }
  }

  async repairEngine(): Promise<void> {
    await this.reconnect();
  }

  async removeManagedEngine(): Promise<void> {
    this.engineBusy.set(true);
    this.engineError.set(null);
    try {
      const status = await invoke<EngineStatus>('remove_managed_engine');
      this.syncEngineStatus(status);
      await this.load();
      this.toast.success('Managed engine removed from configuration');
    } catch (e) {
      this.engineError.set(errorMessage(e));
      this.toast.error(errorMessage(e));
    } finally {
      this.engineBusy.set(false);
    }
  }

  async retryProvisioning(): Promise<void> {
    const needsConsent = this.provisioning()?.target_provider_id === 'wsl_engine';
    const consent =
      !needsConsent ||
      window.confirm(
        'Retry provisioning may request administrator permission to continue WSL engine setup. Continue?',
      );
    if (!consent) {
      return;
    }

    this.engineBusy.set(true);
    this.engineError.set(null);
    try {
      const p = await invoke<ProvisioningState>('retry_engine_provisioning', { consent });
      this.provisioning.set(p);
      this.startProvisioningPoll();
      this.toast.success('Provisioning retried');
    } catch (e) {
      this.engineError.set(errorMessage(e));
      this.toast.error(errorMessage(e));
    } finally {
      this.engineBusy.set(false);
    }
  }

  stageBadgeClass(status: string): string {
    if (status === 'running') return 'badge-paused';
    if (status === 'succeeded') return 'badge-running';
    if (status === 'completed') return 'badge-running';
    if (status === 'in_progress') return 'badge-paused';
    if (status === 'failed') return 'badge-error';
    return 'badge-stopped';
  }

  private syncEngineStatus(engine: EngineStatus): void {
    this.engine.set(engine);
    this.provisioning.set(engine.provisioning);
    if (engine.provisioning?.status === 'running') {
      this.startProvisioningPoll();
    } else {
      this.stopProvisioningPoll();
    }
  }

  private startProvisioningPoll(): void {
    if (this.provisioningPoll) return;
    this.provisioningPoll = setInterval(async () => {
      try {
        const engine = await invoke<EngineStatus>('get_engine_status');
        this.syncEngineStatus(engine);
        if (engine.provisioning?.status === 'succeeded') {
          this.toast.success('Provisioning completed');
          await this.load();
        } else if (engine.provisioning?.status === 'failed') {
          this.toast.error('Provisioning failed. Retry to continue setup.');
        }
      } catch {
        this.stopProvisioningPoll();
      }
    }, 1000);
  }

  private stopProvisioningPoll(): void {
    if (!this.provisioningPoll) return;
    clearInterval(this.provisioningPoll);
    this.provisioningPoll = null;
  }

  setTheme(t: 'dark' | 'light'): void { this.ui.setTheme(t); }
}
