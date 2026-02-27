import { Injectable, signal } from '@angular/core';

export type Theme = 'dark' | 'light' | 'system';

@Injectable({ providedIn: 'root' })
export class UiStore {
  theme = signal<Theme>('dark');

  setTheme(theme: Theme): void {
    this.theme.set(theme);
    const resolved = theme === 'system'
      ? (window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark')
      : theme;
    document.documentElement.dataset['theme'] = resolved;
  }
}
