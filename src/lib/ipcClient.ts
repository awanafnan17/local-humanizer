import { invoke } from "@tauri-apps/api/core";
import type {
  AppStatus,
  HumanizeRequest,
  HumanizeResponse,
  ModelDescriptor,
  RetryChunkRequest,
  RetryChunkResponse,
} from "./types";

export function isTauriRuntime(): boolean {
  return (
    typeof window !== "undefined" &&
    "__TAURI_INTERNALS__" in (window as unknown as Record<string, unknown>)
  );
}

export async function getAppStatus(): Promise<AppStatus> {
  return invoke<AppStatus>("get_app_status");
}

export async function listModels(): Promise<ModelDescriptor[]> {
  return invoke<ModelDescriptor[]>("list_models");
}

export async function selectModel(
  modelId: string,
  useGpu?: boolean | null,
): Promise<AppStatus> {
  return invoke<AppStatus>("select_model", {
    modelId,
    useGpu: useGpu ?? null,
  });
}

export async function downloadModel(modelId: string): Promise<void> {
  return invoke<void>("download_model", { modelId });
}

export async function cancelDownload(modelId: string): Promise<void> {
  return invoke<void>("cancel_download", { modelId });
}

export async function humanizeDocument(
  req: HumanizeRequest,
): Promise<HumanizeResponse> {
  return invoke<HumanizeResponse>("humanize_document", { req });
}

export async function cancelHumanize(requestId: string): Promise<void> {
  return invoke<void>("cancel_humanize", { requestId });
}

export async function retryChunk(
  req: RetryChunkRequest,
): Promise<RetryChunkResponse> {
  return invoke<RetryChunkResponse>("retry_chunk", { req });
}

export async function importTextFile(path: string): Promise<string> {
  return invoke<string>("import_text_file", { path });
}

export async function saveOutputFile(
  path: string,
  content: string,
): Promise<void> {
  return invoke<void>("save_output_file", { path, content });
}

export async function retryRuntimeInit(): Promise<AppStatus> {
  return invoke<AppStatus>("retry_runtime_init");
}
