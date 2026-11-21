import { AlertTriangle, CheckCircle2, Loader2, RotateCcw, XCircle } from "lucide-react";
import { formatDurationMs } from "../lib/constants.ts";
import {
  computeProgressPercent,
  formatProgressStageLabel,
  type HumanizeRunState,
} from "../lib/humanizeState.ts";

export interface ProgressBannerProps {
  runState: HumanizeRunState;
  elapsedMs: number;
  isRetryingChunk: boolean;
  onRetryChunk: (chunkIndex: number) => void;
}

export function ProgressBanner({
  runState,
  elapsedMs,
  isRetryingChunk,
  onRetryChunk,
}: ProgressBannerProps) {
  if (!runState.state) {
    return null;
  }

  const stageLabel = formatProgressStageLabel(
    runState.state,
    runState.chunkIndex,
    runState.chunksTotal,
  );
  const percent = computeProgressPercent(
    runState.state,
    runState.chunkIndex,
    runState.chunksTotal,
  );

  const isError = runState.state === "error";
  const isCancelled = runState.state === "cancelled";
  const isCompleted = runState.state === "completed";
  const hasWarnings =
    runState.warnings.length > 0 || runState.flaggedChunks.length > 0;

  const borderTone = isError
    ? "border-rose-500/40 bg-rose-500/5"
    : isCancelled
      ? "border-amber-500/40 bg-amber-500/5"
      : hasWarnings
        ? "border-amber-500/40 bg-amber-500/5"
        : isCompleted
          ? "border-emerald-500/35 bg-emerald-500/5"
          : "border-[var(--lh-border)] bg-[var(--lh-surface)]";

  const previewText = runState.partialOutput
    ? runState.partialOutput.replace(/\s+/g, " ").trim().slice(-140)
    : "";

  return (
    <section
      aria-label="Paraphrasing Progress"
      data-testid="progress-banner"
      data-state={runState.state}
      className={`shrink-0 rounded-xl border px-4 py-2.5 text-xs ${borderTone}`}
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2 font-medium text-[var(--lh-text)]">
          {runState.isRunning || isRetryingChunk ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin text-[var(--lh-accent-text)]" />
          ) : isError ? (
            <XCircle className="h-3.5 w-3.5 text-rose-500" />
          ) : hasWarnings || isCancelled ? (
            <AlertTriangle className="h-3.5 w-3.5 text-amber-500" />
          ) : (
            <CheckCircle2 className="h-3.5 w-3.5 text-emerald-500" />
          )}
          <span data-testid="progress-stage-label">{stageLabel}</span>
          {runState.chunksTotal > 0 && (
            <span
              className="tabular-nums text-[var(--lh-text-muted)]"
              data-testid="progress-chunk-counter"
            >
              ({Math.min(runState.chunkIndex + 1, runState.chunksTotal)}/{runState.chunksTotal} chunks)
            </span>
          )}
        </div>

        <div className="flex items-center gap-3 tabular-nums text-[var(--lh-text-muted)]">
          {runState.isRunning && (
            <span data-testid="progress-percent">{percent}%</span>
          )}
          <span data-testid="progress-elapsed">
            {formatDurationMs(runState.durationMs ?? elapsedMs)}
          </span>
        </div>
      </div>

      {runState.isRunning && (
        <div className="mt-2 h-1.5 w-full overflow-hidden rounded-full bg-[var(--lh-surface-muted)]">
          <div
            role="progressbar"
            aria-valuenow={percent}
            aria-valuemin={0}
            aria-valuemax={100}
            data-testid="progress-bar"
            className="h-full rounded-full bg-[var(--lh-accent)] transition-all duration-150"
            style={{ width: `${percent}%` }}
          />
        </div>
      )}

      {runState.isRunning && previewText && (
        <p
          data-testid="progress-partial-preview"
          className="mt-1.5 truncate text-[11px] text-[var(--lh-text-muted)]"
        >
          Preview: {previewText}
        </p>
      )}

      {runState.errorMessage && (
        <p
          data-testid="progress-error-message"
          className="mt-1.5 text-rose-700 dark:text-rose-300"
        >
          {runState.errorMessage}
        </p>
      )}

      {(runState.warnings.length > 0 || runState.flaggedChunks.length > 0) && (
        <div
          data-testid="progress-warnings"
          className="mt-2 flex flex-wrap items-center justify-between gap-2 border-t border-[var(--lh-border)] pt-2"
        >
          <ul className="flex flex-col gap-0.5 text-[11px] text-amber-800 dark:text-amber-200">
            {runState.warnings.map((w, idx) => (
              <li key={`${idx}-${w.slice(0, 24)}`}>{w}</li>
            ))}
          </ul>

          {!runState.isRunning && runState.flaggedChunks.length > 0 && (
            <div className="flex flex-wrap items-center gap-1.5">
              {runState.flaggedChunks.map((chunkIdx) => (
                <button
                  key={chunkIdx}
                  type="button"
                  disabled={isRetryingChunk}
                  onClick={() => onRetryChunk(chunkIdx)}
                  data-testid="retry-chunk-button"
                  data-chunk-index={chunkIdx}
                  className="inline-flex items-center gap-1 rounded-md border border-amber-500/50 bg-amber-500/15 px-2.5 py-1 text-[11px] font-semibold text-amber-900 hover:bg-amber-500/25 disabled:opacity-50 dark:text-amber-100 focus:outline-none focus:ring-2 focus:ring-amber-500"
                >
                  <RotateCcw
                    className={`h-3 w-3 ${isRetryingChunk ? "animate-spin" : ""}`}
                  />
                  <span>Retry Flagged Chunk ({chunkIdx + 1})</span>
                </button>
              ))}
            </div>
          )}
        </div>
      )}
    </section>
  );
}
