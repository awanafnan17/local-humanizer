import type {
  AppStatus,
  HumanizeProgressEvent,
  HumanizeRequest,
  HumanizeResponse,
  Intensity,
  ModelDescriptor,
  ModelDownloadProgressEvent,
  RetryChunkRequest,
  RetryChunkResponse,
  RewriteMode,
  RewriteOptions,
} from "./types.ts";
import { segmentParagraphChunks } from "./wordCount.ts";

export interface MockIpcHistoryEntry {
  cmd: string;
  args?: Record<string, unknown>;
}

export interface MockScenarioConfig {
  initialStatus?: Partial<AppStatus>;
  retryInitStatus?: Partial<AppStatus>;
  chunkDelayMs?: number;
  downloadDelayMs?: number;
  flagChunkIndex?: number | null;
  virtualFiles?: Record<string, string>;
}

export interface LocalHumanizerMockBridge {
  invoke?: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> | unknown;
  emit?: <T = unknown>(event: string, payload: T) => void;
  openDialog?: () => Promise<string | null> | string | null;
  saveDialog?: (defaultPath?: string) => Promise<string | null> | string | null;
  history?: MockIpcHistoryEntry[];
  savedFiles?: Record<string, string>;
  scenario?: MockScenarioConfig;
}

declare global {
  interface Window {
    __LOCAL_HUMANIZER_MOCK_IPC__?: LocalHumanizerMockBridge;
  }
}

