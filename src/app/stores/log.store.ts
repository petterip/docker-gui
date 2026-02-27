import { Injectable, signal } from '@angular/core';
import { LogLine } from '../lib/models';

@Injectable() // declared per-component — no providedIn
export class LogStore {
  private readonly MAX_LINES = 5_000;

  readonly lines = signal<LogLine[]>([]);
  readonly autoScroll = signal(true);

  append(line: LogLine): void {
    this.lines.update(buf => {
      const next = [...buf, line];
      return next.length > this.MAX_LINES ? next.slice(-this.MAX_LINES) : next;
    });
  }

  clear(): void {
    this.lines.set([]);
  }
}
