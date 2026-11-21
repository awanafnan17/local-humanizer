import { useCallback, useEffect, useRef, useState } from "react";
import {
  cancelDownload as ipcCancelDownload,
  downloadModel as ipcDownloadModel,
  getAppStatus,
  listModels,
  listenModelDownloadProgress,
  retryRuntimeInit as ipcRetryRuntimeInit,
  selectModel as ipcSelectModel,
} from "../lib/ipcClient.ts";
import {
  applyDownloadProgressEvent,
  isBusyRuntimeState,
} from "../lib/humanizeState.ts";
import { sanitizeUserFacingError } from "../lib/validators.ts";
import type {
  AppStatus,
  ModelDescriptor,
  ModelDownloadProgressEvent,
} from "../lib/types.ts";

export interface UseAppStatusOptions {
  onNotify?: (kind: "info" | "success" | "error", message: string) => void;
}

export function useAppStatus(options: UseAppStatusOptions = {}) {
  const { onNotify } = options;
  const notifyRef = useRef(onNotify);
  notifyRef.current = onNotify;

  const [appStatus, setAppStatus] = useState<AppStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [models, setModels] = useState<ModelDescriptor[]>([]);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [downloadProgress, setDownloadProgress] = useState<
    Record<string, ModelDownloadProgressEvent>
  >({});
  const [isSwitchingModel, setIsSwitchingModel] = useState(false);
  const [isRetryingInit, setIsRetryingInit] = useState(false);
  const downloadTerminalHandledRef = useRef<
    Record<string, "completed" | "cancelled" | "error">
  >({});

  const refreshModels = useCallback(async () => {
    try {
      const list = await listModels();
      setModels(list);
      setModelsError(null);
    } catch (err) {
      const msg = sanitizeUserFacingError(err);
      setModelsError(msg);
      notifyRef.current?.("error", msg);
    }
  }, []);

  const refreshStatus = useCallback(async () => {
    try {
      const status = await getAppStatus();
      setAppStatus(status);
      setStatusError(null);
      return status;
    } catch (err) {
      const msg = sanitizeUserFacingError(err);
      setStatusError(msg);
      return null;
    }
  }, []);

  useEffect(() => {
    let mounted = true;
    void (async () => {
      await Promise.all([refreshStatus(), refreshModels()]);
    })();

    let unlisten: (() => void) | undefined;
    void listenModelDownloadProgress((evt) => {
      if (!mounted) return;
      setDownloadProgress((prev) => applyDownloadProgressEvent(prev, evt));
      if (evt.status === "completed") {
        downloadTerminalHandledRef.current[evt.model_id] = "completed";
        notifyRef.current?.("success", `Model ${evt.model_id} downloaded and verified.`);
        void refreshModels();
        void refreshStatus();
      } else if (evt.status === "cancelled") {
        downloadTerminalHandledRef.current[evt.model_id] = "cancelled";
        notifyRef.current?.("info", `Download cancelled for ${evt.model_id}.`);
        void refreshStatus();
      } else if (evt.status === "error") {
        downloadTerminalHandledRef.current[evt.model_id] = "error";
        notifyRef.current?.(
          "error",
          evt.error ? sanitizeUserFacingError(evt.error) : "Model download failed.",
        );
        void refreshStatus();
      } else {
        delete downloadTerminalHandledRef.current[evt.model_id];
      }
    }).then((fn) => {
      if (mounted) {
        unlisten = fn;
      } else {
        fn();
      }
    });

    return () => {
      mounted = false;
      unlisten?.();
    };
  }, [refreshModels, refreshStatus]);

  useEffect(() => {
    if (!isBusyRuntimeState(appStatus?.state)) {
      return undefined;
    }
    const timer = window.setInterval(() => {
      void refreshStatus();
    }, 600);
    return () => {
      window.clearInterval(timer);
    };
  }, [appStatus?.state, refreshStatus]);

  const retryRuntimeInit = useCallback(async () => {
    setIsRetryingInit(true);
    try {
      const nextStatus = await ipcRetryRuntimeInit();
      setAppStatus(nextStatus);
      setStatusError(null);
      await refreshModels();
      if (nextStatus.state === "error" || !nextStatus.model_ready) {
        notifyRef.current?.(
          "error",
          sanitizeUserFacingError(
            nextStatus.error_message ||
              "Local inference runtime failed to initialize.",
          ),
        );
      } else {
        notifyRef.current?.("success", "Local inference runtime is ready.");
      }
    } catch (err) {
      const msg = sanitizeUserFacingError(err);
      setStatusError(msg);
      notifyRef.current?.("error", msg);
    } finally {
      setIsRetryingInit(false);
    }
  }, [refreshModels]);

  const selectModel = useCallback(
    async (modelId: string, useGpu?: boolean | null) => {
      setIsSwitchingModel(true);
      try {
        const nextStatus = await ipcSelectModel(modelId, useGpu);
        setAppStatus(nextStatus);
        setStatusError(null);
        await refreshModels();
        if (nextStatus.state === "error" || !nextStatus.model_ready) {
          notifyRef.current?.(
            "error",
            sanitizeUserFacingError(
              nextStatus.error_message || "Failed to activate model.",
            ),
          );
        } else {
          notifyRef.current?.(
            "success",
            `Active model set to ${nextStatus.active_model_id} (${nextStatus.backend_mode.toUpperCase()}).`,
          );
        }
      } catch (err) {
        notifyRef.current?.("error", sanitizeUserFacingError(err));
      } finally {
        setIsSwitchingModel(false);
      }
    },
    [refreshModels],
  );

  const downloadModel = useCallback(
    async (modelId: string) => {
      delete downloadTerminalHandledRef.current[modelId];
      try {
        await ipcDownloadModel(modelId);
      } catch (err) {
        const terminal = downloadTerminalHandledRef.current[modelId];
        const errText = String(err ?? "");
        const isCancelledErr = /\bcancell?ed\b/i.test(errText);

        if (!terminal) {
          setDownloadProgress((prev) => {
            const existing = prev[modelId];
            if (!existing) return prev;
            return {
              ...prev,
              [modelId]: {
                ...existing,
                status: isCancelledErr ? "cancelled" : "error",
                error: isCancelledErr ? null : sanitizeUserFacingError(err),
              },
            };
          });
          void refreshStatus();
          void refreshModels();
        }

        if (isCancelledErr) {
          if (terminal !== "cancelled") {
            notifyRef.current?.("info", `Download cancelled for ${modelId}.`);
          }
        } else if (terminal !== "error") {
          notifyRef.current?.("error", sanitizeUserFacingError(err));
        }
      }
    },
    [refreshModels, refreshStatus],
  );

  const cancelDownload = useCallback(async (modelId: string) => {
    try {
      await ipcCancelDownload(modelId);
    } catch (err) {
      notifyRef.current?.("error", sanitizeUserFacingError(err));
    }
  }, []);

  return {
    appStatus,
    statusError,
    models,
    modelsError,
    downloadProgress,
    isSwitchingModel,
    isRetryingInit,
    refreshStatus,
    refreshModels,
    retryRuntimeInit,
    selectModel,
    downloadModel,
    cancelDownload,
  };
}