function defaultMockModels(): ModelDescriptor[] {
  return [
    {
      id: "qwen3-1.7b-q4_k_m",
      name: "Qwen3 1.7B (Recommended Default)",
      quant: "Q4_K_M",
      size_bytes: 1107409472,
      sha256: "b139949c5bd74937ad8ed8c8cf3d9ffb1e99c866c823204dc42c0d91fa181897",
      license: "Apache-2.0",
      license_url: "",
      is_default: true,
      is_installed: true,
      is_bundled: true,
      min_recommended_ram_gb: 8,
      non_commercial_warning: false,
      download_url: "",
    },
    {
      id: "qwen3-4b-instruct-2507-q4_k_m",
      name: "Qwen3 4B Instruct 2507 (High-RAM Quality Tier)",
      quant: "Q4_K_M",
      size_bytes: 2497281120,
      sha256: "3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597",
      license: "Apache-2.0",
      license_url: "",
      is_default: false,
      is_installed: false,
      is_bundled: false,
      min_recommended_ram_gb: 16,
      non_commercial_warning: false,
      download_url: "",
    },
    {
      id: "qwen2.5-1.5b-instruct-q4_k_m",
      name: "Qwen2.5 1.5B Instruct (Fast Apache-2.0 Alternative)",
      quant: "Q4_K_M",
      size_bytes: 1117320736,
      sha256: "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e",
      license: "Apache-2.0",
      license_url: "",
      is_default: false,
      is_installed: false,
      is_bundled: false,
      min_recommended_ram_gb: 8,
      non_commercial_warning: false,
      download_url: "",
    },
    {
      id: "qwen2.5-3b-instruct-q4_k_m",
      name: "Qwen2.5 3B Instruct (Non-Commercial Research Only)",
      quant: "Q4_K_M",
      size_bytes: 2104932768,
      sha256: "626b4a6678b86442240e33df819e00132d3ba7dddfe1cdc4fbb18e0a9615c62d",
      license: "Qwen Research License Agreement",
      license_url: "",
      is_default: false,
      is_installed: false,
      is_bundled: false,
      min_recommended_ram_gb: 16,
      non_commercial_warning: true,
      download_url: "",
    },
  ];
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function rewritePhraseDeterministic(
  chunk: string,
  mode: RewriteMode,
  intensity: Intensity,
  options: RewriteOptions,
): string {
  const trimmed = chunk.trim();
  if (!trimmed) return "";

  const replacements: Array<[RegExp, string]> =
    mode === "academic"
      ? [
          [/\bvery\s+good\b/gi, "notably effective"],
          [/\bshows\b/gi, "demonstrates"],
          [/\bhelps\b/gi, "facilitates"],
        ]
      : mode === "natural"
        ? [
            [/\butilize\b/gi, "use"],
            [/\bdemonstrates\b/gi, "shows"],
            [/\bfurthermore\b/gi, "also"],
          ]
        : mode === "professional"
          ? [
              [/\ba lot of\b/gi, "substantial"],
              [/\bfix\b/gi, "resolve"],
              [/\bmake sure\b/gi, "ensure"],
            ]
          : mode === "simple"
            ? [
                [/\butilize\b/gi, "use"],
                [/\bconsequently\b/gi, "so"],
                [/\bapproximately\b/gi, "about"],
              ]
            : mode === "deep"
              ? [
                  [/\bhowever\b/gi, "that said"],
                  [/\bimportant\b/gi, "essential"],
                  [/\bbecause\b/gi, "given that"],
                ]
              : [
                  [/\butilize\b/gi, "use"],
                  [/\bin order to\b/gi, "to"],
                  [/\badditionally\b/gi, "in addition"],
                ];

  let out = trimmed;
  for (const [pattern, target] of replacements) {
    out = out.replace(pattern, target);
  }

  if (intensity === "strong" && out === trimmed) {
    const sentences = out.split(/(?<=[.!?])\s+/);
    if (sentences.length > 1) {
      out = sentences
        .map((s) => s.replace(/^Additionally,\s+/i, "Moreover, "))
        .join(" ");
    }
  }

  if (!options.preserve_formatting) {
    out = out.replace(/\s+/g, " ").trim();
  }

  return out;
}

export interface MockTauriRuntime {
  invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
  listen: <T = unknown>(
    event: string,
    handler: (payload: T) => void,
  ) => () => void;
  emit: <T = unknown>(event: string, payload: T) => void;
  getHistory: () => MockIpcHistoryEntry[];
  getSavedFiles: () => Record<string, string>;
}

export function createMockTauriRuntime(
  initialScenario?: MockScenarioConfig,
): MockTauriRuntime {
  const models = defaultMockModels();
  const listeners = new Map<string, Set<(payload: unknown) => void>>();
  const history: MockIpcHistoryEntry[] = [];
  const savedFiles: Record<string, string> = {};
  const virtualFiles: Record<string, string> = {
    ...(initialScenario?.virtualFiles ?? {}),
  };
  const cancelledHumanizeIds = new Set<string>();
  const cancelledDownloads = new Set<string>();

  let appStatus: AppStatus = {
    state: "ready",
    active_model_id: "qwen3-1.7b-q4_k_m",
    model_ready: true,
    backend_mode: "cpu",
    ram_bytes: 8 * 1024 * 1024 * 1024,
    error_message: null,
    runtime_version: "b12600",
    ...(initialScenario?.initialStatus ?? {}),
  };

  const emit = <T = unknown>(event: string, payload: T): void => {
    const set = listeners.get(event);
    if (!set) return;
    for (const cb of set) {
      cb(payload);
    }
  };

  const listen = <T = unknown>(
    event: string,
    handler: (payload: T) => void,
  ): (() => void) => {
    let set = listeners.get(event);
    if (!set) {
      set = new Set();
      listeners.set(event, set);
    }
    const wrapped = (payload: unknown) => handler(payload as T);
    set.add(wrapped);
    return () => {
      set?.delete(wrapped);
    };
  };

  const getScenario = (): MockScenarioConfig => {
    if (typeof window !== "undefined" && window.__LOCAL_HUMANIZER_MOCK_IPC__?.scenario) {
      return {
        ...initialScenario,
        ...window.__LOCAL_HUMANIZER_MOCK_IPC__.scenario,
      };
    }
    return initialScenario ?? {};
  };

  const invoke = async (
    cmd: string,
    args?: Record<string, unknown>,
  ): Promise<unknown> => {
    history.push({ cmd, args });

    if (typeof window !== "undefined" && window.__LOCAL_HUMANIZER_MOCK_IPC__) {
      const bridge = window.__LOCAL_HUMANIZER_MOCK_IPC__;
      if (bridge.history) {
        bridge.history.push({ cmd, args });
      }
      if (bridge.invoke) {
        const custom = await bridge.invoke(cmd, args);
        if (custom !== undefined) {
          return custom;
        }
      }
    }

    const scenario = getScenario();

    switch (cmd) {
      case "get_app_status":
        return { ...appStatus };

      case "retry_runtime_init":
        appStatus = {
          ...appStatus,
          state: "ready",
          model_ready: true,
          error_message: null,
          ...(scenario.retryInitStatus ?? {}),
        };
        return { ...appStatus };

      case "list_models":
        return models.map((m) => ({ ...m }));

      case "select_model": {
        const modelId = String(args?.modelId ?? "");
        const useGpu = args?.useGpu as boolean | null | undefined;
        const found = models.find((m) => m.id === modelId);
        if (!found) {
          throw new Error(`Unknown model id: ${modelId}`);
        }
        if (!found.is_installed) {
          throw new Error(`Model ${found.name} must be downloaded before selection.`);
        }
        appStatus = {
          ...appStatus,
          active_model_id: found.id,
          backend_mode: useGpu === true ? "metal" : useGpu === false ? "cpu" : appStatus.backend_mode,
          state: "ready",
          model_ready: true,
          error_message: null,
        };
        return { ...appStatus };
      }

      case "download_model": {
        const modelId = String(args?.modelId ?? "");
        const target = models.find((m) => m.id === modelId);
        if (!target) {
          throw new Error(`Unknown model id: ${modelId}`);
        }
        cancelledDownloads.delete(modelId);
        const stepDelay = scenario.downloadDelayMs ?? 35;
        const total = target.size_bytes;
        const steps = [0.25, 0.55, 0.85, 1];
        for (const ratio of steps) {
          await sleep(stepDelay);
          if (cancelledDownloads.has(modelId)) {
            emit<ModelDownloadProgressEvent>("model-download-progress", {
              model_id: modelId,
              downloaded_bytes: Math.round(total * ratio),
              total_bytes: total,
              status: "cancelled",
              error: null,
            });
            return undefined;
          }
          emit<ModelDownloadProgressEvent>("model-download-progress", {
            model_id: modelId,
            downloaded_bytes: Math.round(total * ratio),
            total_bytes: total,
            status: "downloading",
            error: null,
          });
        }
        emit<ModelDownloadProgressEvent>("model-download-progress", {
          model_id: modelId,
          downloaded_bytes: total,
          total_bytes: total,
          status: "verifying",
          error: null,
        });
        await sleep(stepDelay);
        if (cancelledDownloads.has(modelId)) {
          emit<ModelDownloadProgressEvent>("model-download-progress", {
            model_id: modelId,
            downloaded_bytes: total,
            total_bytes: total,
            status: "cancelled",
            error: null,
          });
          return undefined;
        }
        target.is_installed = true;
        emit<ModelDownloadProgressEvent>("model-download-progress", {
          model_id: modelId,
          downloaded_bytes: total,
          total_bytes: total,
          status: "completed",
          error: null,
        });
        return undefined;
      }

      case "cancel_download": {
        const modelId = String(args?.modelId ?? "");
        cancelledDownloads.add(modelId);
        return undefined;
      }

      case "humanize_document": {
        const req = args?.req as HumanizeRequest;
        if (!req || !req.request_id) {
          throw new Error("Invalid HumanizeRequest payload.");
        }
        const started = Date.now();
        cancelledHumanizeIds.delete(req.request_id);
        const chunks = segmentParagraphChunks(req.text);
        const total = Math.max(1, chunks.length);
        const effectiveChunks = chunks.length > 0 ? chunks : [req.text];
        const chunkDelay = scenario.chunkDelayMs ?? 15;
        const flagIndex =
          typeof scenario.flagChunkIndex === "number"
            ? scenario.flagChunkIndex
            : null;

        emit<HumanizeProgressEvent>("humanize-progress", {
          request_id: req.request_id,
          chunk_index: 0,
          chunks_total: total,
          state: "segmenting",
          partial_output: "",
          chunk_warnings: [],
        });

        const rewrittenParts: string[] = [];
        const warnings: string[] = [];

        for (let i = 0; i < effectiveChunks.length; i += 1) {
          if (chunkDelay > 0) {
            await sleep(chunkDelay);
          }
          if (cancelledHumanizeIds.has(req.request_id)) {
            const partial = rewrittenParts.join("\n\n");
            emit<HumanizeProgressEvent>("humanize-progress", {
              request_id: req.request_id,
              chunk_index: i,
              chunks_total: total,
              state: "cancelled",
              partial_output: partial,
              chunk_warnings: [],
            });
            const cancelledResp: HumanizeResponse = {
              request_id: req.request_id,
              output_text: partial,
              chunks_total: total,
              chunks_completed: rewrittenParts.length,
              chunks_retried: 0,
              warnings,
              duration_ms: Date.now() - started,
              cancelled: true,
            };
            return cancelledResp;
          }

          const rewritten = rewritePhraseDeterministic(
            effectiveChunks[i],
            req.mode,
            req.intensity,
            req.options,
          );
          rewrittenParts.push(rewritten);

          const chunkWarnings =
            flagIndex === i
              ? [`Chunk ${i + 1} closely echoed source phrasing; retry available.`]
              : [];
          if (chunkWarnings.length > 0) {
            warnings.push(...chunkWarnings);
          }

          emit<HumanizeProgressEvent>("humanize-progress", {
            request_id: req.request_id,
            chunk_index: i,
            chunks_total: total,
            state: "inferring",
            partial_output: rewrittenParts.join("\n\n"),
            chunk_warnings: chunkWarnings,
          });
        }

        emit<HumanizeProgressEvent>("humanize-progress", {
          request_id: req.request_id,
          chunk_index: total - 1,
          chunks_total: total,
          state: "validating",
          partial_output: rewrittenParts.join("\n\n"),
          chunk_warnings: [],
        });

        const finalOutput = rewrittenParts.join("\n\n");
        emit<HumanizeProgressEvent>("humanize-progress", {
          request_id: req.request_id,
          chunk_index: total - 1,
          chunks_total: total,
          state: "completed",
          partial_output: finalOutput,
          chunk_warnings: [],
        });

        const resp: HumanizeResponse = {
          request_id: req.request_id,
          output_text: finalOutput,
          chunks_total: total,
          chunks_completed: total,
          chunks_retried: 0,
          warnings,
          duration_ms: Math.max(1, Date.now() - started),
          cancelled: false,
        };
        return resp;
      }

      case "cancel_humanize": {
        const requestId = String(args?.requestId ?? "");
        if (requestId) {
          cancelledHumanizeIds.add(requestId);
        }
        return undefined;
      }

      case "retry_chunk": {
        const req = args?.req as RetryChunkRequest;
        if (!req || !req.request_id) {
          throw new Error("Invalid RetryChunkRequest payload.");
        }
        const rewritten = rewritePhraseDeterministic(
          req.original_chunk,
          req.mode,
          req.intensity,
          req.options,
        );
        const resp: RetryChunkResponse = {
          request_id: req.request_id,
          chunk_index: req.chunk_index,
          rewritten_chunk: rewritten,
          warnings: [],
        };
        return resp;
      }

      case "import_text_file": {
        const path = String(args?.path ?? "");
        if (path in virtualFiles) {
          return virtualFiles[path];
        }
        if (
          typeof window !== "undefined" &&
          window.__LOCAL_HUMANIZER_MOCK_IPC__?.scenario?.virtualFiles &&
          path in window.__LOCAL_HUMANIZER_MOCK_IPC__.scenario.virtualFiles
        ) {
          return window.__LOCAL_HUMANIZER_MOCK_IPC__.scenario.virtualFiles[path];
        }
        throw new Error(`Mock file not found: ${path}`);
      }

      case "save_output_file": {
        const path = String(args?.path ?? "");
        const content = String(args?.content ?? "");
        savedFiles[path] = content;
        if (typeof window !== "undefined" && window.__LOCAL_HUMANIZER_MOCK_IPC__) {
          if (!window.__LOCAL_HUMANIZER_MOCK_IPC__.savedFiles) {
            window.__LOCAL_HUMANIZER_MOCK_IPC__.savedFiles = {};
          }
          window.__LOCAL_HUMANIZER_MOCK_IPC__.savedFiles[path] = content;
        }
        return undefined;
      }

      default:
        throw new Error(`Unhandled mock command: ${cmd}`);
    }
  };

  return {
    invoke,
    listen,
    emit,
    getHistory: () => [...history],
    getSavedFiles: () => ({ ...savedFiles }),
  };
}

let singletonRuntime: MockTauriRuntime | null = null;

export function getBrowserMockRuntime(): MockTauriRuntime {
  if (!singletonRuntime) {
    const initialScenario =
      typeof window !== "undefined"
        ? window.__LOCAL_HUMANIZER_MOCK_IPC__?.scenario
        : undefined;
    singletonRuntime = createMockTauriRuntime(initialScenario);
    if (typeof window !== "undefined") {
      const existing = window.__LOCAL_HUMANIZER_MOCK_IPC__ ?? {};
      window.__LOCAL_HUMANIZER_MOCK_IPC__ = {
        ...existing,
        emit: <T = unknown>(event: string, payload: T) =>
          singletonRuntime?.emit(event, payload),
        history: existing.history ?? [],
        savedFiles: existing.savedFiles ?? {},
      };
    }
  }
  return singletonRuntime;
}

export function resetBrowserMockRuntimeForTests(): void {
  singletonRuntime = null;
}
