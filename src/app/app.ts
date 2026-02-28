import { Component, inject, OnInit, signal } from '@angular/core';
import { RouterLink, RouterLinkActive, RouterOutlet } from '@angular/router';
import { CommonModule } from '@angular/common';

import { ConnectionStore } from './stores/connection.store';
import { UiStore } from './stores/ui.store';
import { ToastContainerComponent } from './components/toast-container.component';
import { invoke, errorMessage } from './lib/tauri';
import { ConnectionGuidance, DockerInfo } from './lib/models';

@Component({
  selector: 'app-root',
  standalone: true,
  imports: [CommonModule, RouterOutlet, RouterLink, RouterLinkActive, ToastContainerComponent],
  templateUrl: './app.html',
})
export class App implements OnInit {
  connection = inject(ConnectionStore);
  ui = inject(UiStore);
  guidance = signal<ConnectionGuidance | null>(null);
  guidanceBusy = signal(false);
  limitedMode = signal(false);

  readonly navItems = [
    { path: '/containers', label: 'Containers', icon: 'containers' },
    { path: '/images',     label: 'Images',     icon: 'images' },
    { path: '/volumes',    label: 'Volumes',    icon: 'volumes' },
    { path: '/compose',    label: 'Compose',    icon: 'compose' },
  ] as const;

  ngOnInit(): void { void this.bootstrap(); }

  private async bootstrap(): Promise<void> {
    try {
      await invoke('resume_engine_provisioning_if_needed');
    } catch {
      // Resume is best-effort. Connection guidance handles remaining recovery paths.
    }
    await this.connect();
  }

  private async connect(): Promise<void> {
    try {
      const info = await invoke<DockerInfo>('get_docker_info');
      this.connection.setConnected({
        server_version: info.server_version,
        api_version: info.api_version,
        socket_path: info.socket_path,
      });
      this.guidance.set(null);
      this.limitedMode.set(false);
    } catch {
      this.connection.setDisconnected();
      await this.refreshGuidance();
    }
  }

  async retryConnection(): Promise<void> {
    this.guidanceBusy.set(true);
    try {
      await invoke('check_connection');
      await this.connect();
    } finally {
      this.guidanceBusy.set(false);
    }
  }

  async fixAutomatically(): Promise<void> {
    const consent = window.confirm(
      'Fix automatically may request administrator permission to repair WSL engine setup. Continue?',
    );
    if (!consent) {
      return;
    }

    this.guidanceBusy.set(true);
    try {
      await invoke('repair_active_engine', { consent });
      await this.connect();
    } catch (e) {
      this.connection.setDisconnected();
      this.guidance.set({
        connected: false,
        title: 'Container engine setup needed',
        message: errorMessage(e),
        failure_class: null,
        primary_action: 'fix_automatically',
      });
    } finally {
      this.guidanceBusy.set(false);
    }
  }

  continueLimitedMode(): void {
    this.limitedMode.set(true);
  }

  private async refreshGuidance(): Promise<void> {
    try {
      const data = await invoke<ConnectionGuidance>('get_connection_guidance');
      if (data.connected) {
        await this.connect();
        return;
      }
      this.guidance.set(data);
    } catch (e) {
      this.guidance.set({
        connected: false,
        title: 'Container engine setup needed',
        message: errorMessage(e),
        failure_class: null,
        primary_action: 'fix_automatically',
      });
    }
  }
}
