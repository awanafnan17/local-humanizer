import { useEffect, useRef, useState } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  Cpu,
  Download,
  Keyboard,
  RefreshCw,
  ShieldCheck,
  X,
  XCircle,
} from "lucide-react";
import { formatBytes, KEYBOARD_SHORTCUTS } from "../lib/constants.ts";
import type {
  AppStatus,
  ModelDescriptor,
  ModelDownloadProgressEvent,
} from "../lib/types.ts";

export interface SettingsModalProps {
  isOpen: boolean;
  appStatus: AppStatus | null;
  models: readonly ModelDescriptor[];
  modelsError?: string | null;
  downloadProgress: Record<string, ModelDownloadProgressEvent>;
  isSwitchingModel: boolean;
  onClose: () => void;
  onRefreshModels?: () => void;
  onSelectModel: (modelId: string, useGpu: boolean) => void;
  onDownloadModel: (modelId: string) => void;
  onCancelDownload: (modelId: string) => void;
}

export function SettingsModal({
  isOpen,
  appStatus,
  models,
  modelsError,
  downloadProgress,
  isSwitchingModel,
  onClose,
  onRefreshModels,
  onSelectModel,
  onDownloadModel,
  onCancelDownload,
}: SettingsModalProps) {
  const closeButtonRef = useRef<HTMLButtonElement | null>(null);
  const [useGpu, setUseGpu] = useState<boolean>(
    appStatus?.backend_mode === "metal",
  );
  const backendMode = appStatus?.backend_mode;

  useEffect(() => {
    if (isOpen && backendMode) {
      setUseGpu(backendMode === "metal");
    }
  }, [backendMode, isOpen]);

  useEffect(() => {
    if (isOpen) {
      closeButtonRef.current?.focus();
    }
  }, [isOpen]);

  if (!isOpen) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-4 backdrop-blur-[1px]"
      onClick={(e) => {
        if (e.target === e.currentTarget) {
          onClose();
        }
      }}
      data-testid="settings-backdrop"
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-modal-title"
        data-testid="settings-modal"
        className="lh-scroll flex max-h-[88vh] w-full max-w-2xl flex-col overflow-hidden rounded-2xl border border-[var(--lh-border)] bg-[var(--lh-surface)] text-[var(--lh-text)] shadow-2xl"
      >
        {/* Modal Header */}
        <div className="flex shrink-0 items-center justify-between border-b border-[var(--lh-border)] bg-[var(--lh-surface-elevated)] px-5 py-3.5">
          <div>
            <h2 id="settings-modal-title" className="text-base font-semibold">
              Settings &amp; Local Models
            </h2>
            <p className="text-xs text-[var(--lh-text-muted)]">
              Manage offline GGUF models, hardware acceleration, and shortcuts
            </p>
          </div>
          <button
            ref={closeButtonRef}
            type="button"
            onClick={onClose}
            aria-label="Close Settings"
            data-testid="close-settings-button"
            className="rounded-lg p-1.5 text-[var(--lh-text-muted)] hover:bg-[var(--lh-surface-muted)] hover:text-[var(--lh-text)] focus:outline-none focus:ring-2 focus:ring-[var(--lh-accent)]"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* Modal Body */}
        <div className="lh-scroll flex-1 space-y-5 overflow-y-auto p-5">
          {/* Runtime Acceleration */}
          <div className="flex items-center justify-between rounded-xl border border-[var(--lh-border)] bg-[var(--lh-surface-elevated)] px-4 py-3">
            <div className="flex items-center gap-2.5">
              <Cpu className="h-4 w-4 text-[var(--lh-accent-text)]" />
              <div>
                <div className="text-xs font-semibold">
                  Hardware Acceleration (Metal / GPU)
                </div>
                <div className="text-[11px] text-[var(--lh-text-muted)]">
                  Applied when switching or activating a model (falls back to CPU when unavailable)
                </div>
              </div>
            </div>

            <label className="inline-flex cursor-pointer items-center gap-2 text-xs font-medium">
              <input
                type="checkbox"
                checked={useGpu}
                onChange={(e) => setUseGpu(e.target.checked)}
                data-testid="gpu-toggle"
                className="h-4 w-4 rounded accent-[var(--lh-accent)]"
              />
              <span>{useGpu ? "Metal / GPU" : "CPU Only"}</span>
            </label>
          </div>

          {/* Model Catalog */}
          <div className="space-y-2.5">
            <div className="flex items-center justify-between">
              <h3 className="text-xs font-semibold uppercase tracking-wider text-[var(--lh-text-muted)]">
                Available Local Models
              </h3>
              <span className="text-[11px] text-[var(--lh-text-muted)]">
                Licenses documented in THIRD_PARTY_NOTICES.md
              </span>
            </div>

            <div className="space-y-2.5">
              {models.length === 0 && (
                <div
                  data-testid="empty-models-state"
                  className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-amber-500/40 bg-amber-500/10 p-3.5 text-xs text-amber-900 dark:text-amber-200"
                >
                  <span>
                    {modelsError ||
                      "No local models could be loaded from the catalog."}
                  </span>
                  {onRefreshModels && (
                    <button
                      type="button"
                      onClick={onRefreshModels}
                      data-testid="refresh-models-button"
                      className="inline-flex items-center gap-1.5 rounded-lg border border-amber-500/50 bg-[var(--lh-surface)] px-2.5 py-1.5 text-xs font-semibold text-[var(--lh-text)] hover:border-[var(--lh-accent)]"
                    >
                      <RefreshCw className="h-3.5 w-3.5" />
                      <span>Refresh / Retry</span>
                    </button>
                  )}
                </div>
              )}

              {models.map((model) => {
                const isActive = appStatus?.active_model_id === model.id;
                const dl = downloadProgress[model.id];
                const isDownloading =
                  dl?.status === "downloading" || dl?.status === "verifying";
                const dlPercent =
                  dl && dl.total_bytes > 0
                    ? Math.min(
                        100,
                        Math.round((dl.downloaded_bytes / dl.total_bytes) * 100),
                      )
                    : 0;
                const canSelect = model.is_installed;

                return (
                  <div
                    key={model.id}
                    data-testid={`model-card-${model.id}`}
                    className={`rounded-xl border p-3.5 transition-colors ${
                      isActive
                        ? "border-[var(--lh-accent)] bg-[var(--lh-accent-soft)]"
                        : "border-[var(--lh-border)] bg-[var(--lh-surface-elevated)]"
                    }`}
                  >
                    <div className="flex flex-wrap items-start justify-between gap-2">
                      <div className="space-y-1">
                        <div className="flex flex-wrap items-center gap-1.5">
                          <span className="text-sm font-semibold">
                            {model.name}
                          </span>
                          {model.is_default && (
                            <span className="rounded bg-[var(--lh-accent)] px-1.5 py-0.5 text-[10px] font-semibold text-white">
                              Default
                            </span>
                          )}
                          {model.is_bundled && (
                            <span className="rounded border border-[var(--lh-border)] bg-[var(--lh-surface)] px-1.5 py-0.5 text-[10px] font-medium text-[var(--lh-text-secondary)]">
                              Bundled
                            </span>
                          )}
                          {model.is_installed && !model.is_bundled && (
                            <span className="rounded border border-emerald-500/40 bg-emerald-500/10 px-1.5 py-0.5 text-[10px] font-medium text-emerald-700 dark:text-emerald-300">
                              Installed
                            </span>
                          )}
                          {isActive && (
                            <span
                              data-testid={`active-badge-${model.id}`}
                              className="inline-flex items-center gap-1 rounded border border-emerald-500/40 bg-emerald-500/15 px-1.5 py-0.5 text-[10px] font-semibold text-emerald-800 dark:text-emerald-200"
                            >
                              <CheckCircle2 className="h-3 w-3" />
                              Active
                            </span>
                          )}
                        </div>

                        <div className="flex flex-wrap items-center gap-2 text-[11px] text-[var(--lh-text-muted)]">
                          <span className="font-mono">{model.quant}</span>
                          <span>·</span>
                          <span className="tabular-nums">
                            {formatBytes(model.size_bytes)}
                          </span>
                          <span>·</span>
                          <span>&gt;={model.min_recommended_ram_gb} GB RAM</span>
                          <span>·</span>
                          <span className="rounded border border-[var(--lh-border)] bg-[var(--lh-surface)] px-1.5 py-0.5 text-[10px]">
                            {model.license}
                          </span>
                        </div>

                        {model.non_commercial_warning && (
                          <div
                            data-testid="non-commercial-badge"
                            className="mt-1 inline-flex items-center gap-1.5 rounded-md border border-amber-500/45 bg-amber-500/15 px-2 py-0.5 text-[11px] font-medium text-amber-900 dark:text-amber-200"
                          >
                            <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
                            <span>
                              Non-commercial research/evaluation license only
                            </span>
                          </div>
                        )}
                      </div>

                      <div className="flex items-center gap-2">
                        {isDownloading ? (
                          <button
                            type="button"
                            onClick={() => onCancelDownload(model.id)}
                            data-testid={`cancel-download-${model.id}`}
                            className="inline-flex items-center gap-1 rounded-lg border border-rose-500/40 bg-rose-500/10 px-2.5 py-1.5 text-xs font-medium text-rose-700 hover:bg-rose-500/20 dark:text-rose-300"
                          >
                            <XCircle className="h-3.5 w-3.5" />
                            <span>Cancel Download</span>
                          </button>
                        ) : canSelect ? (
                          <button
                            type="button"
                            disabled={isSwitchingModel}
                            onClick={() => onSelectModel(model.id, useGpu)}
                            data-testid={`select-model-${model.id}`}
                            className="rounded-lg bg-[var(--lh-accent)] px-3 py-1.5 text-xs font-semibold text-white hover:bg-[var(--lh-accent-hover)] disabled:opacity-50"
                          >
                            {isActive ? "Re-apply" : "Select Model"}
                          </button>
                        ) : (
                          <button
                            type="button"
                            onClick={() => onDownloadModel(model.id)}
                            data-testid={`download-model-${model.id}`}
                            className="inline-flex items-center gap-1 rounded-lg border border-[var(--lh-border-strong)] bg-[var(--lh-surface)] px-3 py-1.5 text-xs font-semibold text-[var(--lh-text)] hover:border-[var(--lh-accent)]"
                          >
                            <Download className="h-3.5 w-3.5" />
                            <span>Download</span>
                          </button>
                        )}
                      </div>
                    </div>

                    {isDownloading && dl && (
                      <div
                        data-testid={`download-progress-${model.id}`}
                        className="mt-2.5 space-y-1"
                      >
                        <div className="flex items-center justify-between text-[11px] tabular-nums text-[var(--lh-text-muted)]">
                          <span>
                            {dl.status === "verifying"
                              ? "Verifying SHA-256…"
                              : "Downloading model…"}
                          </span>
                          <span>
                            {formatBytes(dl.downloaded_bytes)} /{" "}
                            {formatBytes(dl.total_bytes)} ({dlPercent}%)
                          </span>
                        </div>
                        <div className="h-1.5 w-full overflow-hidden rounded-full bg-[var(--lh-surface-muted)]">
                          <div
                            className="h-full rounded-full bg-[var(--lh-accent)] transition-all"
                            style={{ width: `${dlPercent}%` }}
                          />
                        </div>
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          </div>

          {/* 100% Offline Privacy Summary */}
          <div
            data-testid="privacy-summary"
            className="rounded-xl border border-[var(--lh-border)] bg-[var(--lh-surface-elevated)] p-3.5"
          >
            <div className="flex items-center gap-2 text-xs font-semibold text-[var(--lh-text)]">
              <ShieldCheck className="h-4 w-4 text-[var(--lh-accent-text)]" />
              <span>100% Offline Privacy Guarantee</span>
            </div>
            <p className="mt-1 text-xs leading-relaxed text-[var(--lh-text-muted)]">
              All paraphrasing runs locally on your machine using bundled
              llama.cpp and quantized GGUF models. Zero cloud API calls, zero
              telemetry, zero accounts, and zero document persistence in local
              storage.
            </p>
          </div>

          {/* Keyboard Shortcuts Cheat-Sheet */}
          <div
            data-testid="shortcuts-cheatsheet"
            className="rounded-xl border border-[var(--lh-border)] bg-[var(--lh-surface-elevated)] p-3.5"
          >
            <div className="flex items-center gap-2 text-xs font-semibold text-[var(--lh-text)]">
              <Keyboard className="h-4 w-4 text-[var(--lh-accent-text)]" />
              <span>Keyboard Shortcuts</span>
            </div>
            <div className="mt-2 grid grid-cols-1 gap-1.5 sm:grid-cols-2">
              {KEYBOARD_SHORTCUTS.map((entry) => (
                <div
                  key={entry.keys}
                  className="flex items-center justify-between gap-2 rounded-md border border-[var(--lh-border)] bg-[var(--lh-surface)] px-2.5 py-1.5 text-[11px]"
                >
                  <span className="text-[var(--lh-text-secondary)]">
                    {entry.action}
                  </span>
                  <kbd className="rounded border border-[var(--lh-border)] bg-[var(--lh-surface-muted)] px-1.5 py-0.5 font-mono text-[10px] font-semibold">
                    {entry.keys}
                  </kbd>
                </div>
              ))}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
