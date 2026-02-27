import { Component, inject, OnInit } from '@angular/core';
import { RouterLink, RouterLinkActive, RouterOutlet } from '@angular/router';
import { CommonModule } from '@angular/common';

import { ConnectionStore } from './stores/connection.store';
import { UiStore } from './stores/ui.store';
import { ToastContainerComponent } from './components/toast-container.component';
import { invoke, errorMessage } from './lib/tauri';
import { DockerInfo } from './lib/models';

@Component({
  selector: 'app-root',
  standalone: true,
  imports: [CommonModule, RouterOutlet, RouterLink, RouterLinkActive, ToastContainerComponent],
  templateUrl: './app.html',
})
export class App implements OnInit {
  connection = inject(ConnectionStore);
  ui = inject(UiStore);

  readonly navItems = [
    { path: '/containers', label: 'Containers', icon: 'containers' },
    { path: '/images',     label: 'Images',     icon: 'images' },
    { path: '/volumes',    label: 'Volumes',    icon: 'volumes' },
    { path: '/compose',    label: 'Compose',    icon: 'compose' },
  ] as const;

  ngOnInit(): void { this.connect(); }

  private async connect(): Promise<void> {
    try {
      const info = await invoke<DockerInfo>('get_docker_info');
      this.connection.setConnected({
        server_version: info.server_version,
        api_version: info.api_version,
        socket_path: info.socket_path,
      });
    } catch {
      this.connection.setDisconnected();
    }
  }
}
