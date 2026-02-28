import {
  Component, EventEmitter, inject, Input, OnDestroy, OnInit, Output, signal
} from '@angular/core';
import { CommonModule } from '@angular/common';
import { injectQuery } from '@tanstack/angular-query-experimental';
import { Subscription } from 'rxjs';

import { ContainerItem, LogLine } from '../../lib/models';
import { invoke } from '../../lib/tauri';
import { LogStreamService } from '../../lib/log-stream.service';
import { LogStore } from '../../stores/log.store';

type Tab = 'overview' | 'logs' | 'inspect';

@Component({
  selector: 'app-container-detail',
  standalone: true,
  imports: [CommonModule],
  providers: [LogStore],
  templateUrl: './container-detail.component.html',
})
export class ContainerDetailComponent implements OnInit, OnDestroy {
  @Input({ required: true }) container!: ContainerItem;
  @Input() initialTab: Tab = 'overview';
  @Output() closed = new EventEmitter<void>();

  private logStream = inject(LogStreamService);
  logStore = inject(LogStore);

  activeTab = signal<Tab>('overview');
  isLogsFullscreen = signal(false);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  inspectData = signal<any>(null);

  private logSub?: Subscription;

  inspect = injectQuery(() => ({
    queryKey: ['container-inspect', this.container.id],
    queryFn: () => invoke<unknown>('inspect_container', { id: this.container.id }),
    enabled: this.activeTab() === 'inspect',
    staleTime: 5_000,
  }));

  ngOnInit(): void {
    this.activeTab.set(this.initialTab);
    this.startLogs();
  }

  ngOnDestroy(): void {
    this.logSub?.unsubscribe();
  }

  private startLogs(): void {
    this.logSub = this.logStream
      .containerLogs$(this.container.id)
      .subscribe({
        next: (line: LogLine) => this.logStore.append(line),
      });
    // kick off streaming
    invoke('get_container_logs', { id: this.container.id, tail: 200 }).catch(() => {});
  }

  setTab(tab: Tab): void {
    this.activeTab.set(tab);
    if (tab !== 'logs') {
      this.isLogsFullscreen.set(false);
    }
  }

  toggleLogsFullscreen(): void {
    this.isLogsFullscreen.update(v => !v);
  }

  async copyLogs(): Promise<void> {
    const content = this.logStore.lines().map(line => line.text).join('');
    await navigator.clipboard.writeText(content);
  }

  formatJson(val: unknown): string {
    return JSON.stringify(val, null, 2);
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

  close(): void { this.closed.emit(); }
}
