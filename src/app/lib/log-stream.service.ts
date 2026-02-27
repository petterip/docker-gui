import { listen } from '@tauri-apps/api/event';
import { Injectable } from '@angular/core';
import { Observable } from 'rxjs';
import { LogLine } from './models';

@Injectable({ providedIn: 'root' })
export class LogStreamService {
  /**
   * Returns an Observable of log lines for the given container ID.
   * The Tauri event channel is automatically unsubscribed on unsubscribe.
   */
  containerLogs$(id: string): Observable<LogLine> {
    return new Observable<LogLine>(subscriber => {
      const unlistenPromise = listen<LogLine>(`container-log-${id}`, event =>
        subscriber.next(event.payload)
      );
      return () => {
        unlistenPromise.then(fn => fn());
      };
    });
  }

  composeLogs$(stackId: string): Observable<string> {
    return new Observable<string>(subscriber => {
      const unlistenPromise = listen<string>(`compose-log-${stackId}`, event =>
        subscriber.next(event.payload)
      );
      return () => {
        unlistenPromise.then(fn => fn());
      };
    });
  }
}
