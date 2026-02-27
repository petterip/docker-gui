import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import { AppError } from './models';

/**
 * Typed wrapper around Tauri invoke.
 * On error, the Rust AppError is re-thrown as-is so callers can
 * pattern-match on `err.kind`.
 */
export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return tauriInvoke<T>(cmd, args);
}

export function isAppError(e: unknown): e is AppError {
  return typeof e === 'object' && e !== null && 'kind' in e;
}

export function errorMessage(e: unknown): string {
  if (isAppError(e)) {
    const msg = e.message;
    if (msg === undefined) return e.kind;
    if (typeof msg === 'string') return msg;
    // ComposeError struct variant: { code, stderr }
    if (typeof msg === 'object' && 'stderr' in msg) return (msg as { stderr: string }).stderr || e.kind;
    return e.kind;
  }
  if (e instanceof Error) return e.message;
  return String(e);
}
