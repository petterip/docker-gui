import { Routes } from '@angular/router';

export const routes: Routes = [
  {
    path: '',
    redirectTo: 'containers',
    pathMatch: 'full',
  },
  {
    path: 'containers',
    loadComponent: () =>
      import('./views/containers/containers.component').then(m => m.ContainersComponent),
  },
  {
    path: 'images',
    loadComponent: () =>
      import('./views/images/images.component').then(m => m.ImagesComponent),
  },
  {
    path: 'volumes',
    loadComponent: () =>
      import('./views/volumes/volumes.component').then(m => m.VolumesComponent),
  },
  {
    path: 'compose',
    loadComponent: () =>
      import('./views/compose/compose.component').then(m => m.ComposeComponent),
  },
  {
    path: 'settings',
    loadComponent: () =>
      import('./views/settings/settings.component').then(m => m.SettingsComponent),
  },
  { path: '**', redirectTo: 'containers' },
];
