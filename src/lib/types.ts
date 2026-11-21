export type RuntimeState =
  | "initializing"
  | "verifying"
  | "downloading"
  | "starting_runtime"
  | "ready"
  | "error";

export type BackendMode = "metal" | "cpu";

export interface AppStatus {
  state: RuntimeState;
  active_model_id: string;
  model_ready: boolean;
  backend_mode: BackendMode;
  ram_bytes: number;
  error_message: string | null;
  runtime_version: string;
}

export interface ModelDescriptor {
  id: string;
  name: string;
  quant: string;
  size_bytes: number;
  sha256: string;
  license: string;
  license_url: string;
  is_default: boolean;
  is_installed: boolean;
  is_bundled: boolean;
  min_recommended_ram_gb: number;
  non_commercial_warning: boolean;
  download_url: string;
}

export type DownloadStatus =
  | "downloading"
  | "verifying"
  | "completed"
  | "cancelled"
  | "error";

export interface ModelDownloadProgressEvent {
  model_id: string;
  downloaded_bytes: number;
  total_bytes: number;
  status: DownloadStatus;
  error: string | null;
}

export type RewriteMode =
  | "standard"
  | "natural"
  | "academic"
  | "professional"
  | "simple"
  | "deep";

export type Intensity = "light" | "balanced" | "strong";

export type VoiceOption = "auto" | "first_person" | "third_person";

export interface RewriteOptions {
  preserve_length: boolean;
  preserve_formatting: boolean;
  preserve_citations: boolean;
  preserve_quotes: boolean;
  language: string;
  voice: VoiceOption;
}

export interface HumanizeRequest {
  request_id: string;
  text: string;
  mode: RewriteMode;
  intensity: Intensity;
  options: RewriteOptions;
}

export interface HumanizeResponse {
  request_id: string;
  output_text: string;
  chunks_total: number;
  chunks_completed: number;
  chunks_retried: number;
  warnings: string[];
  duration_ms: number;
  cancelled: boolean;
}

export type HumanizeProgressState =
  | "segmenting"
  | "inferring"
  | "validating"
  | "retrying"
  | "completed"
  | "cancelled"
  | "error";

export interface HumanizeProgressEvent {
  request_id: string;
  chunk_index: number;
  chunks_total: number;
  state: HumanizeProgressState;
  partial_output: string;
  chunk_warnings: string[];
}

export interface RetryChunkRequest {
  request_id: string;
  chunk_index: number;
  original_chunk: string;
  context_before: string;
  mode: RewriteMode;
  intensity: Intensity;
  options: RewriteOptions;
}

export interface RetryChunkResponse {
  request_id: string;
  chunk_index: number;
  rewritten_chunk: string;
  warnings: string[];
}
