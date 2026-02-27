import { Component, EventEmitter, Input, Output } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';

export interface ConfirmOption {
  label: string;
  key: string;
}

@Component({
  selector: 'app-confirm-row',
  standalone: true,
  imports: [CommonModule, FormsModule],
  template: `
    <div class="confirm-inline" (keydown.escape)="onCancel()">
      <span>⚠</span>
      <span>{{ message }}</span>
      @for (opt of options; track opt.key) {
        <label>
          <input type="checkbox"
                 [(ngModel)]="optionValues[opt.key]"
                 (ngModelChange)="optionsChange.emit(getValues())">
          {{ opt.label }}
        </label>
      }
      <button class="btn btn-danger" (click)="onConfirm()">Confirm</button>
      <button class="btn btn-ghost" (click)="onCancel()">Cancel</button>
    </div>
  `
})
export class ConfirmRowComponent {
  @Input({ required: true }) message!: string;
  @Input() options: ConfirmOption[] = [];
  @Output() confirmed = new EventEmitter<Record<string, boolean>>();
  @Output() cancelled = new EventEmitter<void>();
  @Output() optionsChange = new EventEmitter<Record<string, boolean>>();

  optionValues: Record<string, boolean> = {};

  getValues(): Record<string, boolean> {
    return { ...this.optionValues };
  }

  onConfirm(): void {
    this.confirmed.emit(this.getValues());
  }

  onCancel(): void {
    this.cancelled.emit();
  }
}
