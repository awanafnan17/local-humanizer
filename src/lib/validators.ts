import { MAX_IMPORT_BYTES, THEME_STORAGE_KEY } from "./constants.ts";

export interface ValidationResult {
  valid: boolean;
  error?: string;
}

const ALLOWED_EXTENSIONS = new Set([".txt", ".md", ".markdown"]);

export function validateImportFilename(filename: string): ValidationResult {
  const trimmed = filename.trim().toLowerCase();
  if (!trimmed) {
    return { valid: false, error: "File name is empty." };
  }
  const dotIdx = trimmed.lastIndexOf(".");
  const ext = dotIdx >= 0 ? trimmed.slice(dotIdx) : "";
  if (!ALLOWED_EXTENSIONS.has(ext)) {
    return {
      valid: false,
      error: "Only UTF-8 .txt, .md, and .markdown files are supported for import.",
    };
  }
  return { valid: true };
}

export function validateImportContent(
  text: string,
  byteLength?: number,
): ValidationResult {
  const actualBytes =
    typeof byteLength === "number"
      ? byteLength
      : new TextEncoder().encode(text).length;

  if (actualBytes > MAX_IMPORT_BYTES) {
    return {
      valid: false,
      error: "File exceeds the 2 MiB text import limit.",
    };
  }

  if (text.includes("\u0000")) {
    return {
      valid: false,
      error: "Binary files are not supported. Please import plain UTF-8 text.",
    };
  }

  return { valid: true };
}

export function isAllowedStorageKey(key: string): boolean {
  return key === THEME_STORAGE_KEY;
}

export function sanitizeUserFacingError(err: unknown): string {
  const raw = err instanceof Error ? err.message : String(err ?? "Unknown error");
  const singleLine = raw.replace(/[\r\n\t]+/g, " ").trim();
  if (!singleLine) return "An unexpected error occurred.";
  return singleLine.length > 260 ? `${singleLine.slice(0, 257)}…` : singleLine;
}
