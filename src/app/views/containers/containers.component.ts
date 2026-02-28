import {
  Component, computed, effect, inject, signal
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

type ContainerColumn = 'name' | 'image' | 'status' | 'ports' | 'created' | 'actions';
type ColumnWidths = Record<ContainerColumn, number>;

@Component({
  selector: 'app-containers',
  standalone: true,
  imports: [CommonModule, FormsModule, ConfirmRowComponent, ContainerDetailComponent],
  templateUrl: './containers.component.html',
})
export class ContainersComponent {
  private queryClient = injectQueryClient();
  private toast = inject(ToastService);
  private readonly composeProjectLabel = 'com.docker.compose.project';
  private readonly widthsStorageKey = 'containers.columnWidths.v1';

  filter = signal('');
  showStopped = signal(true);
  confirmState = signal<ConfirmState | null>(null);
  selectedContainerId = signal<string | null>(null);
  columnWidths = signal<ColumnWidths>(this.loadColumnWidths());

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

  groupedContainers = computed(() => {
    const map = new Map<string, ContainerItem[]>();
    for (const container of this.filteredContainers()) {
      const project = this.getComposeProject(container);
      const list = map.get(project) ?? [];
      list.push(container);
      map.set(project, list);
    }

    return Array.from(map.entries())
      .sort(([a], [b]) => {
        if (a === 'Standalone') return 1;
        if (b === 'Standalone') return -1;
        return a.localeCompare(b);
      })
      .map(([project, containers]) => ({ project, containers }));
  });

  selectedContainer = computed(() => {
    const id = this.selectedContainerId();
    return id ? (this.containers.data()?.find(c => c.id === id) ?? null) : null;
  });

  constructor() {
    effect(() => {
      localStorage.setItem(this.widthsStorageKey, JSON.stringify(this.columnWidths()));
    });
  }

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

  columnWidth(key: ContainerColumn): number {
    return this.columnWidths()[key];
  }

  startResize(event: MouseEvent, key: ContainerColumn): void {
    event.preventDefault();
    event.stopPropagation();
    const startX = event.clientX;
    const startWidth = this.columnWidths()[key];

    const onMove = (moveEvent: MouseEvent) => {
      const delta = moveEvent.clientX - startX;
      const next = Math.max(90, startWidth + delta);
      this.columnWidths.update(w => ({ ...w, [key]: next }));
    };

    const onUp = () => {
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
    };

    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
  }

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

  private loadColumnWidths(): ColumnWidths {
    const defaults: ColumnWidths = {
      name: 220,
      image: 480,
      status: 130,
      ports: 320,
      created: 120,
      actions: 130,
    };

    const raw = localStorage.getItem(this.widthsStorageKey);
    if (!raw) return defaults;
    try {
      const parsed = JSON.parse(raw) as Partial<ColumnWidths>;
      return {
        name: this.coerceWidth(parsed.name, defaults.name),
        image: this.coerceWidth(parsed.image, defaults.image),
        status: this.coerceWidth(parsed.status, defaults.status),
        ports: this.coerceWidth(parsed.ports, defaults.ports),
        created: this.coerceWidth(parsed.created, defaults.created),
        actions: this.coerceWidth(parsed.actions, defaults.actions),
      };
    } catch {
      return defaults;
    }
  }

  private coerceWidth(value: number | undefined, fallback: number): number {
    return typeof value === 'number' && Number.isFinite(value) ? Math.max(90, value) : fallback;
  }

  private getComposeProject(container: ContainerItem): string {
    const labels = container.labels ?? {};
    const direct = labels[this.composeProjectLabel];
    if (direct) return direct;

    for (const [key, value] of Object.entries(labels)) {
      if (key.toLowerCase() === this.composeProjectLabel && value) {
        return value;
      }
    }

    // Fallback for compose-like names when labels are missing.
    // Example: myproj-service-1 -> myproj
    const match = container.name.match(/^([a-zA-Z0-9._-]+)-[a-zA-Z0-9._-]+-\d+$/);
    if (match?.[1]) return match[1];
    return 'Standalone';
  }
}
