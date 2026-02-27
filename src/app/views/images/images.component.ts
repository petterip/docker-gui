import {
  Component, computed, inject, signal
} from '@angular/core';
import { CommonModule } from '@angular/common';
import {
  injectMutation,
  injectQuery,
  injectQueryClient,
} from '@tanstack/angular-query-experimental';

import { ImageItem } from '../../lib/models';
import { invoke, errorMessage } from '../../lib/tauri';
import { ToastService } from '../../components/toast.service';
import { ConfirmRowComponent } from '../../components/confirm-row.component';

@Component({
  selector: 'app-images',
  standalone: true,
  imports: [CommonModule, ConfirmRowComponent],
  templateUrl: './images.component.html',
})
export class ImagesComponent {
  private queryClient = injectQueryClient();
  private toast = inject(ToastService);

  filter = signal('');
  confirmRemoveId = signal<string | null>(null);
  pullName = signal('');
  pulling = signal(false);
  pullProgress = signal('');

  images = injectQuery(() => ({
    queryKey: ['images'],
    queryFn: () => invoke<ImageItem[]>('list_images'),
    refetchInterval: 10_000,
  }));

  filteredImages = computed(() => {
    const all = this.images.data() ?? [];
    const q = this.filter().toLowerCase();
    return !q ? all : all.filter(img =>
      img.repo_tags.some(t => t.toLowerCase().includes(q)) || img.id.includes(q)
    );
  });

  private invalidate = () => this.queryClient.invalidateQueries({ queryKey: ['images'] });

  remove = injectMutation(() => ({
    mutationFn: (id: string) => invoke('remove_image', { id, force: false }),
    onSuccess: () => { this.invalidate(); this.confirmRemoveId.set(null); },
    onError: (e: unknown) => this.toast.error(errorMessage(e)),
  }));

  async pullImage(): Promise<void> {
    const name = this.pullName().trim();
    if (!name) return;
    this.pulling.set(true);
    this.pullProgress.set('Starting pull…');
    try {
      await invoke('pull_image', { name });
      this.pullName.set('');
      this.pullProgress.set('');
      this.invalidate();
      this.toast.success(`Pulled ${name}`);
    } catch (e) {
      this.toast.error(errorMessage(e));
    } finally {
      this.pulling.set(false);
    }
  }

  requestRemove(img: ImageItem): void { this.confirmRemoveId.set(img.id); }
  cancelRemove(): void { this.confirmRemoveId.set(null); }
  confirmRemove(): void {
    const id = this.confirmRemoveId();
    if (id) this.remove.mutate(id);
  }

  formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
    return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
  }

  formatCreated(ts: number): string {
    const diff = Date.now() / 1000 - ts;
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
    return `${Math.floor(diff / 86400)}d ago`;
  }

  shortId(id: string): string {
    return id.replace('sha256:', '').substring(0, 12);
  }

  displayTag(img: ImageItem): string {
    return img.repo_tags[0] ?? '<none>:<none>';
  }
}
