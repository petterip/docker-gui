import { Component, inject, OnInit, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { UiStore } from '../../stores/ui.store';
import { ConnectionStore } from '../../stores/connection.store';
import { invoke, errorMessage } from '../../lib/tauri';
import { ToastService } from '../../components/toast.service';
import { DockerInfo } from '../../lib/models';

@Component({
  selector: 'app-settings',
  standalone: true,
  imports: [CommonModule],
  templateUrl: './settings.component.html',
})
export class SettingsComponent implements OnInit {
  ui = inject(UiStore);
  connection = inject(ConnectionStore);
  private toast = inject(ToastService);

  info = signal<DockerInfo | null>(null);
  infoError = signal<string | null>(null);
  loading = signal(false);

  ngOnInit(): void { this.load(); }

  async load(): Promise<void> {
    this.loading.set(true);
    this.infoError.set(null);
    try {
      const data = await invoke<DockerInfo>('get_docker_info');
      this.info.set(data);
    } catch (e) {
      this.infoError.set(errorMessage(e));
    } finally {
      this.loading.set(false);
    }
  }

  async reconnect(): Promise<void> {
    this.loading.set(true);
    try {
      await invoke('check_connection');
      await this.load();
      this.toast.success('Reconnected to Docker');
    } catch (e) {
      this.toast.error(errorMessage(e));
    } finally {
      this.loading.set(false);
    }
  }

  setTheme(t: 'dark' | 'light'): void { this.ui.setTheme(t); }
}
