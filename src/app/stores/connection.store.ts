import { computed, inject, Injectable, signal } from '@angular/core';
import { QueryClient } from '@tanstack/angular-query-experimental';

export type ConnectionStatus = 'connected' | 'connecting' | 'disconnected';

@Injectable({ providedIn: 'root' })
export class ConnectionStore {
  private queryClient = inject(QueryClient);

  status = signal<ConnectionStatus>('connecting');
  version = signal<string | null>(null);
  apiVersion = signal<string | null>(null);
  socketPath = signal<string>('');

  isConnected = computed(() => this.status() === 'connected');
  isDisconnected = computed(() => this.status() === 'disconnected');

  setConnected(info: { server_version: string; api_version: string; socket_path: string }): void {
    this.status.set('connected');
    this.version.set(info.server_version);
    this.apiVersion.set(info.api_version);
    this.socketPath.set(info.socket_path);
  }

  onReconnect(): void {
    this.status.set('connected');
    // Invalidate all stale data from before the outage
    this.queryClient.invalidateQueries();
  }

  setDisconnected(): void {
    this.status.set('disconnected');
    this.version.set(null);
  }
}
