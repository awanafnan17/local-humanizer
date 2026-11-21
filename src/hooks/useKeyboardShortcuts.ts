import { useEffect } from "react";

export interface KeyboardShortcutsConfig {
  isSettingsOpen: boolean;
  isRunning: boolean;
  onHumanize: () => void;
  onCopyOutput: () => void;
  onSaveOutput: () => void;
  onCancelHumanize: () => void;
  onCloseSettings: () => void;
}

export function useKeyboardShortcuts(config: KeyboardShortcutsConfig): void {
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const mod = event.metaKey || event.ctrlKey;

      if (event.key === "Escape") {
        if (config.isSettingsOpen) {
          event.preventDefault();
          config.onCloseSettings();
          return;
        }
        if (config.isRunning) {
          event.preventDefault();
          config.onCancelHumanize();
        }
        return;
      }

      if (config.isSettingsOpen) return;

      if (!mod) return;

      if (event.key === "Enter" && !event.shiftKey && !event.altKey) {
        event.preventDefault();
        config.onHumanize();
        return;
      }

      if (event.shiftKey && event.key.toLowerCase() === "c" && !event.altKey) {
        event.preventDefault();
        config.onCopyOutput();
        return;
      }

      if (!event.shiftKey && event.key.toLowerCase() === "s" && !event.altKey) {
        event.preventDefault();
        config.onSaveOutput();
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [config]);
}
