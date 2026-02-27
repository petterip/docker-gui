import { ApplicationConfig, provideBrowserGlobalErrorListeners } from '@angular/core';
import { provideRouter, withComponentInputBinding } from '@angular/router';
import { QueryClient, provideTanStackQuery } from '@tanstack/angular-query-experimental';

import { routes } from './app.routes';

export const appConfig: ApplicationConfig = {
  providers: [
    provideBrowserGlobalErrorListeners(),
    provideRouter(routes, withComponentInputBinding()),
    provideTanStackQuery(new QueryClient({
      defaultOptions: {
        queries: {
          retry: 1,
          staleTime: 2_000,
        },
      },
    })),
    // ConnectionStore, UiStore, ToastService all use providedIn:'root' — do not list here
  ],
};
