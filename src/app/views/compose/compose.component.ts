import {
  Component, inject, signal, OnDestroy
} from '@angular/core';
import { CommonModule } from '@angular/common';
import {
  injectMutation,
  injectQuery,
  injectQueryClient,
} from '@tanstack/angular-query-experimental';
import { Subscription } from 'rxjs';

import { StackItem } from '../../lib/models';
import { invoke, errorMessage } from '../../lib/tauri';
import { LogStreamService } from '../../lib/log-stream.service';
import { LogStore } from '../../stores/log.store';
import { ToastService } from '../../components/toast.service';
@Component({
  selector: 'app-compose',
  standalone: true,
  imports: [CommonModule],
  providers: [LogStore],
  templateUrl: './compose.component.html',
})
export class ComposeComponent implements OnDestroy {
  private queryClient = injectQueryClient();
  private toast = inject(ToastService);
  private logStream = inject(LogStreamService);
  logStore = inject(LogStore);

  expandedId = signal<string | null>(null);
  activeLogStackId = signal<string | null>(null);
  private logSub?: Subscription;

  stacks = injectQuery(() => ({
    queryKey: ['stacks'],
    queryFn: () => invoke<StackItem[]>('list_stacks'),
    refetchInterval: 5_000,
  }));

  private invalidate = () => this.queryClient.invalidateQueries({ queryKey: ['stacks'] });

  up = injectMutation(() => ({
    mutationFn: (id: string) => invoke('stack_up', { stack_id: id }),
    onSuccess: this.invalidate,
    onError: (e: unknown) => this.toast.error(errorMessage(e)),
  }));

  down = injectMutation(() => ({
    mutationFn: (id: string) => invoke('stack_down', { stack_id: id, remove_volumes: false }),
    onSuccess: this.invalidate,
    onError: (e: unknown) => this.toast.error(errorMessage(e)),
  }));

  restart = injectMutation(() => ({
    mutationFn: (id: string) => invoke('stack_restart', { stack_id: id }),
    onSuccess: this.invalidate,
    onError: (e: unknown) => this.toast.error(errorMessage(e)),
  }));

  ngOnDestroy(): void { this.logSub?.unsubscribe(); }

  toggle(stack: StackItem): void {
    const current = this.expandedId();
    this.expandedId.set(current === stack.id ? null : stack.id);
  }

  openLogs(stack: StackItem): void {
    if (this.activeLogStackId() === stack.id) return;
    this.logSub?.unsubscribe();
    this.logStore.clear();
    this.activeLogStackId.set(stack.id);
    this.logSub = this.logStream.composeLogs$(stack.id).subscribe({
      next: (raw: string) => this.logStore.append({ stream: 'stdout', text: raw }),
    });
    invoke('stack_logs', { stack_id: stack.id, tail: 200 }).catch(() => {});
  }

  closeLogs(): void {
    this.logSub?.unsubscribe();
    this.activeLogStackId.set(null);
    this.logStore.clear();
  }

  /** Auto-discovered stacks (id prefixed with 'auto-') are not in the registry;
   *  Up/Down/Restart actions will always fail — disable those buttons. */
  isAutoDiscovered(id: string): boolean {
    return id.startsWith('auto-');
  }

  statusClass(status: string): string {
    switch (status) {
      case 'all_running': return 'badge badge-running';
      case 'partial':     return 'badge badge-paused';
      case 'stopped':     return 'badge badge-stopped';
      case 'running':     return 'badge badge-running'; // container state
      case 'exited':      return 'badge badge-exited';
      default:            return 'badge badge-stopped';
    }
  }
}
