import { AlertCircle, CheckCircle2, Info, X } from "lucide-react";

export interface ToastItem {
  id: string;
  kind: "info" | "success" | "error";
  message: string;
}

export interface ToastStackProps {
  toasts: readonly ToastItem[];
  onDismiss: (id: string) => void;
}

export function ToastStack({ toasts, onDismiss }: ToastStackProps) {
  if (toasts.length === 0) return null;

  return (
    <div
      className="fixed bottom-4 right-4 z-50 flex max-w-sm flex-col gap-2 pointer-events-none"
      aria-live="polite"
      aria-label="Notifications"
      data-testid="toast-stack"
    >
      {toasts.map((toast) => {
        const Icon =
          toast.kind === "error"
            ? AlertCircle
            : toast.kind === "success"
              ? CheckCircle2
              : Info;

        const accentClass =
          toast.kind === "error"
            ? "border-rose-500/40 bg-rose-50 text-rose-900 dark:bg-rose-950/90 dark:text-rose-100"
            : toast.kind === "success"
              ? "border-emerald-500/40 bg-emerald-50 text-emerald-900 dark:bg-emerald-950/90 dark:text-emerald-100"
              : "border-[var(--lh-border-strong)] bg-[var(--lh-surface)] text-[var(--lh-text)]";

        return (
          <div
            key={toast.id}
            role="status"
            data-testid={`toast-${toast.kind}`}
            className={`pointer-events-auto flex items-start gap-2.5 rounded-lg border px-3.5 py-2.5 text-xs shadow-lg ${accentClass}`}
          >
            <Icon className="mt-0.5 h-4 w-4 shrink-0" />
            <span className="flex-1 leading-relaxed">{toast.message}</span>
            <button
              type="button"
              onClick={() => onDismiss(toast.id)}
              aria-label="Dismiss notification"
              className="rounded p-0.5 opacity-70 hover:opacity-100 focus:outline-none focus:ring-2 focus:ring-[var(--lh-accent)]"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        );
      })}
    </div>
  );
}
