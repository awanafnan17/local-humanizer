import { useEffect, useState } from "react";
import { ShieldCheck } from "lucide-react";
import logoUrl from "./assets/logo.svg";
import { getAppStatus, isTauriRuntime } from "./lib/ipcClient";
import type { AppStatus } from "./lib/types";

export default function App() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);

  useEffect(() => {
    if (!isTauriRuntime()) {
      setStatus({
        state: "initializing",
        active_model_id: "pending-m2-selection",
        model_ready: false,
        backend_mode: "cpu",
        ram_bytes: 0,
        error_message: null,
        runtime_version: "b12600",
      });
      return;
    }

    let cancelled = false;
    getAppStatus()
      .then((res) => {
        if (!cancelled) {
          setStatus(res);
          setStatusError(null);
        }
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setStatusError(err instanceof Error ? err.message : String(err));
        }
      });

    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <div className="min-h-screen bg-slate-950 text-slate-100 flex flex-col">
      <header className="border-b border-slate-800 bg-slate-900/80 px-6 py-4 flex items-center justify-between">
        <div className="flex items-center gap-3">
          <img
            src={logoUrl}
            alt="Local Humanizer"
            className="h-9 w-9 rounded-lg"
          />
          <div>
            <h1 className="text-lg font-semibold tracking-tight text-white">
              Local Humanizer
            </h1>
            <p className="text-xs text-slate-400">
              Privacy-first offline AI writing &amp; paraphrasing
            </p>
          </div>
        </div>

        <div className="flex items-center gap-2 rounded-full border border-slate-700 bg-slate-800/80 px-3 py-1.5 text-xs text-slate-200">
          <ShieldCheck className="h-4 w-4 text-teal-400" />
          <span>
            {statusError
              ? `IPC Error: ${statusError}`
              : `Runtime ${status?.runtime_version ?? "b12600"} · ${status?.state ?? "initializing"}`}
          </span>
        </div>
      </header>

      <main className="flex-1 mx-auto w-full max-w-5xl px-6 py-10 flex flex-col gap-6">
        <section className="rounded-xl border border-slate-800 bg-slate-900/60 p-6">
          <h2 className="text-base font-medium text-white mb-2">
            Offline Desktop Shell Ready
          </h2>
          <p className="text-sm text-slate-300 leading-relaxed">
            Local Humanizer runs 100% locally on your computer with zero cloud
            APIs, zero accounts, and zero telemetry. Full model management and
            the two-pane paraphrasing workspace are wired cleanly via native
            Tauri v2 IPC.
          </p>
        </section>
      </main>
    </div>
  );
}
