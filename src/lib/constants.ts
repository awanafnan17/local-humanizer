import type { Intensity, RewriteMode, RewriteOptions, VoiceOption } from "./types.ts";

export const THEME_STORAGE_KEY = "local-humanizer.theme";
export const MAX_IMPORT_BYTES = 2 * 1024 * 1024;
export const LONG_DOC_WORD_THRESHOLD = 3000;

export interface ModeMetadata {
  id: RewriteMode;
  label: string;
  shortLabel: string;
  description: string;
}

export const REWRITE_MODES: readonly ModeMetadata[] = [
  {
    id: "standard",
    label: "Standard Humanize",
    shortLabel: "Standard",
    description: "Balanced clarity and natural phrasing with faithful meaning retention.",
  },
  {
    id: "natural",
    label: "Natural & Conversational",
    shortLabel: "Natural",
    description: "Warm, fluid everyday cadence with smoother transitions.",
  },
  {
    id: "academic",
    label: "Academic & Formal",
    shortLabel: "Academic",
    description: "Scholarly precision while keeping terminology and citations intact.",
  },
  {
    id: "professional",
    label: "Professional",
    shortLabel: "Professional",
    description: "Crisp executive and business prose free of filler.",
  },
  {
    id: "simple",
    label: "Simple & Clear",
    shortLabel: "Simple",
    description: "Plain-language readability with shorter, direct sentences.",
  },
  {
    id: "deep",
    label: "Deep Paraphrase",
    shortLabel: "Deep",
    description: "Thorough syntactic restructuring while preserving all facts.",
  },
];

export interface IntensityMetadata {
  id: Intensity;
  label: string;
  description: string;
}

export const INTENSITIES: readonly IntensityMetadata[] = [
  { id: "light", label: "Light", description: "Subtle polish and flow edits" },
  { id: "balanced", label: "Balanced", description: "Moderate structural rewording" },
  { id: "strong", label: "Strong", description: "Extensive phrasing transformation" },
];

export interface VoiceMetadata {
  id: VoiceOption;
  label: string;
}

export const VOICE_OPTIONS: readonly VoiceMetadata[] = [
  { id: "auto", label: "Auto (Preserve Voice)" },
  { id: "first_person", label: "First-person (I / We)" },
  { id: "third_person", label: "Third-person (They / It)" },
];

export interface LanguageOption {
  code: string;
  label: string;
}

export const SUPPORTED_LANGUAGES: readonly LanguageOption[] = [
  { code: "en", label: "English (Auto)" },
  { code: "en-US", label: "English (US)" },
  { code: "en-GB", label: "English (UK)" },
  { code: "es", label: "Spanish" },
  { code: "fr", label: "French" },
  { code: "de", label: "German" },
  { code: "it", label: "Italian" },
  { code: "pt", label: "Portuguese" },
  { code: "nl", label: "Dutch" },
  { code: "ja", label: "Japanese" },
  { code: "zh", label: "Chinese" },
  { code: "ko", label: "Korean" },
  { code: "ar", label: "Arabic" },
];

export const DEFAULT_REWRITE_OPTIONS: RewriteOptions = {
  preserve_length: true,
  preserve_formatting: true,
  preserve_citations: true,
  preserve_quotes: true,
  language: "en",
  voice: "auto",
};

export interface KeyboardShortcutEntry {
  keys: string;
  action: string;
}

export const KEYBOARD_SHORTCUTS: readonly KeyboardShortcutEntry[] = [
  { keys: "Cmd / Ctrl + Enter", action: "Humanize original text" },
  { keys: "Cmd / Ctrl + Shift + C", action: "Copy humanized output" },
  { keys: "Cmd / Ctrl + S", action: "Save humanized output" },
  { keys: "Escape", action: "Close Settings or cancel active run" },
];

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let idx = 0;
  while (value >= 1024 && idx < units.length - 1) {
    value /= 1024;
    idx += 1;
  }
  const decimals = idx >= 2 ? 2 : idx === 1 ? 1 : 0;
  return `${value.toFixed(decimals)} ${units[idx]}`;
}

export function formatDurationMs(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "0.0s";
  if (ms < 1000) return `${Math.round(ms)}ms`;
  const totalSeconds = ms / 1000;
  if (totalSeconds < 60) return `${totalSeconds.toFixed(1)}s`;
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = Math.floor(totalSeconds % 60);
  return `${minutes}m ${String(seconds).padStart(2, "0")}s`;
}
