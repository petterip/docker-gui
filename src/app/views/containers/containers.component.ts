import {
  Component, computed, inject, OnInit, signal
} from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import {
  injectMutation,
  injectQuery,
  injectQueryClient,
} from '@tanstack/angular-query-experimental';

import { ContainerItem } from '../../lib/models';
import { invoke, errorMessage } from '../../lib/tauri';
import { ToastService } from '../../components/toast.service';
import { ConfirmRowComponent } from '../../components/confirm-row.component';
import { ContainerDetailComponent } from './container-detail.component';

type ConfirmAction = 'stop' | 'remove';
interface ConfirmState {
  id: string;
  action: ConfirmAction;
  name: string;
}

@Component({
  selector: 'app-containers',
  standalone: true,
  imports: [CommonModule, FormsModule, ConfirmRowComponent, ContainerDetailComponent],
  templateUrl: './containers.component.html',
})
export class ContainersComponent {
  private queryClient = injectQueryClient();
  private toast = inject(ToastService);

  filter = signal('');
  showStopped = signal(true);
  confirmState = signal<ConfirmState | null>(null);
  selectedContainerId = signal<string | null>(null);

  // ── Queries ──────────────────────────────────────────────────────────────
  containers = injectQuery(() => ({
    queryKey: ['containers'],
    queryFn: () => invoke<ContainerItem[]>('list_containers'),
    refetchInterval: 3_000,
  }));

  filteredContainers = computed(() => {
    const all = this.containers.data() ?? [];
    const q = this.filter().toLowerCase();
    return all
      .filter(c => this.showStopped() || c.state === 'running')
      .filter(c =>
        !q || c.name.toLowerCase().includes(q) || c.image.toLowerCase().includes(q)
      );
  });

  selectedContainer = computed(() => {
    const id = this.selectedContainerId();
    return id ? (this.containers.data()?.find(c => c.id === id) ?? null) : null;
  });

  // ── Mutations ────────────────────────────────────────────────────────────
  private invalidateContainers = () =>
    this.queryClient.invalidateQueries({ queryKey: ['containers'] });

  start = injectMutation(() => ({
    mutationFn: (id: string) => invoke('start_container', { id }),
    onSuccess: this.invalidateContainers,
    onError: (e: unknown) => this.toast.error(errorMessage(e)),
  }));

  stop = injectMutation(() => ({
    mutationFn: (id: string) => invoke('stop_container', { id }),
    onSuccess: this.invalidateContainers,
    onError: (e: unknown) => this.toast.error(errorMessage(e)),
  }));

  restart = injectMutation(() => ({
    mutationFn: (id: string) => invoke('restart_container', { id }),
    onSuccess: this.invalidateContainers,
    onError: (e: unknown) => this.toast.error(errorMessage(e)),
  }));

  remove = injectMutation(() => ({
    mutationFn: ({ id, remove_volumes }: { id: string; remove_volumes: boolean }) =>
      invoke('remove_container', { id, remove_volumes, force: false }),
    onSuccess: () => {
      this.invalidateContainers();
      this.confirmState.set(null);
    },
    onError: (e: unknown) => this.toast.error(errorMessage(e)),
  }));

  // ── Confirm row helpers ──────────────────────────────────────────────────
  requestStop(c: ContainerItem): void {
    this.confirmState.set({ id: c.id, action: 'stop', name: c.name });
  }

  requestRemove(c: ContainerItem): void {
    this.confirmState.set({ id: c.id, action: 'remove', name: c.name });
  }

  cancelConfirm(): void { this.confirmState.set(null); }

  confirmAction(opts: Record<string, boolean>): void {
    const s = this.confirmState();
    if (!s) return;
    if (s.action === 'stop') {
      this.stop.mutate(s.id);
      this.confirmState.set(null);
    } else {
      this.remove.mutate({ id: s.id, remove_volumes: opts['volumes'] ?? false });
    }
  }

  // ── Template helpers ─────────────────────────────────────────────────────
  isConfirming(id: string): boolean {
    return this.confirmState()?.id === id;
  }

  confirmMessage(): string {
    const s = this.confirmState();
    if (!s) return '';
    return s.action === 'stop'
      ? `Stop container ${s.name}?`
      : `Remove container ${s.name}?`;
  }

  confirmOptions() {
    const s = this.confirmState();
    if (s?.action === 'remove') return [{ key: 'volumes', label: 'Also remove anonymous volumes' }];
    return [];
  }

  openDetail(id: string): void { this.selectedContainerId.set(id); }
  closeDetail(): void { this.selectedContainerId.set(null); }

  statusClass(state: string): string {
    switch (state) {
      case 'running':    return 'badge badge-running';
      case 'paused':     return 'badge badge-paused';
      case 'restarting': return 'badge badge-running';
      case 'exited':     return 'badge badge-exited';
      default:           return 'badge badge-stopped';
    }
  }

  formatPorts(c: ContainerItem): string {
    return c.ports
      .map(p => `${p.host_port}→${p.container_port}`)
      .join(', ');
  }

  formatCreated(ts: number): string {
    const diff = Date.now() / 1000 - ts;
    if (diff < 60) return 'just now';
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
    return `${Math.floor(diff / 86400)}d ago`;
  }
}
