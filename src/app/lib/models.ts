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
