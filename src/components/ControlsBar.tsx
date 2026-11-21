import { useState } from "react";
import { SlidersHorizontal, Sparkles, Square } from "lucide-react";
import {
  INTENSITIES,
  REWRITE_MODES,
  SUPPORTED_LANGUAGES,
  VOICE_OPTIONS,
} from "../lib/constants.ts";
import type {
  Intensity,
  RewriteMode,
  RewriteOptions,
  VoiceOption,
} from "../lib/types.ts";

export interface ControlsBarProps {
  mode: RewriteMode;
  intensity: Intensity;
  options: RewriteOptions;
  canHumanize: boolean;
  isRunning: boolean;
  modelReady: boolean;
  onModeChange: (mode: RewriteMode) => void;
  onIntensityChange: (intensity: Intensity) => void;
  onOptionChange: <K extends keyof RewriteOptions>(
    key: K,
    value: RewriteOptions[K],
  ) => void;
  onHumanize: () => void;
  onCancel: () => void;
}

export function ControlsBar({
  mode,
  intensity,
  options,
  canHumanize,
  isRunning,
  modelReady,
  onModeChange,
  onIntensityChange,
  onOptionChange,
  onHumanize,
  onCancel,
}: ControlsBarProps) {
  const [optionsExpanded, setOptionsExpanded] = useState(false);

  const activeMode = REWRITE_MODES.find((m) => m.id === mode);
  const enabledGuardCount = [
    options.preserve_length,
    options.preserve_formatting,
    options.preserve_citations,
    options.preserve_quotes,
  ].filter(Boolean).length;

  return (
    <section
      aria-label="Paraphrasing Controls"
      className="shrink-0 border-b border-[var(--lh-border)] bg-[var(--lh-surface-elevated)] px-4 py-2.5 sm:px-6"
    >
      <div className="mx-auto flex max-w-[1600px] flex-col gap-2.5">
        <div className="flex flex-wrap items-center justify-between gap-2.5">
          {/* 6 Modes */}
          <div
            className="flex flex-wrap items-center gap-1"
            role="group"
            aria-label="Rewriting Mode"
          >
            {REWRITE_MODES.map((m) => {
              const selected = mode === m.id;
              return (
                <button
                  key={m.id}
                  type="button"
                  onClick={() => onModeChange(m.id)}
                  aria-pressed={selected}
                  data-testid={`mode-${m.id}`}
                  title={m.description}
                  disabled={isRunning}
                  className={`rounded-md px-2.5 py-1.5 text-xs font-medium transition-colors focus:outline-none focus:ring-2 focus:ring-[var(--lh-accent)] disabled:opacity-60 ${
                    selected
                      ? "bg-[var(--lh-accent)] text-white shadow-xs"
                      : "border border-[var(--lh-border)] bg-[var(--lh-surface)] text-[var(--lh-text-secondary)] hover:border-[var(--lh-border-strong)] hover:text-[var(--lh-text)]"
                  }`}
                >
                  {m.label}
                </button>
              );
            })}
          </div>

          <div className="flex flex-wrap items-center gap-2">
            {/* 3 Intensities */}
            <div
              className="inline-flex rounded-lg border border-[var(--lh-border)] bg-[var(--lh-surface)] p-0.5"
              role="group"
              aria-label="Rewriting Intensity"
            >
              {INTENSITIES.map((item) => {
                const selected = intensity === item.id;
                return (
                  <button
                    key={item.id}
                    type="button"
                    onClick={() => onIntensityChange(item.id)}
                    aria-pressed={selected}
                    data-testid={`intensity-${item.id}`}
                    title={item.description}
                    disabled={isRunning}
                    className={`rounded-md px-2.5 py-1 text-xs font-medium transition-colors focus:outline-none focus:ring-2 focus:ring-[var(--lh-accent)] disabled:opacity-60 ${
                      selected
                        ? "bg-[var(--lh-accent-soft)] text-[var(--lh-accent-text)] font-semibold"
                        : "text-[var(--lh-text-muted)] hover:text-[var(--lh-text)]"
                    }`}
                  >
                    {item.label}
                  </button>
                );
              })}
            </div>

            {/* Optional Controls Toggle */}
            <button
              type="button"
              onClick={() => setOptionsExpanded((prev) => !prev)}
              aria-expanded={optionsExpanded}
              data-testid="options-toggle-button"
              className={`inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-xs font-medium transition-colors focus:outline-none focus:ring-2 focus:ring-[var(--lh-accent)] ${
                optionsExpanded
                  ? "border-[var(--lh-accent)] bg-[var(--lh-accent-soft)] text-[var(--lh-accent-text)]"
                  : "border-[var(--lh-border)] bg-[var(--lh-surface)] text-[var(--lh-text-secondary)] hover:border-[var(--lh-border-strong)]"
              }`}
            >
              <SlidersHorizontal className="h-3.5 w-3.5" />
              <span>Options ({enabledGuardCount})</span>
            </button>

            {/* Primary Action: Humanize / Cancel */}
            {isRunning ? (
              <button
                type="button"
                onClick={onCancel}
                data-testid="cancel-humanize-button"
                className="inline-flex items-center gap-1.5 rounded-lg bg-rose-600 px-4 py-1.5 text-xs font-semibold text-white shadow-xs hover:bg-rose-700 focus:outline-none focus:ring-2 focus:ring-rose-500"
              >
                <Square className="h-3.5 w-3.5 fill-current" />
                <span>Cancel</span>
              </button>
            ) : (
              <button
                type="button"
                onClick={onHumanize}
                disabled={!canHumanize}
                data-testid="humanize-button"
                title={
                  !modelReady
                    ? "Waiting for local model runtime to become ready"
                    : !canHumanize
                      ? "Enter or paste text to humanize"
                      : "Humanize text (Cmd/Ctrl + Enter)"
                }
                className="inline-flex items-center gap-1.5 rounded-lg bg-[var(--lh-accent)] px-4 py-1.5 text-xs font-semibold text-white shadow-xs transition-colors hover:bg-[var(--lh-accent-hover)] disabled:cursor-not-allowed disabled:opacity-45 focus:outline-none focus:ring-2 focus:ring-[var(--lh-accent)]"
              >
                <Sparkles className="h-3.5 w-3.5" />
                <span>Humanize</span>
              </button>
            )}
          </div>
        </div>

        {/* Collapsible Optional Controls Panel */}
        {optionsExpanded && (
          <div
            data-testid="optional-controls-panel"
            className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-[var(--lh-border)] bg-[var(--lh-surface)] px-3 py-2 text-xs"
          >
            <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
              <label className="inline-flex cursor-pointer items-center gap-1.5 text-[var(--lh-text-secondary)]">
                <input
                  type="checkbox"
                  checked={options.preserve_length}
                  onChange={(e) =>
                    onOptionChange("preserve_length", e.target.checked)
                  }
                  disabled={isRunning}
                  data-testid="option-preserve-length"
                  className="h-3.5 w-3.5 rounded accent-[var(--lh-accent)]"
                />
                <span>Preserve length</span>
              </label>

              <label className="inline-flex cursor-pointer items-center gap-1.5 text-[var(--lh-text-secondary)]">
                <input
                  type="checkbox"
                  checked={options.preserve_formatting}
                  onChange={(e) =>
                    onOptionChange("preserve_formatting", e.target.checked)
                  }
                  disabled={isRunning}
                  data-testid="option-preserve-formatting"
                  className="h-3.5 w-3.5 rounded accent-[var(--lh-accent)]"
                />
                <span>Preserve formatting</span>
              </label>

              <label className="inline-flex cursor-pointer items-center gap-1.5 text-[var(--lh-text-secondary)]">
                <input
                  type="checkbox"
                  checked={options.preserve_citations}
                  onChange={(e) =>
                    onOptionChange("preserve_citations", e.target.checked)
                  }
                  disabled={isRunning}
                  data-testid="option-preserve-citations"
                  className="h-3.5 w-3.5 rounded accent-[var(--lh-accent)]"
                />
                <span>Preserve citations</span>
              </label>

              <label className="inline-flex cursor-pointer items-center gap-1.5 text-[var(--lh-text-secondary)]">
                <input
                  type="checkbox"
                  checked={options.preserve_quotes}
                  onChange={(e) =>
                    onOptionChange("preserve_quotes", e.target.checked)
                  }
                  disabled={isRunning}
                  data-testid="option-preserve-quotes"
                  className="h-3.5 w-3.5 rounded accent-[var(--lh-accent)]"
                />
                <span>Preserve exact quotes</span>
              </label>
            </div>

            <div className="flex flex-wrap items-center gap-3">
              <label className="inline-flex items-center gap-1.5 text-[var(--lh-text-muted)]">
                <span>Language:</span>
                <select
                  value={options.language}
                  onChange={(e) => onOptionChange("language", e.target.value)}
                  disabled={isRunning}
                  data-testid="option-language"
                  className="rounded-md border border-[var(--lh-border)] bg-[var(--lh-surface-elevated)] px-2 py-1 text-xs text-[var(--lh-text)] focus:outline-none focus:ring-2 focus:ring-[var(--lh-accent)]"
                >
                  {SUPPORTED_LANGUAGES.map((lang) => (
                    <option key={lang.code} value={lang.code}>
                      {lang.label}
                    </option>
                  ))}
                </select>
              </label>

              <label className="inline-flex items-center gap-1.5 text-[var(--lh-text-muted)]">
                <span>Voice:</span>
                <select
                  value={options.voice}
                  onChange={(e) =>
                    onOptionChange("voice", e.target.value as VoiceOption)
                  }
                  disabled={isRunning}
                  data-testid="option-voice"
                  className="rounded-md border border-[var(--lh-border)] bg-[var(--lh-surface-elevated)] px-2 py-1 text-xs text-[var(--lh-text)] focus:outline-none focus:ring-2 focus:ring-[var(--lh-accent)]"
                >
                  {VOICE_OPTIONS.map((v) => (
                    <option key={v.id} value={v.id}>
                      {v.label}
                    </option>
                  ))}
                </select>
              </label>
            </div>
          </div>
        )}

        {activeMode && (
          <p className="text-[11px] text-[var(--lh-text-muted)]">
            <span className="font-medium text-[var(--lh-text-secondary)]">
              {activeMode.label}:
            </span>{" "}
            {activeMode.description}
          </p>
        )}
      </div>
    </section>
  );
}
