// Typed models matching the Rust serde output

export interface PortMapping {
  host_ip: string;
  host_port: string;
  container_port: string;
  protocol: string;
}

export interface ContainerItem {
  id: string;
  name: string;
  image: string;
  status: string;
  state: string;
  ports: PortMapping[];
  created: number;
  labels: Record<string, string>;
}

export interface ImageItem {
  id: string;
  repo_tags: string[];
  size: number;
  created: number;
  dangling: boolean;
}

export interface VolumeItem {
  name: string;
  driver: string;
  mount_point: string;
  created_at: string | null;
  labels: Record<string, string>;
  in_use: boolean;
}

export type StackStatus = 'all_running' | 'partial' | 'stopped' | 'unknown';

export interface ServiceItem {
  name: string;
  image: string;
  state: string;   // running | exited | ...
  status: string;  // human-readable status string
  ports: string[];
}

export interface StackItem {
  id: string;
  name: string;
  compose_file: string;
  missing: boolean;
  services: ServiceItem[];
  status: StackStatus;
}

export interface DockerInfo {
  server_version: string;
  api_version: string;
  socket_path: string;
  containers: number;
  containers_running: number;
  images: number;
  os: string;
  arch: string;
}

export type EngineHealth = 'ready' | 'needs_repair' | 'not_installed';

export interface EngineProviderStatus {
  id: 'wsl_engine' | 'host_engine' | 'custom_host' | string;
  label: string;
  active: boolean;
  health: EngineHealth;
  endpoint: string | null;
}

export interface EngineStatus {
  active_provider_id: string | null;
  providers: EngineProviderStatus[];
  resume_checkpoint: string | null;
  provisioning: ProvisioningState | null;
}

export type ProvisioningRunStatus = 'running' | 'succeeded' | 'failed';
export type ProvisioningStageStatus = 'pending' | 'in_progress' | 'completed' | 'failed';

export interface ProvisioningStage {
  id: string;
  label: string;
  status: ProvisioningStageStatus;
  failure_class: string | null;
  message: string | null;
}

export interface ProvisioningState {
  run_id: string;
  target_provider_id: string;
  status: ProvisioningRunStatus;
  stages: ProvisioningStage[];
  started_at: string;
  updated_at: string;
  finished_at: string | null;
}

export interface ConnectionGuidance {
  connected: boolean;
  title: string;
  message: string;
  failure_class: string | null;
  primary_action: string;
}

export interface PrivilegedActionSpec {
  id: string;
  description: string;
  requires_elevation: boolean;
}

export interface PrivilegedActionContract {
  version: string;
  transport: string;
  supported_actions: PrivilegedActionSpec[];
  execution_mode: string;
  helper_binary: string;
}

export interface LogLine {
  stream: 'stdout' | 'stderr';
  text: string;
}

export interface AppError {
  kind: string;
  /** Present for newtype variants (string) or struct variants (object); absent for unit variants like ComposeNotFound. */
  message?: string | { code: number; stderr: string };
}

export type PullProgressEvent = {
  id?: string;
  status?: string;
  progress?: string;
  progressDetail?: { current?: number; total?: number };
};
