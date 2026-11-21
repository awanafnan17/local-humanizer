import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import type {
  AppStatus,
  HumanizeProgressEvent,
  HumanizeRequest,
  HumanizeResponse,
  ModelDescriptor,
  ModelDownloadProgressEvent,
  RetryChunkRequest,
  RetryChunkResponse,
} from "./types.ts";

export function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && isTauri();
}

async function callIpc<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (isTauriRuntime()) {
    return invoke<T>(cmd, args);
  }
  const { getBrowserMockRuntime } = await import("./mockTauriIpc.ts");
  return getBrowserMockRuntime().invoke(cmd, args) as Promise<T>;
}

export async function getAppStatus(): Promise<AppStatus> {
  return callIpc<AppStatus>("get_app_status");
}

export async function listModels(): Promise<ModelDescriptor[]> {
  return callIpc<ModelDescriptor[]>("list_models");
}

export async function selectModel(
  modelId: string,
  useGpu?: boolean | null,
): Promise<AppStatus> {
  return callIpc<AppStatus>("select_model", {
    modelId,
    useGpu: useGpu ?? null,
  });
}

export async function downloadModel(modelId: string): Promise<void> {
  return callIpc<void>("download_model", { modelId });
}

export async function cancelDownload(modelId: string): Promise<void> {
  return callIpc<void>("cancel_download", { modelId });
}

export async function humanizeDocument(
  req: HumanizeRequest,
): Promise<HumanizeResponse> {
  return callIpc<HumanizeResponse>("humanize_document", { req });
}

export async function cancelHumanize(requestId: string): Promise<void> {
  return callIpc<void>("cancel_humanize", { requestId });
}

export async function retryChunk(
  req: RetryChunkRequest,
): Promise<RetryChunkResponse> {
  return callIpc<RetryChunkResponse>("retry_chunk", { req });
}

export async function importTextFile(path: string): Promise<string> {
  return callIpc<string>("import_text_file", { path });
}

export async function saveOutputFile(
  path: string,
  content: string,
): Promise<void> {
  return callIpc<void>("save_output_file", { path, content });
}

export async function retryRuntimeInit(): Promise<AppStatus> {
  return callIpc<AppStatus>("retry_runtime_init");
}

export async function listenModelDownloadProgress(
  handler: (payload: ModelDownloadProgressEvent) => void,
): Promise<() => void> {
  if (isTauriRuntime()) {
    return listen<ModelDownloadProgressEvent>(
      "model-download-progress",
      (event) => handler(event.payload),
    );
  }
  const { getBrowserMockRuntime } = await import("./mockTauriIpc.ts");
  return getBrowserMockRuntime().listen<ModelDownloadProgressEvent>(
    "model-download-progress",
    handler,
  );
}

export async function listenHumanizeProgress(
  handler: (payload: HumanizeProgressEvent) => void,
): Promise<() => void> {
  if (isTauriRuntime()) {
    return listen<HumanizeProgressEvent>("humanize-progress", (event) =>
      handler(event.payload),
    );
  }
  const { getBrowserMockRuntime } = await import("./mockTauriIpc.ts");
  return getBrowserMockRuntime().listen<HumanizeProgressEvent>(
    "humanize-progress",
    handler,
  );
}

export async function pickOpenTextFile(): Promise<string | null> {
  if (isTauriRuntime()) {
    const selected = await open({
      title: "Import Text or Markdown",
      multiple: false,
      directory: false,
      filters: [{ name: "Text & Markdown", extensions: ["txt", "md", "markdown"] }],
    });
    return typeof selected === "string" ? selected : null;
  }
  if (
    typeof window !== "undefined" &&
    window.__LOCAL_HUMANIZER_MOCK_IPC__?.openDialog
  ) {
    return window.__LOCAL_HUMANIZER_MOCK_IPC__.openDialog();
  }
  return null;
}

export async function pickSaveOutputFile(
  defaultPath = "humanized-output.txt",
): Promise<string | null> {
  if (isTauriRuntime()) {
    const selected = await save({
      title: "Save Humanized Text",
      defaultPath,
      filters: [
        { name: "Text Document", extensions: ["txt"] },
        { name: "Markdown", extensions: ["md"] },
      ],
    });
    return typeof selected === "string" ? selected : null;
  }
  if (
    typeof window !== "undefined" &&
    window.__LOCAL_HUMANIZER_MOCK_IPC__?.saveDialog
  ) {
    return window.__LOCAL_HUMANIZER_MOCK_IPC__.saveDialog(defaultPath);
  }
  return null;
}
