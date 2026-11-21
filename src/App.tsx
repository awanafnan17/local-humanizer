import { useCallback, useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import { ControlsBar } from "./components/ControlsBar.tsx";
import { EditorPane } from "./components/EditorPane.tsx";
import { Header } from "./components/Header.tsx";
import { ProgressBanner } from "./components/ProgressBanner.tsx";
import { SettingsModal } from "./components/SettingsModal.tsx";
import { ToastStack, type ToastItem } from "./components/Toast.tsx";
import { useAppStatus } from "./hooks/useAppStatus.ts";
import { useHumanizer } from "./hooks/useHumanizer.ts";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts.ts";
import { useTheme } from "./hooks/useTheme.ts";
import { canStartHumanize } from "./lib/humanizeState.ts";
import {
  importTextFile,
  isTauriRuntime,
  pickOpenTextFile,
  pickSaveOutputFile,
  saveOutputFile,
} from "./lib/ipcClient.ts";
import {
  sanitizeUserFacingError,
  validateImportContent,
  validateImportFilename,
} from "./lib/validators.ts";
import { countWords } from "./lib/wordCount.ts";

export default function App() {
  const { theme, toggleTheme } = useTheme();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const [copiedOutput, setCopiedOutput] = useState(false);
  const copyTimerRef = useRef<number | null>(null);

  const dismissToast = useCallback((id: string) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const notify = useCallback(
    (kind: "info" | "success" | "error", message: string) => {
      const id = `toast-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
      setToasts((prev) => [...prev.slice(-3), { id, kind, message }]);
      window.setTimeout(() => {
        setToasts((prev) => prev.filter((t) => t.id !== id));
      }, 4200);
    },
    [],
  );

  useEffect(() => {
    return () => {
      if (copyTimerRef.current !== null) {
        window.clearTimeout(copyTimerRef.current);
      }
    };
  }, []);

  const {
    appStatus,
    statusError,
    models,
    modelsError,
    downloadProgress,
    isSwitchingModel,
    isRetryingInit,
    refreshModels,
    retryRuntimeInit,
    selectModel,
    downloadModel,
    cancelDownload,
  } = useAppStatus({ onNotify: notify });

  const {
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
  } = useHumanizer({ onNotify: notify });

  const deferredInput = useDeferredValue(inputText);
  const inputWordCount = useMemo(() => countWords(deferredInput), [deferredInput]);

  const humanizeAllowed = useMemo(
    () => canStartHumanize(appStatus, inputText, runState.isRunning),
    [appStatus, inputText, runState.isRunning],
  );

  const handlePaste = useCallback(async () => {
    try {
      if (!navigator.clipboard || typeof navigator.clipboard.readText !== "function") {
        notify("info", "Use Cmd/Ctrl+V inside Original Text to paste.");
        return;
      }
      const clipText = await navigator.clipboard.readText();
      if (!clipText) {
        notify("info", "Clipboard is empty.");
        return;
      }
      setInputText(clipText);
    } catch {
      notify(
        "info",
        "Clipboard read permission denied. Paste directly with Cmd/Ctrl+V.",
      );
    }
  }, [notify, setInputText]);

  const handleBrowserFileSelected = useCallback(
    async (file: File) => {
      const nameCheck = validateImportFilename(file.name);
      if (!nameCheck.valid) {
        notify("error", nameCheck.error ?? "Unsupported file type.");
        return;
      }
      try {
        const bytes = new Uint8Array(await file.arrayBuffer());
        let text: string;
        try {
          text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
        } catch {
          notify("error", "File must be valid UTF-8 plain text or Markdown.");
          return;
        }
        const contentCheck = validateImportContent(text, bytes.byteLength);
        if (!contentCheck.valid) {
          notify("error", contentCheck.error ?? "Invalid file content.");
          return;
        }
        setInputText(text);
        notify("success", "Text file imported.");
      } catch (err) {
        notify("error", sanitizeUserFacingError(err));
      }
    },
    [notify, setInputText],
  );

  const useNativeOrMockOpenDialog =
    isTauriRuntime() ||
    (typeof window !== "undefined" &&
      Boolean(window.__LOCAL_HUMANIZER_MOCK_IPC__?.openDialog));

  const handleImportDialog = useCallback(async () => {
    try {
      const path = await pickOpenTextFile();
      if (!path) return;
      const nameCheck = validateImportFilename(path);
      if (!nameCheck.valid) {
        notify("error", nameCheck.error ?? "Unsupported file type.");
        return;
      }
      const content = await importTextFile(path);
      const contentCheck = validateImportContent(content);
      if (!contentCheck.valid) {
        notify("error", contentCheck.error ?? "Invalid file content.");
        return;
      }
      setInputText(content);
      notify("success", "Text file imported.");
    } catch (err) {
      notify("error", sanitizeUserFacingError(err));
    }
  }, [notify, setInputText]);

  const handleCopyOutput = useCallback(async () => {
    if (!outputText) return;
    try {
      let ok = false;
      if (navigator.clipboard && typeof navigator.clipboard.writeText === "function") {
        await navigator.clipboard.writeText(outputText);
        ok = true;
      } else if (typeof document !== "undefined") {
        const helper = document.createElement("textarea");
        helper.value = outputText;
        helper.setAttribute("readonly", "");
        helper.style.position = "fixed";
        helper.style.opacity = "0";
        document.body.appendChild(helper);
        helper.select();
        ok = document.execCommand("copy");
        document.body.removeChild(helper);
      }

      if (ok) {
        setCopiedOutput(true);
        if (copyTimerRef.current !== null) {
          window.clearTimeout(copyTimerRef.current);
        }
        copyTimerRef.current = window.setTimeout(() => {
          setCopiedOutput(false);
          copyTimerRef.current = null;
        }, 2000);
      } else {
        notify("error", "Unable to write to clipboard.");
      }
    } catch {
      notify("error", "Clipboard write permission denied.");
    }
  }, [notify, outputText]);

  const handleSaveOutput = useCallback(async () => {
    if (!outputText) return;
    const useNativeOrMockSave =
      isTauriRuntime() ||
      (typeof window !== "undefined" &&
        Boolean(window.__LOCAL_HUMANIZER_MOCK_IPC__?.saveDialog));

    if (useNativeOrMockSave) {
      try {
        const path = await pickSaveOutputFile("humanized-output.txt");
        if (!path) return;
        await saveOutputFile(path, outputText);
        notify("success", "Saved humanized text.");
      } catch (err) {
        notify("error", sanitizeUserFacingError(err));
      }
      return;
    }

    try {
      const blob = new Blob([outputText], { type: "text/plain;charset=utf-8" });
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = "humanized-output.txt";
      document.body.appendChild(anchor);
      anchor.click();
      document.body.removeChild(anchor);
      window.setTimeout(() => URL.revokeObjectURL(url), 1000);
      notify("success", "Downloaded humanized-output.txt.");
    } catch (err) {
      notify("error", sanitizeUserFacingError(err));
    }
  }, [notify, outputText]);

  const shortcutConfig = useMemo(
    () => ({
      isSettingsOpen: settingsOpen,
      isRunning: runState.isRunning,
      onHumanize: () => {
        if (humanizeAllowed) {
          void startHumanize();
        }
      },
      onCopyOutput: () => {
        void handleCopyOutput();
      },
      onSaveOutput: () => {
        void handleSaveOutput();
      },
      onCancelHumanize: () => {
        void cancelActiveHumanize();
      },
      onCloseSettings: () => setSettingsOpen(false),
    }),
    [
      cancelActiveHumanize,
      handleCopyOutput,
      handleSaveOutput,
      humanizeAllowed,
      runState.isRunning,
      settingsOpen,
      startHumanize,
    ],
  );

  useKeyboardShortcuts(shortcutConfig);

  return (
    <div className="flex min-h-screen flex-col md:h-screen md:overflow-hidden">
      <Header
        appStatus={appStatus}
        statusError={statusError}
        models={models}
        theme={theme}
        isRetryingInit={isRetryingInit}
        onToggleTheme={toggleTheme}
        onOpenSettings={() => setSettingsOpen(true)}
        onRetryInit={() => void retryRuntimeInit()}
      />

      <ControlsBar
        mode={mode}
        intensity={intensity}
        options={options}
        canHumanize={humanizeAllowed}
        isRunning={runState.isRunning}
        modelReady={Boolean(appStatus?.model_ready)}
        onModeChange={setMode}
        onIntensityChange={setIntensity}
        onOptionChange={updateOption}
        onHumanize={() => void startHumanize()}
        onCancel={() => void cancelActiveHumanize()}
      />

      <main className="mx-auto flex min-h-0 w-full max-w-[1600px] flex-1 flex-col gap-3 p-4 sm:px-6">
        <ProgressBanner
          runState={runState}
          elapsedMs={elapsedMs}
          isRetryingChunk={isRetryingChunk}
          onRetryChunk={(chunkIdx) => void retryFlaggedChunk(chunkIdx)}
        />

        <div className="grid min-h-0 flex-1 grid-cols-1 gap-4 md:grid-cols-2">
          <EditorPane
            variant="input"
            value={inputText}
            onChange={setInputText}
            disabled={runState.isRunning}
            onPasteClick={() => void handlePaste()}
            onClearClick={clearInput}
            onImportFileClick={
              useNativeOrMockOpenDialog
                ? () => void handleImportDialog()
                : undefined
            }
            onBrowserFileSelected={(file) => void handleBrowserFileSelected(file)}
          />

          <EditorPane
            variant="output"
            value={outputText}
            onChange={setOutputText}
            comparisonWordCount={inputWordCount}
            copied={copiedOutput}
            onCopyClick={() => void handleCopyOutput()}
            onSaveClick={() => void handleSaveOutput()}
          />
        </div>
      </main>

      <SettingsModal
        isOpen={settingsOpen}
        appStatus={appStatus}
        models={models}
        modelsError={modelsError}
        downloadProgress={downloadProgress}
        isSwitchingModel={isSwitchingModel}
        onClose={() => setSettingsOpen(false)}
        onRefreshModels={() => void refreshModels()}
        onSelectModel={(modelId, useGpu) => void selectModel(modelId, useGpu)}
        onDownloadModel={(modelId) => void downloadModel(modelId)}
        onCancelDownload={(modelId) => void cancelDownload(modelId)}
      />

      <ToastStack toasts={toasts} onDismiss={dismissToast} />
    </div>
  );
}
