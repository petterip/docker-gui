#!/usr/bin/env node
import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, chmodSync, existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const tauriDir = path.join(repoRoot, 'src-tauri');

const cargoCmd = process.platform === 'win32' ? 'cargo.exe' : 'cargo';
const cargoArgs = ['build', '--release', '--bin', 'docker-gui-provisioning-helper'];
const cargoResult = spawnSync(cargoCmd, cargoArgs, {
  cwd: tauriDir,
  stdio: 'inherit',
});

if (cargoResult.status !== 0) {
  process.exit(cargoResult.status ?? 1);
}

const helperName =
  process.platform === 'win32'
    ? 'docker-gui-provisioning-helper.exe'
    : 'docker-gui-provisioning-helper';
const builtPath = path.join(tauriDir, 'target', 'release', helperName);
if (!existsSync(builtPath)) {
  console.error(`Built helper not found at ${builtPath}`);
  process.exit(1);
}

const platformDir =
  process.platform === 'win32'
    ? 'win32'
    : process.platform === 'darwin'
    ? 'darwin'
    : 'linux';
const destDir = path.join(tauriDir, 'bin', platformDir);
mkdirSync(destDir, { recursive: true });

const destPath = path.join(destDir, helperName);
copyFileSync(builtPath, destPath);
try {
  chmodSync(destPath, 0o755);
} catch (_) {
  // Ignore chmod errors on platforms that do not support it.
}

console.log(`Helper binary copied to ${destPath}`);
