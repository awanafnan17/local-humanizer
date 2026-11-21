import {
  AlertTriangle,
  CheckCircle2,
  Cpu,
  Loader2,
  Moon,
  RefreshCw,
  Settings,
  ShieldCheck,
  Sun,
} from "lucide-react";
import logoUrl from "../assets/logo.svg";
import { getReadinessDisplay } from "../lib/humanizeState.ts";
import type { AppStatus, ModelDescriptor } from "../lib/types.ts";
import type { ThemeMode } from "../hooks/useTheme.ts";

export interface HeaderProps {
  appStatus: AppStatus | null;
  statusError: string | null;
  models: readonly ModelDescriptor[];
  theme: ThemeMode;
  isRetryingInit: boolean;
  onToggleTheme: () => void;
  onOpenSettings: () => void;
  onRetryInit: () => void;
}

export function Header({
  appStatus,
  statusError,
  models,
  theme,
  isRetryingInit,
  onToggleTheme,
  onOpenSettings,
  onRetryInit,
}: HeaderProps) {
  const readiness = getReadinessDisplay(appStatus, statusError);
  const activeModel = models.find((m) => m.id === appStatus?.active_model_id);
  const modelLabel =
    activeModel?.name ??
    (appStatus?.active_model_id || "No model selected");

  const pillClasses =
    readiness.tone === "ready"
      ? "border-emerald-500/35 bg-emerald-500/10 text-emerald-800 dark:text-emerald-300"
      : readiness.tone === "error"
        ? "border-rose-500/40 bg-rose-500/10 text-rose-800 dark:text-rose-300"
        : "border-amber-500/35 bg-amber-500/10 text-amber-800 dark:text-amber-300";

  return (
    <header className="shrink-0 border-b border-[var(--lh-border)] bg-[var(--lh-surface)] px-4 py-2.5 sm:px-6">
      <div className="mx-auto flex max-w-[1600px] flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-3">
          <img
            src={logoUrl}
            alt="Local Humanizer"
            className="h-9 w-9 shrink-0 rounded-lg shadow-xs"
            data-testid="app-logo"
          />
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <h1 className="text-base font-semibold tracking-tight text-[var(--lh-text)] sm:text-lg">
                Local Humanizer
              </h1>
              <span className="inline-flex items-center gap-1 rounded-full border border-[var(--lh-border)] bg-[var(--lh-surface-muted)] px-2 py-0.5 text-[11px] font-medium text-[var(--lh-text-muted)]">
                <ShieldCheck className="h-3 w-3 text-[var(--lh-accent-text)]" />
                100% Offline
              </span>
            </div>
            <p className="text-xs text-[var(--lh-text-muted)] truncate">
              Privacy-first desktop paraphrasing · Zero cloud calls
            </p>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <div
            data-testid="readiness-indicator"
            data-state={appStatus?.state ?? (statusError ? "error" : "initializing")}
            className={`flex items-center gap-2 rounded-full border px-3 py-1 text-xs font-medium ${pillClasses}`}
            title={readiness.detail}
          >
            {readiness.tone === "ready" ? (
              <CheckCircle2 className="h-3.5 w-3.5 shrink-0" />
            ) : readiness.tone === "error" ? (
              <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
            ) : (
              <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" />
            )}
            <span data-testid="readiness-label">{readiness.label}</span>

            {appStatus && readiness.tone !== "error" && (
              <>
                <span className="opacity-40">·</span>
                <span
                  className="max-w-[180px] truncate text-[11px] opacity-90"
                  data-testid="active-model-label"
                >
                  {modelLabel}
                </span>
                <span
                  data-testid="backend-mode-badge"
                  className="inline-flex items-center gap-1 rounded bg-black/5 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide dark:bg-white/10"
                >
                  <Cpu className="h-2.5 w-2.5" />
                  {appStatus.backend_mode === "metal" ? "Metal" : "CPU"}
                </span>
              </>
            )}

            {readiness.tone === "error" && (
              <>
                <span className="opacity-40">·</span>
                <span
                  className="max-w-[220px] truncate text-[11px]"
                  data-testid="readiness-error-detail"
                >
                  {readiness.detail}
                </span>
                <button
                  type="button"
                  onClick={onRetryInit}
                  disabled={isRetryingInit}
                  data-testid="retry-runtime-button"
                  className="ml-1 inline-flex items-center gap-1 rounded-full bg-rose-600 px-2 py-0.5 text-[11px] font-semibold text-white hover:bg-rose-700 disabled:opacity-60 focus:outline-none focus:ring-2 focus:ring-rose-500"
                >
                  <RefreshCw
                    className={`h-3 w-3 ${isRetryingInit ? "animate-spin" : ""}`}
                  />
                  Retry
                </button>
              </>
            )}
          </div>

          <button
            type="button"
            onClick={onToggleTheme}
            data-testid="theme-toggle"
            aria-label={
              theme === "dark" ? "Switch to light mode" : "Switch to dark mode"
            }
            title={
              theme === "dark" ? "Switch to light mode" : "Switch to dark mode"
            }
            className="inline-flex h-8 w-8 items-center justify-center rounded-lg border border-[var(--lh-border)] bg-[var(--lh-surface-elevated)] text-[var(--lh-text-secondary)] hover:border-[var(--lh-border-strong)] hover:text-[var(--lh-text)] focus:outline-none focus:ring-2 focus:ring-[var(--lh-accent)]"
          >
            {theme === "dark" ? (
              <Sun className="h-4 w-4" />
            ) : (
              <Moon className="h-4 w-4" />
            )}
          </button>

          <button
            type="button"
            onClick={onOpenSettings}
            data-testid="settings-button"
            aria-label="Open Settings"
            className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-[var(--lh-border)] bg-[var(--lh-surface-elevated)] px-2.5 text-xs font-medium text-[var(--lh-text-secondary)] hover:border-[var(--lh-border-strong)] hover:text-[var(--lh-text)] focus:outline-none focus:ring-2 focus:ring-[var(--lh-accent)]"
          >
            <Settings className="h-3.5 w-3.5" />
            <span>Settings</span>
          </button>
        </div>
      </div>
    </header>
  );
}
