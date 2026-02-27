import {
  Component, computed, inject, signal
} from '@angular/core';
import { CommonModule } from '@angular/common';
import {
  injectMutation,
  injectQuery,
  injectQueryClient,
} from '@tanstack/angular-query-experimental';

import { VolumeItem } from '../../lib/models';
import { invoke, errorMessage } from '../../lib/tauri';
import { ToastService } from '../../components/toast.service';
import { ConfirmRowComponent } from '../../components/confirm-row.component';

@Component({
  selector: 'app-volumes',
  standalone: true,
  imports: [CommonModule, ConfirmRowComponent],
  templateUrl: './volumes.component.html',
})
export class VolumesComponent {
  private queryClient = injectQueryClient();
  private toast = inject(ToastService);

  filter = signal('');
  confirmRemoveId = signal<string | null>(null);
  showCreate = signal(false);
  newVolumeName = signal('');

  volumes = injectQuery(() => ({
    queryKey: ['volumes'],
    queryFn: () => invoke<VolumeItem[]>('list_volumes'),
    refetchInterval: 10_000,
  }));

  filteredVolumes = computed(() => {
    const all = this.volumes.data() ?? [];
    const q = this.filter().toLowerCase();
    return !q ? all : all.filter(v => v.name.toLowerCase().includes(q) || v.driver.toLowerCase().includes(q));
  });

  private invalidate = () => this.queryClient.invalidateQueries({ queryKey: ['volumes'] });

  create = injectMutation(() => ({
    mutationFn: (name: string) => invoke('create_volume', { name }),
    onSuccess: () => {
      this.invalidate();
      this.showCreate.set(false);
      this.newVolumeName.set('');
      this.toast.success('Volume created');
    },
    onError: (e: unknown) => this.toast.error(errorMessage(e)),
  }));

  remove = injectMutation(() => ({
    mutationFn: (name: string) => invoke('remove_volume', { name }),
    onSuccess: () => { this.invalidate(); this.confirmRemoveId.set(null); },
    onError: (e: unknown) => this.toast.error(errorMessage(e)),
  }));

  requestRemove(v: VolumeItem): void { this.confirmRemoveId.set(v.name); }
  cancelRemove(): void { this.confirmRemoveId.set(null); }
  confirmRemove(): void {
    const name = this.confirmRemoveId();
    if (name) this.remove.mutate(name);
  }

  submitCreate(): void {
    const name = this.newVolumeName().trim();
    if (name) this.create.mutate(name);
  }
}
