import {
  useDeferredValue,
  useMemo,
  useRef,
  type ChangeEvent,
} from "react";
import {
  Check,
  ClipboardPaste,
  Copy,
  Download,
  FileUp,
  ListChecks,
  Trash2,
} from "lucide-react";
import {
  computeWordDeltaPercent,
  countChars,
  countWords,
} from "../lib/wordCount.ts";

export interface EditorPaneProps {
  variant: "input" | "output";
  value: string;
  onChange: (next: string) => void;
  disabled?: boolean;
  comparisonWordCount?: number;
  copied?: boolean;
  onPasteClick?: () => void;
  onClearClick?: () => void;
  onImportFileClick?: () => void;
  onBrowserFileSelected?: (file: File) => void;
  onCopyClick?: () => void;
  onSaveClick?: () => void;
}

export function EditorPane({
  variant,
  value,
  onChange,
  disabled = false,
  comparisonWordCount = 0,
  copied = false,
  onPasteClick,
  onClearClick,
  onImportFileClick,
  onBrowserFileSelected,
  onCopyClick,
  onSaveClick,
}: EditorPaneProps) {
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  const deferredValue = useDeferredValue(value);

  const wordCount = useMemo(() => countWords(deferredValue), [deferredValue]);
  const charCount = useMemo(() => countChars(deferredValue), [deferredValue]);

  const wordDelta = useMemo(() => {
    if (variant !== "output") return null;
    return computeWordDeltaPercent(comparisonWordCount, wordCount);
  }, [variant, comparisonWordCount, wordCount]);

  const handleSelectAll = () => {
    const el = textareaRef.current;
    if (!el) return;
    el.focus();
    el.select();
  };

  const handleFileInputChange = (e: ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file && onBrowserFileSelected) {
      onBrowserFileSelected(file);
    }
    e.target.value = "";
  };

  const triggerImport = () => {
    if (onImportFileClick) {
      onImportFileClick();
    } else {
      fileInputRef.current?.click();
    }
  };

  const isInput = variant === "input";
  const title = isInput ? "Original Text" : "Humanized Text";

  return (
    <div
      data-testid={isInput ? "original-pane" : "humanized-pane"}
      className="flex min-h-[260px] flex-1 flex-col overflow-hidden rounded-xl border border-[var(--lh-border)] bg-[var(--lh-surface)] shadow-xs"
    >
      {/* Pane Toolbar */}
      <div className="flex shrink-0 flex-wrap items-center justify-between gap-2 border-b border-[var(--lh-border)] bg-[var(--lh-surface-elevated)] px-3.5 py-2">
        <div className="flex items-center gap-2">
          <h2 className="text-xs font-semibold uppercase tracking-wider text-[var(--lh-text-secondary)]">
            {title}
          </h2>
          {!isInput && wordDelta && (
            <span
              data-testid="word-delta"
              className="tabular-nums rounded-full border border-[var(--lh-border)] bg-[var(--lh-surface)] px-2 py-0.5 text-[11px] font-medium text-[var(--lh-text-muted)]"
              title={`Length change vs. original (${wordDelta.deltaWords >= 0 ? "+" : ""}${wordDelta.deltaWords} words)`}
            >
              {wordDelta.formatted}
            </span>
          )}
        </div>

        <div className="flex flex-wrap items-center gap-1.5">
          {isInput ? (
            <>
              <button
                type="button"
                onClick={onPasteClick}
                disabled={disabled}
                data-testid="paste-button"
                className="inline-flex items-center gap-1 rounded-md border border-[var(--lh-border)] bg-[var(--lh-surface)] px-2 py-1 text-xs font-medium text-[var(--lh-text-secondary)] hover:border-[var(--lh-border-strong)] hover:text-[var(--lh-text)] disabled:opacity-50 focus:outline-none focus:ring-2 focus:ring-[var(--lh-accent)]"
              >
                <ClipboardPaste className="h-3.5 w-3.5" />
                <span>Paste</span>
              </button>

              <button
                type="button"
                onClick={triggerImport}
                disabled={disabled}
                data-testid="import-button"
                className="inline-flex items-center gap-1 rounded-md border border-[var(--lh-border)] bg-[var(--lh-surface)] px-2 py-1 text-xs font-medium text-[var(--lh-text-secondary)] hover:border-[var(--lh-border-strong)] hover:text-[var(--lh-text)] disabled:opacity-50 focus:outline-none focus:ring-2 focus:ring-[var(--lh-accent)]"
              >
                <FileUp className="h-3.5 w-3.5" />
                <span>Import</span>
              </button>

              <input
                ref={fileInputRef}
                type="file"
                accept=".txt,.md,.markdown,text/plain,text/markdown"
                onChange={handleFileInputChange}
                data-testid="import-file-input"
                className="hidden"
                aria-hidden="true"
                tabIndex={-1}
              />

              <button
                type="button"
                onClick={onClearClick}
                disabled={disabled || value.length === 0}
                data-testid="clear-button"
                className="inline-flex items-center gap-1 rounded-md border border-[var(--lh-border)] bg-[var(--lh-surface)] px-2 py-1 text-xs font-medium text-[var(--lh-text-secondary)] hover:border-[var(--lh-border-strong)] hover:text-rose-600 dark:hover:text-rose-400 disabled:opacity-45 focus:outline-none focus:ring-2 focus:ring-[var(--lh-accent)]"
              >
                <Trash2 className="h-3.5 w-3.5" />
                <span>Clear</span>
              </button>
            </>
          ) : (
            <>
              <button
                type="button"
                onClick={onCopyClick}
                disabled={value.length === 0}
                data-testid="copy-button"
                className="inline-flex items-center gap-1 rounded-md border border-[var(--lh-border)] bg-[var(--lh-surface)] px-2.5 py-1 text-xs font-medium text-[var(--lh-text-secondary)] hover:border-[var(--lh-border-strong)] hover:text-[var(--lh-text)] disabled:opacity-45 focus:outline-none focus:ring-2 focus:ring-[var(--lh-accent)]"
              >
                {copied ? (
                  <>
                    <Check className="h-3.5 w-3.5 text-emerald-600 dark:text-emerald-400" />
                    <span className="text-emerald-700 dark:text-emerald-300 font-semibold">
                      Copied!
                    </span>
                  </>
                ) : (
                  <>
                    <Copy className="h-3.5 w-3.5" />
                    <span>Copy</span>
                  </>
                )}
              </button>

              <button
                type="button"
                onClick={handleSelectAll}
                disabled={value.length === 0}
                data-testid="select-all-button"
                className="inline-flex items-center gap-1 rounded-md border border-[var(--lh-border)] bg-[var(--lh-surface)] px-2 py-1 text-xs font-medium text-[var(--lh-text-secondary)] hover:border-[var(--lh-border-strong)] hover:text-[var(--lh-text)] disabled:opacity-45 focus:outline-none focus:ring-2 focus:ring-[var(--lh-accent)]"
              >
                <ListChecks className="h-3.5 w-3.5" />
                <span>Select All</span>
              </button>

              <button
                type="button"
                onClick={onSaveClick}
                disabled={value.length === 0}
                data-testid="save-button"
                className="inline-flex items-center gap-1 rounded-md border border-[var(--lh-border)] bg-[var(--lh-surface)] px-2 py-1 text-xs font-medium text-[var(--lh-text-secondary)] hover:border-[var(--lh-border-strong)] hover:text-[var(--lh-text)] disabled:opacity-45 focus:outline-none focus:ring-2 focus:ring-[var(--lh-accent)]"
              >
                <Download className="h-3.5 w-3.5" />
                <span>Save</span>
              </button>
            </>
          )}
        </div>
      </div>

      {/* Native Textarea for zero-freeze 5,000+ word responsiveness */}
      <textarea
        ref={textareaRef}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        readOnly={false}
        disabled={disabled}
        aria-label={title}
        data-testid={isInput ? "original-textarea" : "humanized-textarea"}
        placeholder={
          isInput
            ? "Paste or import your draft here (supports 3,000+ words, Markdown, citations, quotes, and UTF-8 text)…"
            : "Your locally paraphrased text will appear here. You can edit it freely before copying or saving…"
        }
        className="min-h-0 flex-1 w-full resize-none bg-transparent p-4 text-[15px] leading-relaxed text-[var(--lh-text)] placeholder:text-[var(--lh-text-muted)] focus:outline-none"
      />

      {/* Footer Counters */}
      <div className="flex shrink-0 items-center justify-between border-t border-[var(--lh-border)] bg-[var(--lh-surface-elevated)] px-3.5 py-1.5 text-[11px] text-[var(--lh-text-muted)]">
        <div className="flex items-center gap-3 tabular-nums">
          <span data-testid={isInput ? "original-word-count" : "humanized-word-count"}>
            <strong className="font-semibold text-[var(--lh-text-secondary)]">
              {wordCount.toLocaleString()}
            </strong>{" "}
            {wordCount === 1 ? "word" : "words"}
          </span>
          <span data-testid={isInput ? "original-char-count" : "humanized-char-count"}>
            <strong className="font-semibold text-[var(--lh-text-secondary)]">
              {charCount.toLocaleString()}
            </strong>{" "}
            {charCount === 1 ? "char" : "chars"}
          </span>
        </div>

        <span>{isInput ? "UTF-8 Input" : "Editable Output"}</span>
      </div>
    </div>
  );
}
