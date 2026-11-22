import { useCallback, useEffect, useRef, useState } from "react";
import { DEFAULT_REWRITE_OPTIONS } from "../lib/constants.ts";
import {
  alignOutputChunks,
  applyHumanizeProgressEvent,
  createInitialHumanizeRunState,
  extractChunkWarningIndices,
  spliceRetriedChunkOutput,
  warningBelongsToChunk,
  type HumanizeRunState,
} from "../lib/humanizeState.ts";
import {
  cancelHumanize as ipcCancelHumanize,
  humanizeDocument as ipcHumanizeDocument,
  listenHumanizeProgress,
  retryChunk as ipcRetryChunk,
} from "../lib/ipcClient.ts";
import { sanitizeUserFacingError } from "../lib/validators.ts";
import { segmentParagraphChunks } from "../lib/wordCount.ts";
import type {
  Intensity,
  RewriteMode,
  RewriteOptions,
} from "../lib/types.ts";

export interface UseHumanizerOptions {
  onNotify?: (kind: "info" | "success" | "error", message: string) => void;
}

function createRequestId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return `req-${crypto.randomUUID()}`;
  }
  return `req-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

export function useHumanizer(hookOptions: UseHumanizerOptions = {}) {
  const notifyRef = useRef(hookOptions.onNotify);
  notifyRef.current = hookOptions.onNotify;

  const [inputText, setInputText] = useState("");
  const [outputText, setOutputText] = useState("");
  const [mode, setMode] = useState<RewriteMode>("standard");
  const [intensity, setIntensity] = useState<Intensity>("balanced");
  const [options, setOptions] = useState<RewriteOptions>(DEFAULT_REWRITE_OPTIONS);
  const [runState, setRunState] = useState<HumanizeRunState>(() =>
    createInitialHumanizeRunState(),
  );
  const [elapsedMs, setElapsedMs] = useState(0);
  const [isRetryingChunk, setIsRetryingChunk] = useState(false);

  const inFlightRef = useRef<boolean>(false);
  const activeRequestIdRef = useRef<string | null>(null);
  const startTimestampRef = useRef<number | null>(null);
  const originalChunksRef = useRef<string[]>([]);
  const outputChunksRef = useRef<string[]>([]);
  const chunksTotalRef = useRef<number>(0);
  const outputTextRef = useRef<string>(outputText);
  outputTextRef.current = outputText;
  const lastRequestConfigRef = useRef<{
    requestId: string;
    mode: RewriteMode;
    intensity: Intensity;
    options: RewriteOptions;
  } | null>(null);

  useEffect(() => {
    let mounted = true;
    let unlisten: (() => void) | undefined;
    void listenHumanizeProgress((evt) => {
      if (!mounted) return;
      setRunState((prev) => applyHumanizeProgressEvent(prev, evt));
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
  }, []);

  useEffect(() => {
    if (!runState.isRunning) {
      return undefined;
    }
    const timer = window.setInterval(() => {
      if (startTimestampRef.current !== null) {
        setElapsedMs(Date.now() - startTimestampRef.current);
      }
    }, 100);
    return () => {
      window.clearInterval(timer);
    };
  }, [runState.isRunning]);

  const startHumanize = useCallback(async () => {
    if (inFlightRef.current) {
      return;
    }
    const trimmed = inputText.trim();
    if (!trimmed) {
      return;
    }

    const requestId = createRequestId();
    inFlightRef.current = true;
    activeRequestIdRef.current = requestId;

    const segmented = segmentParagraphChunks(inputText);
    originalChunksRef.current = segmented.length > 0 ? segmented : [inputText];
    outputChunksRef.current = [];
    chunksTotalRef.current = originalChunksRef.current.length;
    lastRequestConfigRef.current = {
      requestId,
      mode,
      intensity,
      options: { ...options },
    };

    startTimestampRef.current = Date.now();
    setElapsedMs(0);
    setOutputText("");
    setRunState({
      ...createInitialHumanizeRunState(),
      requestId,
      isRunning: true,
      state: "segmenting",
      chunksTotal: originalChunksRef.current.length,
    });

    try {
      const resp = await ipcHumanizeDocument({
        request_id: requestId,
        text: inputText,
        mode,
        intensity,
        options,
      });

      if (activeRequestIdRef.current !== requestId) {
        return;
      }

      const finalElapsed =
        resp.duration_ms > 0
          ? resp.duration_ms
          : startTimestampRef.current !== null
            ? Date.now() - startTimestampRef.current
            : 0;
      setElapsedMs(finalElapsed);

      if (resp.failed) {
        setOutputText("");
        outputChunksRef.current = [];
        const errMsg = sanitizeUserFacingError(
          resp.error_message ||
            "Paraphrasing failed to produce a sufficiently rewritten, accurate result. Your original text has been preserved.",
        );
        setRunState((prev) => {
          if (prev.requestId !== requestId) return prev;
          return {
            ...prev,
            isRunning: false,
            state: "error",
            chunksTotal: resp.chunks_total || prev.chunksTotal,
            chunkIndex: Math.max(0, resp.chunks_completed),
            warnings: resp.warnings ?? [],
            flaggedChunks: [],
            durationMs: finalElapsed,
            cancelled: false,
            errorMessage: errMsg,
          };
        });
        notifyRef.current?.("error", errMsg);
        return;
      }

      const expectedChunks = Math.max(
        1,
        resp.chunks_total || originalChunksRef.current.length,
      );
      chunksTotalRef.current = expectedChunks;

      if (resp.output_text) {
        setOutputText(resp.output_text);
        outputChunksRef.current = alignOutputChunks(
          resp.output_text,
          expectedChunks,
        );
      } else {
        setOutputText("");
        outputChunksRef.current = [];
      }

      setRunState((prev) => {
        if (prev.requestId !== requestId) return prev;
        const flagged = extractChunkWarningIndices(
          resp.warnings,
          prev.chunkWarningsMap,
          resp.chunks_total || originalChunksRef.current.length,
        );
        return {
          ...prev,
          isRunning: false,
          state: resp.cancelled ? "cancelled" : "completed",
          chunksTotal: resp.chunks_total || prev.chunksTotal,
          chunkIndex: Math.max(0, (resp.chunks_completed || 1) - 1),
          warnings: resp.warnings,
          flaggedChunks: flagged,
          durationMs: finalElapsed,
          cancelled: resp.cancelled,
          errorMessage: null,
        };
      });
    } catch (err) {
      if (activeRequestIdRef.current !== requestId) {
        return;
      }
      const msg = sanitizeUserFacingError(err);
      setRunState((prev) => {
        if (prev.requestId !== requestId) return prev;
        return {
          ...prev,
          isRunning: false,
          state: "error",
          errorMessage: msg,
        };
      });
      notifyRef.current?.("error", msg);
    } finally {
      if (activeRequestIdRef.current === requestId) {
        inFlightRef.current = false;
        activeRequestIdRef.current = null;
      }
    }
  }, [inputText, intensity, mode, options]);

  const cancelActiveHumanize = useCallback(async () => {
    const requestIdToCancel = activeRequestIdRef.current;
    if (!requestIdToCancel) {
      return;
    }
    inFlightRef.current = false;
    activeRequestIdRef.current = null;
    try {
      await ipcCancelHumanize(requestIdToCancel);
      setRunState((prev) => {
        if (prev.requestId !== requestIdToCancel) return prev;
        return {
          ...prev,
          isRunning: false,
          state: "cancelled",
          cancelled: true,
        };
      });
    } catch (err) {
      notifyRef.current?.("error", sanitizeUserFacingError(err));
    }
  }, []);

  const retryFlaggedChunk = useCallback(
    async (chunkIndex: number) => {
      if (inFlightRef.current) {
        return;
      }
      const cfg = lastRequestConfigRef.current;
      if (!cfg) return;

      inFlightRef.current = true;
      const origChunks = originalChunksRef.current;
      const safeIndex =
        chunkIndex >= 0 && chunkIndex < origChunks.length ? chunkIndex : 0;
      const originalChunk = origChunks[safeIndex] ?? inputText;
      const contextBefore =
        safeIndex > 0 ? origChunks.slice(Math.max(0, safeIndex - 1), safeIndex).join("\n\n") : "";

      setIsRetryingChunk(true);
      setRunState((prev) => ({
        ...prev,
        state: "retrying",
        chunkIndex: safeIndex,
      }));

      try {
        const resp = await ipcRetryChunk({
          request_id: cfg.requestId,
          chunk_index: safeIndex,
          original_chunk: originalChunk,
          context_before: contextBefore,
          mode: cfg.mode,
          intensity: cfg.intensity,
          options: cfg.options,
        });

        if (!resp.rewritten_chunk || !resp.rewritten_chunk.trim()) {
          throw new Error(`Retry for chunk ${safeIndex + 1} returned empty output.`);
        }

        const latestOutput = outputTextRef.current;
        const currentJoined = outputChunksRef.current
          .filter((c) => c.length > 0)
          .join("\n\n");
        const expectedCount = Math.max(
          1,
          chunksTotalRef.current || origChunks.length,
        );
        const currentSlices =
          outputChunksRef.current.length > 0 &&
          currentJoined === latestOutput.trim()
            ? outputChunksRef.current
            : alignOutputChunks(latestOutput, expectedCount);

        const spliced = spliceRetriedChunkOutput(
          currentSlices,
          safeIndex,
          resp.rewritten_chunk,
          latestOutput,
        );
        outputChunksRef.current = spliced.nextChunks;
        setOutputText(spliced.nextOutputText);

        setRunState((prev) => {
          const nextMap = { ...prev.chunkWarningsMap };
          if (resp.warnings.length > 0) {
            nextMap[safeIndex] = resp.warnings;
          } else {
            delete nextMap[safeIndex];
          }
          const nextFlaggedSet = new Set<number>(prev.flaggedChunks);
          if (resp.warnings.length > 0) {
            nextFlaggedSet.add(safeIndex);
          } else {
            nextFlaggedSet.delete(safeIndex);
          }
          const nextFlagged = Array.from(nextFlaggedSet).sort((a, b) => a - b);

          const preservedOthers = prev.warnings.filter(
            (w) => !warningBelongsToChunk(w, safeIndex),
          );
          const remainingWarnings =
            resp.warnings.length > 0
              ? [...preservedOthers, ...resp.warnings]
              : preservedOthers;

          return {
            ...prev,
            state: "completed",
            chunkWarningsMap: nextMap,
            flaggedChunks: nextFlagged,
            warnings: remainingWarnings,
          };
        });

        notifyRef.current?.(
          resp.warnings.length > 0 ? "info" : "success",
          `Retried chunk ${safeIndex + 1}.`,
        );
      } catch (err) {
        const msg = sanitizeUserFacingError(err);
        setRunState((prev) => ({
          ...prev,
          state: "error",
          errorMessage: msg,
        }));
        notifyRef.current?.("error", msg);
      } finally {
        inFlightRef.current = false;
        setIsRetryingChunk(false);
      }
    },
    [inputText],
  );

  const clearInput = useCallback(() => {
    setInputText("");
    if (!runState.isRunning) {
      setRunState(createInitialHumanizeRunState());
      setElapsedMs(0);
    }
  }, [runState.isRunning]);

  const updateOption = useCallback(
    <K extends keyof RewriteOptions>(key: K, value: RewriteOptions[K]) => {
      setOptions((prev) => ({ ...prev, [key]: value }));
    },
    [],
  );

  return {
    inputText,
    setInputText,
    outputText,
    setOutputText,
    mode,
    setMode,
    intensity,
    setIntensity,
    options,
    updateOption,
    runState,
    elapsedMs,
    isRetryingChunk,
    startHumanize,
    cancelActiveHumanize,
    retryFlaggedChunk,
    clearInput,
  };
}
