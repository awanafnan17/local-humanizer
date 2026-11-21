#!/usr/bin/env python3
"""Reproducible M2 Model Benchmark Harness for Local Humanizer.

Pure Python 3.14 standard-library benchmark suite that sequentially downloads,
SHA256-verifies, benchmarks (CPU-only `-ngl 0` and Apple Silicon Metal `-ngl 99`),
and cleans up candidate GGUF models one at a time to respect tight disk budgets.

Telemetry uses macOS `proc_pid_rusage` (`RUSAGE_INFO_V4` via `ctypes`),
`os.wait4` (`ru_maxrss`, `ru_utime`, `ru_stime`, `ru_majflt`), and
`sysctl vm.swapusage` so it works reliably with or without `/bin/ps`.
"""

from __future__ import annotations

import argparse
from collections import Counter
import csv
import ctypes
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import socket
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from typing import Any


REPO_ROOT = Path(__file__).resolve().parent.parent
TASKS_PATH = Path(__file__).resolve().parent / "tasks.json"
TMP_DIR = Path(__file__).resolve().parent / "tmp"
RESULTS_DIR = Path(__file__).resolve().parent / "results"
MODELS_DIR = REPO_ROOT / "src-tauri" / "resources" / "models"
RUNTIME_DIR = REPO_ROOT / "src-tauri" / "resources" / "runtime" / "macos-arm64"
LLAMA_SERVER_BIN = RUNTIME_DIR / "llama-server"

DISK_HEADROOM_BYTES = 2_000_000_000  # Keep >= 2 GB free before downloading any GGUF.

CANDIDATES: list[dict[str, Any]] = [
    {
        "id": "qwen3-1.7b-q4_k_m",
        "name": "Qwen3 1.7B",
        "repo_id": "unsloth/Qwen3-1.7B-GGUF",
        "filename": "Qwen3-1.7B-Q4_K_M.gguf",
        "download_url": "https://huggingface.co/unsloth/Qwen3-1.7B-GGUF/resolve/main/Qwen3-1.7B-Q4_K_M.gguf",
        "quant": "Q4_K_M",
        "size_bytes": 1107409472,
        "sha256": "b139949c5bd74937ad8ed8c8cf3d9ffb1e99c866c823204dc42c0d91fa181897",
        "license": "Apache-2.0",
        "license_url": "https://huggingface.co/Qwen/Qwen3-1.7B/blob/main/LICENSE",
        "family": "qwen3",
        "min_recommended_ram_gb": 8,
        "non_commercial_warning": False,
        "disable_thinking": True,
        "fold_system_into_user": False,
        "ctx_size": 4096,
    },
    {
        "id": "qwen3-4b-instruct-2507-q4_k_m",
        "name": "Qwen3 4B Instruct 2507",
        "repo_id": "unsloth/Qwen3-4B-Instruct-2507-GGUF",
        "filename": "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        "download_url": "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/main/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        "quant": "Q4_K_M",
        "size_bytes": 2497281120,
        "sha256": "3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597",
        "license": "Apache-2.0",
        "license_url": "https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507/blob/main/LICENSE",
        "family": "qwen3",
        "min_recommended_ram_gb": 16,
        "non_commercial_warning": False,
        "disable_thinking": False,
        "fold_system_into_user": False,
        "ctx_size": 4096,
    },
    {
        "id": "gemma-3-1b-it-q4_k_m",
        "name": "Gemma 3 1B IT",
        "repo_id": "ggml-org/gemma-3-1b-it-GGUF",
        "filename": "gemma-3-1b-it-Q4_K_M.gguf",
        "download_url": "https://huggingface.co/ggml-org/gemma-3-1b-it-GGUF/resolve/main/gemma-3-1b-it-Q4_K_M.gguf",
        "quant": "Q4_K_M",
        "size_bytes": 806058240,
        "sha256": "8ccc5cd1f1b3602548715ae25a66ed73fd5dc68a210412eea643eb20eb75a135",
        "license": "Gemma Terms of Use",
        "license_url": "https://ai.google.dev/gemma/terms",
        "family": "gemma3",
        "min_recommended_ram_gb": 4,
        "non_commercial_warning": False,
        "disable_thinking": False,
        "fold_system_into_user": True,
        "ctx_size": 4096,
    },
    {
        "id": "qwen2.5-3b-instruct-q4_k_m",
        "name": "Qwen2.5 3B Instruct",
        "repo_id": "Qwen/Qwen2.5-3B-Instruct-GGUF",
        "filename": "qwen2.5-3b-instruct-q4_k_m.gguf",
        "download_url": "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf",
        "quant": "Q4_K_M",
        "size_bytes": 2104932768,
        "sha256": "626b4a6678b86442240e33df819e00132d3ba7dddfe1cdc4fbb18e0a9615c62d",
        "license": "Qwen Research License Agreement",
        "license_url": "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/blob/main/LICENSE",
        "family": "qwen2.5",
        "min_recommended_ram_gb": 16,
        "non_commercial_warning": True,
        "disable_thinking": False,
        "fold_system_into_user": False,
        "ctx_size": 4096,
    },
    {
        "id": "qwen2.5-1.5b-instruct-q4_k_m",
        "name": "Qwen2.5 1.5B Instruct",
        "repo_id": "Qwen/Qwen2.5-1.5B-Instruct-GGUF",
        "filename": "qwen2.5-1.5b-instruct-q4_k_m.gguf",
        "download_url": "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf",
        "quant": "Q4_K_M",
        "size_bytes": 1117320736,
        "sha256": "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e",
        "license": "Apache-2.0",
        "license_url": "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/blob/main/LICENSE",
        "family": "qwen2.5",
        "min_recommended_ram_gb": 8,
        "non_commercial_warning": False,
        "disable_thinking": False,
        "fold_system_into_user": False,
        "ctx_size": 4096,
    },
    {
        "id": "smollm2-1.7b-instruct-q4_k_m",
        "name": "SmolLM2 1.7B Instruct",
        "repo_id": "bartowski/SmolLM2-1.7B-Instruct-GGUF",
        "filename": "SmolLM2-1.7B-Instruct-Q4_K_M.gguf",
        "download_url": "https://huggingface.co/bartowski/SmolLM2-1.7B-Instruct-GGUF/resolve/main/SmolLM2-1.7B-Instruct-Q4_K_M.gguf",
        "quant": "Q4_K_M",
        "size_bytes": 1055609824,
        "sha256": "77665ea4815999596525c636fbeb56ba8b080b46ae85efef4f0d986a139834d7",
        "license": "Apache-2.0",
        "license_url": "https://www.apache.org/licenses/LICENSE-2.0",
        "family": "smollm2",
        "min_recommended_ram_gb": 8,
        "non_commercial_warning": False,
        "disable_thinking": False,
        "fold_system_into_user": False,
        "ctx_size": 4096,
    },
]

CANDIDATE_MAP = {c["id"]: c for c in CANDIDATES}

_THINK_OPEN_RAW = (
    r"(?:<\s*think\s*>|&lt;\s*think\s*&gt;|"
    r"&#0*60;\s*think\s*&#0*62;|&#x0*3c;\s*think\s*&#x0*3e;|"
    r"\\u003c\s*think\s*\\u003e)"
)
_THINK_CLOSE_RAW = (
    r"(?:<\s*/\s*think\s*>|&lt;\s*/\s*think\s*&gt;|"
    r"&#0*60;\s*/\s*think\s*&#0*62;|&#x0*3c;\s*/\s*think\s*&#x0*3e;|"
    r"\\u003c\s*/\s*think\s*\\u003e)"
)
_THINK_RE = re.compile(
    r"(?:<\s*/?\s*think\s*>|&lt;\s*/?\s*think\s*&gt;|"
    r"&#0*60;\s*/?\s*think\s*&#0*62;|&#x0*3c;\s*/?\s*think\s*&#x0*3e;|"
    r"\\u003c\s*/?\s*think\s*\\u003e)",
    re.IGNORECASE,
)
_THINK_BLOCK_RE = re.compile(rf"{_THINK_OPEN_RAW}[\s\S]*?{_THINK_CLOSE_RAW}", re.IGNORECASE)
_THINK_UNCLOSED_RE = re.compile(rf"{_THINK_OPEN_RAW}[\s\S]*$", re.IGNORECASE)
_SENTENCE_END_RE = re.compile(r"(?<=[.!?])\s+")


# --- macOS proc_pid_rusage (RUSAGE_INFO_V4) ---
class RusageInfoV4(ctypes.Structure):
    _fields_ = [
        ("ri_uuid", ctypes.c_uint8 * 16),
        ("ri_user_time", ctypes.c_uint64),
        ("ri_system_time", ctypes.c_uint64),
        ("ri_pkg_idle_wkups", ctypes.c_uint64),
        ("ri_interrupt_wkups", ctypes.c_uint64),
        ("ri_pageins", ctypes.c_uint64),
        ("ri_wired_size", ctypes.c_uint64),
        ("ri_resident_size", ctypes.c_uint64),
        ("ri_phys_footprint", ctypes.c_uint64),
        ("ri_proc_start_abstime", ctypes.c_uint64),
        ("ri_proc_exit_abstime", ctypes.c_uint64),
        ("ri_child_user_time", ctypes.c_uint64),
        ("ri_child_system_time", ctypes.c_uint64),
        ("ri_child_pkg_idle_wkups", ctypes.c_uint64),
        ("ri_child_interrupt_wkups", ctypes.c_uint64),
        ("ri_child_pageins", ctypes.c_uint64),
        ("ri_child_elapsed_abstime", ctypes.c_uint64),
        ("ri_diskio_bytesread", ctypes.c_uint64),
        ("ri_diskio_byteswritten", ctypes.c_uint64),
        ("ri_cpu_time_qos_default", ctypes.c_uint64),
        ("ri_cpu_time_qos_maintenance", ctypes.c_uint64),
        ("ri_cpu_time_qos_background", ctypes.c_uint64),
        ("ri_cpu_time_qos_utility", ctypes.c_uint64),
        ("ri_cpu_time_qos_legacy", ctypes.c_uint64),
        ("ri_cpu_time_qos_user_initiated", ctypes.c_uint64),
        ("ri_cpu_time_qos_user_interactive", ctypes.c_uint64),
        ("ri_billed_system_time", ctypes.c_uint64),
        ("ri_serviced_system_time", ctypes.c_uint64),
        ("ri_logical_writes", ctypes.c_uint64),
        ("ri_lifetime_max_phys_footprint", ctypes.c_uint64),
        ("ri_instructions", ctypes.c_uint64),
        ("ri_cycles", ctypes.c_uint64),
        ("ri_billed_energy", ctypes.c_uint64),
        ("ri_serviced_energy", ctypes.c_uint64),
        ("ri_interval_max_phys_footprint", ctypes.c_uint64),
        ("ri_runnable_time", ctypes.c_uint64),
    ]


_LIBSYSTEM = None
if sys.platform == "darwin":
    try:
        _LIBSYSTEM = ctypes.CDLL("/usr/lib/libSystem.B.dylib")
        _LIBSYSTEM.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.POINTER(RusageInfoV4)]
        _LIBSYSTEM.proc_pid_rusage.restype = ctypes.c_int
    except OSError:
        _LIBSYSTEM = None


def sample_proc_rusage(pid: int) -> dict[str, int] | None:
    if _LIBSYSTEM is None:
        return None
    info = RusageInfoV4()
    if _LIBSYSTEM.proc_pid_rusage(pid, 4, ctypes.byref(info)) != 0:
        return None
    return {
        "resident_bytes": int(info.ri_resident_size),
        "phys_footprint_bytes": int(info.ri_phys_footprint),
        "lifetime_max_phys_footprint_bytes": int(info.ri_lifetime_max_phys_footprint),
        "pageins": int(info.ri_pageins),
    }


def read_swap_usage_mb() -> float | None:
    if sys.platform != "darwin":
        return None
    try:
        out = subprocess.check_output(["sysctl", "-n", "vm.swapusage"], text=True, timeout=5)
        m = re.search(r"used\s*=\s*([0-9.]+)([MG])", out)
        if not m:
            return None
        val = float(m.group(1))
        return val * 1024.0 if m.group(2) == "G" else val
    except Exception:
        return None


def collect_host_info() -> dict[str, Any]:
    def sysctl_val(key: str) -> str:
        try:
            return subprocess.check_output(["sysctl", "-n", key], text=True, timeout=5).strip()
        except Exception:
            return "unknown"

    sw_vers = "unknown"
    try:
        sw_vers = subprocess.check_output(["sw_vers"], text=True, timeout=5).strip().replace("\n", "; ")
    except Exception:
        pass

    llama_ver = "unknown"
    try:
        llama_ver = subprocess.check_output([str(LLAMA_SERVER_BIN), "--version"], text=True, timeout=5, stderr=subprocess.STDOUT).strip().splitlines()[0]
    except Exception:
        pass

    return {
        "platform": sys.platform,
        "cpu_brand": sysctl_val("machdep.cpu.brand_string"),
        "hw_model": sysctl_val("hw.model"),
        "logical_cpus": sysctl_val("hw.ncpu"),
        "perf_cores": sysctl_val("hw.perflevel0.physicalcpu"),
        "eff_cores": sysctl_val("hw.perflevel1.physicalcpu"),
        "mem_bytes": int(sysctl_val("hw.memsize")) if sysctl_val("hw.memsize").isdigit() else 0,
        "os_version": sw_vers,
        "llama_server_version": llama_ver,
        "disk_free_bytes": shutil.disk_usage(REPO_ROOT).free,
    }


def pick_ephemeral_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return int(s.getsockname()[1])


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        while True:
            chunk = f.read(1024 * 1024)
            if not chunk:
                break
            h.update(chunk)
    return h.hexdigest()


def download_verify_gguf(candidate: dict[str, Any], skip_download: bool) -> Path:
    TMP_DIR.mkdir(parents=True, exist_ok=True)
    final_path = TMP_DIR / candidate["filename"]
    bundled_path = MODELS_DIR / candidate["filename"]

    if skip_download:
        for p in (final_path, bundled_path):
            if p.exists():
                digest = sha256_file(p)
                if p.stat().st_size == candidate["size_bytes"] and digest == candidate["sha256"]:
                    print(f"[reuse] Verified existing {p} ({digest[:12]}...)")
                    return p
        raise FileNotFoundError(f"--skip-download set but verified {candidate['filename']} not found")

    if final_path.exists():
        if final_path.stat().st_size == candidate["size_bytes"] and sha256_file(final_path) == candidate["sha256"]:
            print(f"[cache] Reusing verified {final_path}")
            return final_path
        final_path.unlink()

    free_bytes = shutil.disk_usage(TMP_DIR).free
    required = candidate["size_bytes"] + DISK_HEADROOM_BYTES
    if free_bytes < required:
        raise RuntimeError(
            f"Insufficient disk space for {candidate['id']}: free={free_bytes:,} bytes < required={required:,} bytes "
            f"(model {candidate['size_bytes']:,} + headroom {DISK_HEADROOM_BYTES:,})"
        )

    part_path = TMP_DIR / f"{candidate['filename']}.part"
    url = candidate["download_url"]
    expected_size = candidate["size_bytes"]
    print(f"[download] {candidate['id']} ({expected_size / 1048576:.1f} MiB) from {url}")

    for attempt in range(1, 4):
        existing = part_path.stat().st_size if part_path.exists() else 0
        if existing > expected_size:
            part_path.unlink()
            existing = 0

        headers = {"User-Agent": "LocalHumanizer-M2-Benchmark/1.0"}
        if existing > 0:
            headers["Range"] = f"bytes={existing}-"

        req = urllib.request.Request(url, headers=headers)
        try:
            t0 = time.perf_counter()
            with urllib.request.urlopen(req, timeout=60) as resp:
                status = getattr(resp, "status", 200)
                mode = "ab" if (existing > 0 and status == 206) else "wb"
                if mode == "wb":
                    existing = 0
                downloaded = existing
                last_log = time.monotonic()
                with part_path.open(mode) as f:
                    while True:
                        buf = resp.read(1024 * 1024)
                        if not buf:
                            break
                        f.write(buf)
                        downloaded += len(buf)
                        now = time.monotonic()
                        if now - last_log >= 5.0:
                            pct = (downloaded / expected_size) * 100.0
                            print(f"  ... {downloaded / 1048576:.1f} / {expected_size / 1048576:.1f} MiB ({pct:.1f}%)", flush=True)
                            last_log = now
            dt = time.perf_counter() - t0
            print(f"[download] Finished transfer in {dt:.1f}s; verifying SHA256...", flush=True)
            actual_size = part_path.stat().st_size
            actual_sha = sha256_file(part_path)
            if actual_size != expected_size:
                if part_path.exists():
                    part_path.unlink()
                raise RuntimeError(f"Size mismatch: got {actual_size}, expected {expected_size}")
            if actual_sha != candidate["sha256"]:
                if part_path.exists():
                    part_path.unlink()
                raise RuntimeError(f"SHA256 mismatch: got {actual_sha}, expected {candidate['sha256']}")
            os.replace(part_path, final_path)
            print(f"[verify] PASS size={actual_size} sha256={actual_sha}")
            return final_path
        except Exception as exc:
            print(f"[download] Attempt {attempt}/3 failed: {exc}", flush=True)
            if attempt == 3:
                if part_path.exists():
                    part_path.unlink()
                raise
            time.sleep(2 * attempt)

    raise RuntimeError("Download loop exited unexpectedly")


def build_prompts(
    task: dict[str, Any],
    text_to_rewrite: str,
    prev_context: str | None = None,
    strict_retry: bool = False,
    anti_echo_retry: bool = False,
    disable_thinking: bool = False,
) -> tuple[str, str]:
    mode = task.get("mode", "standard")
    intensity = task.get("intensity", "balanced")
    opts = task.get("options", {})

    mode_guides = {
        "standard": "Clear, natural, well-edited human prose that improves flow and sentence variety while remaining faithful to the source.",
        "natural": "Warm, idiomatic, conversational human prose with varied rhythm, natural transitions, and zero stiff academic filler.",
        "academic": "Formal scholarly academic register with precise terminology, objective analytical tone, and rigorous prose.",
        "professional": "Crisp, authoritative executive business communication suitable for governance briefs and technical leadership.",
        "simple": "Plain, direct, accessible everyday English with short clear sentences while keeping every required rule, fee, date, statute, and link intact.",
        "deep": "Thorough structural paraphrasing that recasts sentence architecture and phrasing cleanly without changing any underlying facts.",
    }
    intensity_guides = {
        "light": "Light paraphrase: make gentle clarity and phrasing refinements while keeping structure close to the original.",
        "balanced": "Balanced paraphrase: rephrase sentences naturally and vary syntax while preserving paragraph layout and length.",
        "strong": "Strong paraphrase: substantially recast wording and sentence structure to fit the target register while preserving every fact.",
    }

    rules = [
        f"Target mode: {mode} — {mode_guides.get(mode, mode_guides['standard'])}",
        f"Target intensity: {intensity} — {intensity_guides.get(intensity, intensity_guides['balanced'])}",
        f"Voice preference: {opts.get('voice', 'third_person')}",
        "Strictly preserve ALL factual meaning, numbers, percentages, statistics, p-values, currencies, dates, named entities, organizations, places, statute/section numbers, URLs, DOIs, inline backtick code spans, APA/IEEE citations, and exact verbatim double-quoted strings.",
        "Do NOT summarize, omit sentences, invent citations/facts, or add preamble/explanations/notes.",
        "Preserve the exact number of paragraphs (separated by blank lines) and keep output length roughly comparable to the input.",
        "Output ONLY the rewritten text.",
    ]
    if strict_retry:
        rules.append("CRITICAL: Your previous draft dropped or altered required numbers, citations, quotes, URLs, or length. Double-check every citation, URL, DOI, number, currency, date, backtick code token, and quoted sentence verbatim.")
    if anti_echo_retry:
        rules.append("CRITICAL ANTI-COPY INSTRUCTION: Your previous draft echoed the source text too closely. Substantially rephrase sentence structure, vocabulary, and clause ordering throughout EVERY paragraph while still preserving all required citations, URLs, DOIs, numbers, currencies, dates, backtick code spans, and exact verbatim double-quoted strings.")

    if prev_context:
        rules.append(f"Continuity reference from the preceding section (match its tense and register ONLY; NEVER repeat or copy these sentences into your output): {prev_context}")
    system_prompt = "You are Local Humanizer, an offline writing and paraphrasing assistant.\n" + "\n".join(f"- {r}" for r in rules)
    user_prompt = f"Rewrite the following text according to the instructions. Return only the rewritten text:\n\n{text_to_rewrite}"
    if disable_thinking:
        # Defense-in-depth Qwen3 soft switch paired with chat_template_kwargs enable_thinking=False.
        user_prompt = f"{user_prompt}\n\n/no_think"

    return system_prompt, user_prompt


def build_chat_messages(candidate: dict[str, Any], system_prompt: str, user_prompt: str) -> list[dict[str, str]]:
    if candidate.get("fold_system_into_user"):
        return [{"role": "user", "content": f"{system_prompt}\n\n{user_prompt}"}]
    return [
        {"role": "system", "content": system_prompt},
        {"role": "user", "content": user_prompt},
    ]


class LlamaServerRunner:
    def __init__(self, model_path: Path, candidate: dict[str, Any], mode: str):
        self.model_path = model_path
        self.candidate = candidate
        self.mode = mode  # "cpu" or "metal"
        self.port = pick_ephemeral_port()
        self.api_key = secrets.token_hex(24)
        self.proc: subprocess.Popen[str] | None = None
        self.samples: list[dict[str, int]] = []
        self._stop_sampling = threading.Event()
        self._sampler_thread: threading.Thread | None = None
        self.start_monotonic: float = 0.0
        self.load_ms: float = 0.0
        self.swap_before_mb: float | None = None
        self.swap_after_mb: float | None = None

    def start(self, ready_timeout_s: float = 120.0) -> None:
        ngl = "0" if self.mode == "cpu" else "99"
        cmd = [
            str(LLAMA_SERVER_BIN),
            "--host", "127.0.0.1",
            "--port", str(self.port),
            "--api-key", self.api_key,
            "--model", str(self.model_path),
            "--ctx-size", str(self.candidate.get("ctx_size", 4096)),
            "--parallel", "1",
            "--threads", "4",
            "--n-gpu-layers", ngl,
            "--no-webui",
        ]
        if self.candidate.get("disable_thinking"):
            cmd.extend([
                "--reasoning", "off",
                "--reasoning-budget", "0",
                "--chat-template-kwargs", '{"enable_thinking":false}',
            ])

        env = os.environ.copy()
        env["DYLD_LIBRARY_PATH"] = str(RUNTIME_DIR)
        self.swap_before_mb = read_swap_usage_mb()
        self.start_monotonic = time.perf_counter()

        self.proc = subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            env=env,
        )

        self._sampler_thread = threading.Thread(target=self._poll_rusage, daemon=True)
        self._sampler_thread.start()

        health_url = f"http://127.0.0.1:{self.port}/health"
        deadline = time.perf_counter() + ready_timeout_s
        while time.perf_counter() < deadline:
            if self.proc.poll() is not None:
                out = self.proc.stdout.read() if self.proc.stdout else ""
                raise RuntimeError(f"llama-server exited early (code={self.proc.returncode}): {out[-1200:]}")
            try:
                req = urllib.request.Request(
                    health_url,
                    headers={"Authorization": f"Bearer {self.api_key}"},
                )
                with urllib.request.urlopen(req, timeout=2.0) as resp:
                    body = resp.read().decode("utf-8", errors="replace")
                    if resp.status == 200 and '"ok"' in body:
                        self.load_ms = (time.perf_counter() - self.start_monotonic) * 1000.0
                        return
            except Exception:
                pass
            time.sleep(0.25)

        raise TimeoutError(f"llama-server /health did not become ready within {ready_timeout_s}s")

    def _poll_rusage(self) -> None:
        while not self._stop_sampling.is_set() and self.proc is not None:
            s = sample_proc_rusage(self.proc.pid)
            if s is not None:
                self.samples.append(s)
            self._stop_sampling.wait(0.25)

    def chat_completion(
        self,
        messages: list[dict[str, str]],
        max_tokens: int,
        temperature: float = 0.45,
        top_p: float = 0.85,
        timeout_s: float = 300.0,
    ) -> dict[str, Any]:
        url = f"http://127.0.0.1:{self.port}/v1/chat/completions"
        payload: dict[str, Any] = {
            "messages": messages,
            "temperature": temperature,
            "top_p": top_p,
            "top_k": 20,
            "repeat_penalty": 1.08,
            "max_tokens": max_tokens,
            "seed": 42,
            "stream": False,
        }
        if self.candidate.get("disable_thinking"):
            payload["chat_template_kwargs"] = {"enable_thinking": False}

        data = json.dumps(payload).encode("utf-8")
        req = urllib.request.Request(
            url,
            data=data,
            headers={
                "Content-Type": "application/json",
                "Authorization": f"Bearer {self.api_key}",
            },
            method="POST",
        )
        t0 = time.perf_counter()
        with urllib.request.urlopen(req, timeout=timeout_s) as resp:
            raw = resp.read().decode("utf-8")
        wall_ms = (time.perf_counter() - t0) * 1000.0
        parsed = json.loads(raw)
        parsed["_client_wall_ms"] = wall_ms
        return parsed

    def stop(self) -> dict[str, Any]:
        self.swap_after_mb = read_swap_usage_mb()
        self._stop_sampling.set()
        if self._sampler_thread is not None:
            self._sampler_thread.join(timeout=2.0)

        wall_total_s = max(0.001, time.perf_counter() - self.start_monotonic)
        ru_stats: dict[str, Any] = {
            "server_wall_sec": round(wall_total_s, 3),
            "model_load_ms": round(self.load_ms, 1),
            "swap_before_mb": self.swap_before_mb,
            "swap_after_mb": self.swap_after_mb,
            "swap_delta_mb": (
                round(self.swap_after_mb - self.swap_before_mb, 2)
                if (self.swap_before_mb is not None and self.swap_after_mb is not None)
                else None
            ),
        }

        if self.proc is not None:
            pid = self.proc.pid
            wait4_maxrss = 0
            wait4_utime = 0.0
            wait4_stime = 0.0
            wait4_majflt = 0
            wait4_nswap = 0
            reaped = False

            if self.proc.returncode is None:
                try:
                    self.proc.terminate()
                except OSError:
                    pass
                deadline = time.perf_counter() + 5.0
                while time.perf_counter() < deadline:
                    try:
                        wpid, status, ru = os.wait4(pid, os.WNOHANG)
                        if wpid == pid:
                            reaped = True
                            wait4_maxrss = int(ru.ru_maxrss)
                            wait4_utime = float(ru.ru_utime)
                            wait4_stime = float(ru.ru_stime)
                            wait4_majflt = int(ru.ru_majflt)
                            wait4_nswap = int(ru.ru_nswap)
                            self.proc.returncode = os.waitstatus_to_exitcode(status)
                            break
                    except ChildProcessError:
                        reaped = True
                        break
                    time.sleep(0.1)

                if not reaped:
                    try:
                        self.proc.kill()
                    except OSError:
                        pass
                    try:
                        _, status, ru = os.wait4(pid, 0)
                        reaped = True
                        wait4_maxrss = int(ru.ru_maxrss)
                        wait4_utime = float(ru.ru_utime)
                        wait4_stime = float(ru.ru_stime)
                        wait4_majflt = int(ru.ru_majflt)
                        wait4_nswap = int(ru.ru_nswap)
                        self.proc.returncode = os.waitstatus_to_exitcode(status)
                    except ChildProcessError:
                        self.proc.wait()

            polled_rss = max((s["resident_bytes"] for s in self.samples), default=0)
            polled_phys = max((s["phys_footprint_bytes"] for s in self.samples), default=0)
            polled_max_phys = max((s["lifetime_max_phys_footprint_bytes"] for s in self.samples), default=0)
            polled_pageins = max((s["pageins"] for s in self.samples), default=0)

            peak_rss_bytes = max(wait4_maxrss, polled_rss)
            peak_phys_bytes = max(polled_phys, polled_max_phys)
            cpu_total_s = wait4_utime + wait4_stime

            ru_stats.update({
                "peak_rss_bytes": peak_rss_bytes,
                "peak_rss_mib": round(peak_rss_bytes / 1048576.0, 2),
                "peak_phys_footprint_bytes": peak_phys_bytes,
                "peak_phys_footprint_mib": round(peak_phys_bytes / 1048576.0, 2),
                "cpu_user_sec": round(wait4_utime, 3),
                "cpu_sys_sec": round(wait4_stime, 3),
                "cpu_utilization_pct": round((cpu_total_s / wall_total_s) * 100.0, 2),
                "major_page_faults": wait4_majflt,
                "polled_pageins": polled_pageins,
                "nswap": wait4_nswap,
                "rusage_samples": len(self.samples),
            })

            if self.proc.stdout:
                self.proc.stdout.close()

        return ru_stats


def strip_think_blocks(text: str) -> str:
    """Strip closed, unclosed, and HTML/Unicode-encoded <think> reasoning blocks."""
    if not text:
        return ""
    cleaned = _THINK_BLOCK_RE.sub("", text)
    cleaned = _THINK_UNCLOSED_RE.sub("", cleaned)
    cleaned = _THINK_RE.sub("", cleaned)
    return cleaned.strip()


def _boundary_contains(needle: str, haystack: str, ignore_case: bool = False) -> bool:
    if not needle:
        return True
    escaped = re.escape(needle)
    left = ""
    right = ""
    if needle[0].isalnum() or needle[0] == "_":
        left = r"(?<!\w)(?<!\d\.)"
    elif needle[0] in "$€£¥₹":
        left = r"(?<!\d)"
    if needle[-1].isalnum() or needle[-1] == "_":
        right = r"(?!\w)(?!\.\d)"
    elif needle[-1] in ")%":
        right = r"(?!\w)"
    flags = re.IGNORECASE if ignore_case else 0
    return re.search(f"{left}{escaped}{right}", haystack, flags) is not None


def _token_present(expected: str, output_text: str, category: str) -> bool:
    if category in ("citations", "DOIs", "URLs", "quoted_strings", "code_blocks"):
        if expected in output_text:
            return True
        norm_exp = expected.replace("–", "-").replace("—", "-").replace("“", '"').replace("”", '"')
        norm_out = output_text.replace("–", "-").replace("—", "-").replace("“", '"').replace("”", '"')
        return norm_exp in norm_out

    if _boundary_contains(expected, output_text):
        return True
    if category == "named_entities" and _boundary_contains(expected, output_text, ignore_case=True):
        return True
    # Normalize curly vs straight quotes/dashes for minor punctuation equivalence
    norm_exp = expected.replace("–", "-").replace("—", "-").replace("“", '"').replace("”", '"')
    norm_out = output_text.replace("–", "-").replace("—", "-").replace("“", '"').replace("”", '"')
    if _boundary_contains(norm_exp, norm_out, ignore_case=(category == "named_entities")):
        return True
    # Also check number formatting variants (e.g., 1,420 vs 1420)
    if category == "numbers" and "," in expected:
        if _boundary_contains(expected.replace(",", ""), output_text):
            return True
    return False


def _split_paragraphs(text: str) -> list[str]:
    """Split text into paragraphs on blank lines without splitting inside fenced code blocks."""
    normalized = text.replace("\r\n", "\n").replace("\r", "\n").strip()
    if not normalized:
        return []
    paras: list[str] = []
    cur_lines: list[str] = []
    in_fence = False
    fence_char = ""
    fence_len = 0

    for line in normalized.split("\n"):
        stripped = line.lstrip()
        fence_match = re.match(r"^(`{3,}|~{3,})(.*)$", stripped)
        if fence_match:
            marker = fence_match.group(1)
            rest = fence_match.group(2)
            if not in_fence:
                in_fence = True
                fence_char = marker[0]
                fence_len = len(marker)
            elif marker[0] == fence_char and len(marker) >= fence_len and not rest.strip():
                in_fence = False
                fence_char = ""
                fence_len = 0
            cur_lines.append(line)
        elif not in_fence and not line.strip():
            if cur_lines:
                block = "\n".join(cur_lines).strip()
                if block:
                    paras.append(block)
                cur_lines = []
        else:
            cur_lines.append(line)

    if cur_lines:
        block = "\n".join(cur_lines).strip()
        if block:
            paras.append(block)
    return paras


def evaluate_text_metrics(
    source_text: str,
    raw_output: str,
    reasoning_content: str | None,
    expected_invariants: dict[str, Any],
) -> dict[str, Any]:
    think_in_content = bool(_THINK_RE.search(raw_output or ""))
    think_in_reasoning = bool((reasoning_content or "").strip())
    think_leakage = think_in_content or think_in_reasoning

    cleaned = strip_think_blocks(raw_output or "")

    src_words = len(source_text.split())
    out_words = len(cleaned.split())
    word_ratio = round(out_words / max(1, src_words), 4)

    src_paras = len(_split_paragraphs(source_text))
    out_paras = len(_split_paragraphs(cleaned))

    # Invariant retention
    cat_results: dict[str, Any] = {}
    total_expected = 0
    total_matched = 0
    missing_items: list[str] = []

    for cat in ("citations", "DOIs", "URLs", "numbers", "currencies", "dates", "quoted_strings", "code_blocks", "named_entities"):
        items = expected_invariants.get(cat, []) or []
        if not items:
            continue
        matched = 0
        cat_missing: list[str] = []
        for item in items:
            total_expected += 1
            if _token_present(item, cleaned, cat):
                matched += 1
                total_matched += 1
            else:
                cat_missing.append(item)
                missing_items.append(f"{cat}:{item}")
        cat_results[cat] = {
            "expected": len(items),
            "matched": matched,
            "missing": cat_missing,
            "rate": round(matched / len(items), 4),
        }

    overall_retention = round(total_matched / total_expected, 4) if total_expected > 0 else 1.0

    # N-gram repetition check (4-grams)
    tokens = re.findall(r"[A-Za-z0-9']+", cleaned.lower())
    ngrams = [tuple(tokens[i : i + 4]) for i in range(max(0, len(tokens) - 3))]
    ngram_counts = Counter(ngrams)
    max_4gram_count = max(ngram_counts.values(), default=0)
    ngram_repeat_rate = round(1.0 - (len(ngram_counts) / len(ngrams)), 4) if ngrams else 0.0

    # Lexical distance vs source (1 - Jaccard on unigrams+bigrams)
    src_toks = re.findall(r"[A-Za-z0-9']+", source_text.lower())
    src_grams = set(src_toks) | set(zip(src_toks, src_toks[1:]))
    out_grams = set(tokens) | set(zip(tokens, tokens[1:]))
    union_n = len(src_grams | out_grams)
    jaccard = (len(src_grams & out_grams) / union_n) if union_n > 0 else 1.0
    lex_distance = round(1.0 - jaccard, 4)

    return {
        "cleaned_output": cleaned,
        "think_leakage": think_leakage,
        "think_in_content": think_in_content,
        "think_in_reasoning": think_in_reasoning,
        "source_words": src_words,
        "output_words": out_words,
        "word_count_ratio": word_ratio,
        "truncation_flag": word_ratio < 0.70,
        "expansion_flag": word_ratio > 1.45,
        "source_paragraphs": src_paras,
        "output_paragraphs": out_paras,
        "paragraph_count_delta": out_paras - src_paras,
        "paragraph_preserved": out_paras == src_paras,
        "paragraph_no_loss": out_paras >= src_paras,
        "invariant_retention_overall": overall_retention,
        "invariant_matched": total_matched,
        "invariant_expected": total_expected,
        "invariant_categories": cat_results,
        "missing_invariants": missing_items,
        "max_4gram_count": max_4gram_count,
        "ngram_4_repeat_rate": ngram_repeat_rate,
        "repetition_flag": max_4gram_count >= 4,
        "lexical_distance": lex_distance,
        "verbatim_echo_flag": lex_distance < 0.08,
    }


def chunk_paragraphs(text: str, min_words: int = 320, target_max_words: int = 430) -> list[str]:
    paras = _split_paragraphs(text)
    chunks: list[str] = []
    cur: list[str] = []
    cur_words = 0

    for p in paras:
        pw = len(p.split())
        if cur and (cur_words + pw > target_max_words) and cur_words >= min_words:
            chunks.append("\n\n".join(cur))
            cur = [p]
            cur_words = pw
        elif cur and (cur_words + pw > 480):
            chunks.append("\n\n".join(cur))
            cur = [p]
            cur_words = pw
        else:
            cur.append(p)
            cur_words += pw
    if cur:
        chunks.append("\n\n".join(cur))
    return chunks


def trailing_sentences(text: str, count: int = 2) -> str:
    sents = [s.strip() for s in _SENTENCE_END_RE.split(text.strip()) if s.strip()]
    if not sents:
        return ""
    tail = " ".join(sents[-count:])
    words = tail.split()
    if len(words) > 85:
        tail = " ".join(words[-85:])
    return tail


def extract_chunk_invariants(chunk_text: str, doc_invariants: dict[str, Any]) -> dict[str, Any]:
    sub: dict[str, list[str]] = {}
    for cat in ("citations", "DOIs", "URLs", "numbers", "currencies", "dates", "quoted_strings", "code_blocks", "named_entities"):
        items = [x for x in (doc_invariants.get(cat, []) or []) if _token_present(x, chunk_text, cat)]
        if items:
            sub[cat] = items
    return sub


def run_short_tasks(candidate: dict[str, Any], model_path: Path, mode: str, tasks: list[dict[str, Any]], timeout_s: float) -> dict[str, Any]:
    print(f"\n=== [{candidate['id']}] mode={mode} (short_tasks={len(tasks)}) ===", flush=True)
    runner = LlamaServerRunner(model_path, candidate, mode)
    task_results: list[dict[str, Any]] = []
    server_error: str | None = None

    try:
        runner.start(ready_timeout_s=120.0)
        print(f"[ready] {candidate['id']} ({mode}) loaded in {runner.load_ms:.1f} ms on port {runner.port}", flush=True)
        for idx, task in enumerate(tasks, start=1):
            src = task["text"]
            src_words = len(src.split())
            max_toks = min(1400, max(512, int(src_words * 2.6)))
            sys_p, usr_p = build_prompts(
                task,
                src,
                disable_thinking=bool(candidate.get("disable_thinking")),
            )
            messages = build_chat_messages(candidate, sys_p, usr_p)

            print(f"  [{idx}/{len(tasks)}] {task['id']} ({src_words}w)...", end="", flush=True)
            t_req = time.perf_counter()
            try:
                resp = runner.chat_completion(messages, max_tokens=max_toks, timeout_s=timeout_s)
                choice = resp["choices"][0]["message"]
                raw_content = choice.get("content") or ""
                reasoning_content = choice.get("reasoning_content")
                timings = resp.get("timings", {})
                usage = resp.get("usage", {})
                metrics = evaluate_text_metrics(src, raw_content, reasoning_content, task["expected_invariants"])
                wall_ms = round(resp.get("_client_wall_ms", (time.perf_counter() - t_req) * 1000.0), 1)
                gen_tps = round(float(timings.get("predicted_per_second", 0.0)), 2)
                prompt_tps = round(float(timings.get("prompt_per_second", 0.0)), 2)
                print(
                    f" done {wall_ms:.0f}ms | {gen_tps} tok/s | ret={metrics['invariant_retention_overall']*100:.1f}% | "
                    f"ratio={metrics['word_count_ratio']:.2f} | think={metrics['think_leakage']}",
                    flush=True,
                )
                task_results.append({
                    "task_id": task["id"],
                    "mode": task["mode"],
                    "intensity": task["intensity"],
                    "wall_clock_ms": wall_ms,
                    "timings": {
                        "prompt_n": timings.get("prompt_n"),
                        "prompt_ms": timings.get("prompt_ms"),
                        "prompt_per_second": prompt_tps,
                        "predicted_n": timings.get("predicted_n"),
                        "predicted_ms": timings.get("predicted_ms"),
                        "predicted_per_second": gen_tps,
                    },
                    "usage": usage,
                    "raw_output": raw_content,
                    "reasoning_content": reasoning_content,
                    "metrics": metrics,
                    "error": None,
                })
            except Exception as exc:
                wall_ms = round((time.perf_counter() - t_req) * 1000.0, 1)
                print(f" ERROR after {wall_ms:.0f}ms: {exc}", flush=True)
                task_results.append({
                    "task_id": task["id"],
                    "mode": task["mode"],
                    "intensity": task["intensity"],
                    "wall_clock_ms": wall_ms,
                    "error": str(exc),
                })
    except Exception as exc:
        server_error = str(exc)
        print(f"[server-error] {candidate['id']} ({mode}): {server_error}", flush=True)
    finally:
        proc_metrics = runner.stop()

    completed = [r for r in task_results if r.get("error") is None]
    agg: dict[str, Any] = {
        "tasks_completed": len(completed),
        "tasks_total": len(tasks),
        "server_error": server_error,
    }
    if completed:
        agg.update({
            "mean_wall_clock_ms": round(sum(r["wall_clock_ms"] for r in completed) / len(completed), 1),
            "total_wall_clock_ms": round(sum(r["wall_clock_ms"] for r in completed), 1),
            "mean_prompt_tok_per_sec": round(sum(r["timings"]["prompt_per_second"] for r in completed) / len(completed), 2),
            "mean_gen_tok_per_sec": round(sum(r["timings"]["predicted_per_second"] for r in completed) / len(completed), 2),
            "mean_invariant_retention": round(sum(r["metrics"]["invariant_retention_overall"] for r in completed) / len(completed), 4),
            "mean_word_ratio": round(sum(r["metrics"]["word_count_ratio"] for r in completed) / len(completed), 4),
            "mean_lexical_distance": round(sum(r["metrics"]["lexical_distance"] for r in completed) / len(completed), 4),
            "think_leakage_count": sum(1 for r in completed if r["metrics"]["think_leakage"]),
            "truncation_count": sum(1 for r in completed if r["metrics"]["truncation_flag"]),
            "repetition_count": sum(1 for r in completed if r["metrics"]["repetition_flag"]),
            "verbatim_echo_count": sum(1 for r in completed if r["metrics"]["verbatim_echo_flag"]),
        })

    return {
        "schema_version": "1.0.0",
        "timestamp_utc": datetime.now(timezone.utc).isoformat(),
        "model_id": candidate["id"],
        "model_name": candidate["name"],
        "repo_id": candidate["repo_id"],
        "filename": candidate["filename"],
        "license": candidate["license"],
        "non_commercial_warning": candidate["non_commercial_warning"],
        "artifact_bytes": candidate["size_bytes"],
        "artifact_sha256": candidate["sha256"],
        "backend_mode": mode,
        "runtime_flags": {
            "ctx_size": candidate.get("ctx_size", 4096),
            "parallel": 1,
            "threads": 4,
            "n_gpu_layers": 0 if mode == "cpu" else 99,
            "disable_thinking": candidate.get("disable_thinking", False),
            "fold_system_into_user": candidate.get("fold_system_into_user", False),
        },
        "process_metrics": proc_metrics,
        "aggregate": agg,
        "tasks": task_results,
    }


def _select_best_attempt(chunk_record: dict[str, Any]) -> dict[str, Any]:
    attempts = chunk_record.get("attempts", [])
    if not attempts:
        raise ValueError(f"Chunk {chunk_record.get('chunk_index')} has no attempts")
    selected_num = chunk_record.get("selected_attempt")
    if selected_num is not None:
        for att in attempts:
            if att.get("attempt") == selected_num:
                return att
    best = attempts[0]
    best_score = (
        best["metrics"]["invariant_retention_overall"],
        not best["metrics"]["truncation_flag"],
        not best["metrics"]["verbatim_echo_flag"],
        best["metrics"]["lexical_distance"],
    )
    for att in attempts[1:]:
        score = (
            att["metrics"]["invariant_retention_overall"],
            not att["metrics"]["truncation_flag"],
            not att["metrics"]["verbatim_echo_flag"],
            att["metrics"]["lexical_distance"],
        )
        if score >= best_score:
            best = att
            best_score = score
    return best


def recompute_longdoc_payload(
    data: dict[str, Any],
    expected_invariants: dict[str, Any],
    source_text: str | None = None,
) -> dict[str, Any]:
    """Recompute long-doc aggregate fields from saved chunk/attempt logs without altering raw attempts or process_metrics."""
    chunks = data.get("chunks", [])
    best_attempts = [_select_best_attempt(c) for c in chunks]
    rewritten_chunks = [c.get("final_output", "") for c in chunks]
    src = source_text if source_text is not None else "\n\n".join(c.get("source_text", "") for c in chunks)

    full_output = "\n\n".join(rewritten_chunks)
    full_metrics = evaluate_text_metrics(src, full_output, None, expected_invariants)

    total_wall_ms = round(
        sum(a["wall_clock_ms"] for c in chunks for a in c.get("attempts", [])),
        1,
    )
    accepted_wall_ms = round(sum(b["wall_clock_ms"] for b in best_attempts), 1)
    selected_gen_tps = [b["predicted_per_second"] for b in best_attempts if b.get("predicted_per_second")]
    all_gen_tps = [
        a["predicted_per_second"]
        for c in chunks
        for a in c.get("attempts", [])
        if a.get("predicted_per_second")
    ]
    echo_indices = [
        c["chunk_index"]
        for c, b in zip(chunks, best_attempts)
        if b.get("metrics", {}).get("verbatim_echo_flag")
    ]

    updated = dict(data)
    updated["total_inference_wall_ms"] = total_wall_ms
    updated["accepted_inference_wall_ms"] = accepted_wall_ms
    updated["mean_gen_tok_per_sec"] = (
        round(sum(selected_gen_tps) / len(selected_gen_tps), 2) if selected_gen_tps else 0.0
    )
    updated["all_attempts_mean_gen_tok_per_sec"] = (
        round(sum(all_gen_tps) / len(all_gen_tps), 2) if all_gen_tps else 0.0
    )
    updated["per_chunk_verbatim_echo_count"] = len(echo_indices)
    updated["per_chunk_verbatim_echo_indices"] = echo_indices
    updated["document_metrics"] = full_metrics
    return updated


def run_long_doc_benchmark(candidate: dict[str, Any], model_path: Path, mode: str, long_doc: dict[str, Any], timeout_s: float) -> dict[str, Any]:
    src = long_doc["text"]
    chunks = chunk_paragraphs(src, min_words=240, target_max_words=330)
    print(f"\n=== [{candidate['id']}] mode={mode} LONG DOC ({len(src.split())}w, {len(chunks)} chunks) ===", flush=True)

    runner = LlamaServerRunner(model_path, candidate, mode)
    chunk_records: list[dict[str, Any]] = []
    chunks_retried = 0
    server_error: str | None = None

    try:
        runner.start(ready_timeout_s=120.0)
        print(f"[ready] {candidate['id']} ({mode}) loaded in {runner.load_ms:.1f} ms", flush=True)
        prev_tail: str | None = None

        for idx, chunk_text in enumerate(chunks, start=1):
            c_words = len(chunk_text.split())
            c_inv = extract_chunk_invariants(chunk_text, long_doc.get("expected_invariants", {}))

            # M2 runs unmasked natural text: inline __LH_PROT_###__ placeholders caused 1.7B weights
            # to freeze into verbatim copying during initial 8-chunk experiments.
            # On retry: separate invariant/truncation retries (lower temp + strict check) from
            # verbatim-echo retries (raise temp + top_p + inject explicit anti-copy instruction).
            best_attempt: dict[str, Any] | None = None
            attempts_log: list[dict[str, Any]] = []
            last_metrics: dict[str, Any] | None = None

            for attempt_no in (1, 2):
                if attempt_no == 1:
                    strict_retry = False
                    anti_echo_retry = False
                    temp = 0.55
                    top_p = 0.85
                else:
                    inv_or_trunc_fail = (
                        last_metrics is not None
                        and (
                            last_metrics["invariant_retention_overall"] < 1.0
                            or last_metrics["truncation_flag"]
                            or last_metrics["think_leakage"]
                        )
                    )
                    echo_fail = last_metrics is not None and bool(last_metrics["verbatim_echo_flag"])
                    if echo_fail and not inv_or_trunc_fail:
                        strict_retry = False
                        anti_echo_retry = True
                        temp = 0.72
                        top_p = 0.90
                    elif echo_fail and inv_or_trunc_fail:
                        strict_retry = True
                        anti_echo_retry = True
                        temp = 0.60
                        top_p = 0.88
                    else:
                        strict_retry = True
                        anti_echo_retry = False
                        temp = 0.45
                        top_p = 0.85

                sys_p, usr_p = build_prompts(
                    long_doc,
                    chunk_text,
                    prev_context=prev_tail,
                    strict_retry=strict_retry,
                    anti_echo_retry=anti_echo_retry,
                    disable_thinking=bool(candidate.get("disable_thinking")),
                )
                messages = build_chat_messages(candidate, sys_p, usr_p)
                max_toks = min(1400, max(512, int(c_words * 2.2)))

                t0 = time.perf_counter()
                resp = runner.chat_completion(
                    messages,
                    max_tokens=max_toks,
                    temperature=temp,
                    top_p=top_p,
                    timeout_s=timeout_s,
                )
                wall_ms = round(resp.get("_client_wall_ms", (time.perf_counter() - t0) * 1000.0), 1)
                choice = resp["choices"][0]["message"]
                raw_out = choice.get("content") or ""
                reasoning_out = choice.get("reasoning_content")
                timings = resp.get("timings", {})

                cleaned_pre = strip_think_blocks(raw_out)
                # Guard against accidental leading repetition of previous chunk's trailing context
                if prev_tail and cleaned_pre.startswith(prev_tail):
                    cleaned_pre = cleaned_pre[len(prev_tail):].lstrip()

                m = evaluate_text_metrics(chunk_text, cleaned_pre, reasoning_out, c_inv)
                if _THINK_RE.search(raw_out):
                    m["think_leakage"] = True
                    m["think_in_content"] = True
                last_metrics = m

                needs_retry = (
                    m["invariant_retention_overall"] < 1.0
                    or m["truncation_flag"]
                    or m["think_leakage"]
                    or m["verbatim_echo_flag"]
                )
                att_info = {
                    "attempt": attempt_no,
                    "masked": False,
                    "unrestored_placeholders": [],
                    "wall_clock_ms": wall_ms,
                    "prompt_per_second": round(float(timings.get("prompt_per_second", 0.0)), 2),
                    "predicted_per_second": round(float(timings.get("predicted_per_second", 0.0)), 2),
                    "predicted_n": timings.get("predicted_n"),
                    "metrics": m,
                }
                attempts_log.append(att_info)
                best_attempt = att_info

                if not needs_retry:
                    break
                if attempt_no == 1:
                    chunks_retried += 1
                    print(
                        f"    chunk {idx}/{len(chunks)} retry triggered (ret={m['invariant_retention_overall']:.2f}, "
                        f"lex={m['lexical_distance']:.4f}, ratio={m['word_count_ratio']:.2f})",
                        flush=True,
                    )

            assert best_attempt is not None
            # Choose better attempt between 1 and 2 based on retention, non-truncation, and non-echo
            if len(attempts_log) == 2:
                a1, a2 = attempts_log[0], attempts_log[1]
                s1 = (a1["metrics"]["invariant_retention_overall"], not a1["metrics"]["truncation_flag"], not a1["metrics"]["verbatim_echo_flag"], a1["metrics"]["lexical_distance"])
                s2 = (a2["metrics"]["invariant_retention_overall"], not a2["metrics"]["truncation_flag"], not a2["metrics"]["verbatim_echo_flag"], a2["metrics"]["lexical_distance"])
                best_attempt = a2 if s2 >= s1 else a1

            final_chunk_text = best_attempt["metrics"]["cleaned_output"]
            prev_tail = trailing_sentences(final_chunk_text, count=2)

            print(
                f"  [chunk {idx}/{len(chunks)}] {c_words}w -> {best_attempt['metrics']['output_words']}w | "
                f"{best_attempt['wall_clock_ms']:.0f}ms | {best_attempt['predicted_per_second']} tok/s | "
                f"ret={best_attempt['metrics']['invariant_retention_overall']*100:.1f}%",
                flush=True,
            )
            chunk_records.append({
                "chunk_index": idx,
                "source_words": c_words,
                "source_text": chunk_text,
                "final_output": final_chunk_text,
                "selected_attempt": best_attempt["attempt"],
                "attempts": attempts_log,
            })
    except Exception as exc:
        server_error = str(exc)
        print(f"[longdoc-error] {candidate['id']} ({mode}): {server_error}", flush=True)
    finally:
        proc_metrics = runner.stop()

    base_payload = {
        "schema_version": "1.0.0",
        "timestamp_utc": datetime.now(timezone.utc).isoformat(),
        "model_id": candidate["id"],
        "model_name": candidate["name"],
        "backend_mode": mode,
        "artifact_bytes": candidate["size_bytes"],
        "artifact_sha256": candidate["sha256"],
        "chunks_total": len(chunks),
        "chunks_completed": len(chunk_records),
        "chunks_retried": chunks_retried,
        "total_inference_wall_ms": 0.0,
        "accepted_inference_wall_ms": 0.0,
        "mean_gen_tok_per_sec": 0.0,
        "all_attempts_mean_gen_tok_per_sec": 0.0,
        "per_chunk_verbatim_echo_count": 0,
        "per_chunk_verbatim_echo_indices": [],
        "server_error": server_error,
        "process_metrics": proc_metrics,
        "document_metrics": {},
        "chunks": chunk_records,
    }
    return recompute_longdoc_payload(
        base_payload,
        long_doc.get("expected_invariants", {}),
        source_text=src,
    )


def rebuild_summaries() -> None:
    RESULTS_DIR.mkdir(parents=True, exist_ok=True)
    rows: list[dict[str, Any]] = []
    for p in sorted(RESULTS_DIR.glob("*.json")):
        if p.name in ("summary.json",) or p.name.endswith("_longdoc.json"):
            continue
        data = json.loads(p.read_text(encoding="utf-8"))
        agg = data.get("aggregate", {})
        pm = data.get("process_metrics", {})
        rows.append({
            "model_id": data.get("model_id"),
            "model_name": data.get("model_name"),
            "license": data.get("license"),
            "non_commercial_warning": data.get("non_commercial_warning"),
            "artifact_bytes": data.get("artifact_bytes"),
            "artifact_mib": round((data.get("artifact_bytes") or 0) / 1048576.0, 2),
            "backend_mode": data.get("backend_mode"),
            "tasks_completed": agg.get("tasks_completed"),
            "tasks_total": agg.get("tasks_total"),
            "model_load_ms": pm.get("model_load_ms"),
            "peak_rss_mib": pm.get("peak_rss_mib"),
            "peak_phys_footprint_mib": pm.get("peak_phys_footprint_mib"),
            "cpu_utilization_pct": pm.get("cpu_utilization_pct"),
            "swap_delta_mb": pm.get("swap_delta_mb"),
            "major_page_faults": pm.get("major_page_faults"),
            "mean_wall_clock_ms": agg.get("mean_wall_clock_ms"),
            "total_wall_clock_ms": agg.get("total_wall_clock_ms"),
            "mean_prompt_tok_per_sec": agg.get("mean_prompt_tok_per_sec"),
            "mean_gen_tok_per_sec": agg.get("mean_gen_tok_per_sec"),
            "mean_invariant_retention": agg.get("mean_invariant_retention"),
            "mean_word_ratio": agg.get("mean_word_ratio"),
            "mean_lexical_distance": agg.get("mean_lexical_distance"),
            "think_leakage_count": agg.get("think_leakage_count"),
            "truncation_count": agg.get("truncation_count"),
            "repetition_count": agg.get("repetition_count"),
            "verbatim_echo_count": agg.get("verbatim_echo_count"),
            "server_error": agg.get("server_error"),
        })

    summary_json = {
        "schema_version": "1.0.0",
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "host": collect_host_info(),
        "runs": rows,
    }
    (RESULTS_DIR / "summary.json").write_text(json.dumps(summary_json, indent=2), encoding="utf-8")

    if rows:
        fieldnames = list(rows[0].keys())
        with (RESULTS_DIR / "summary.csv").open("w", newline="", encoding="utf-8") as f:
            writer = csv.DictWriter(f, fieldnames=fieldnames)
            writer.writeheader()
            writer.writerows(rows)


def main() -> int:
    parser = argparse.ArgumentParser(description="Local Humanizer M2 Benchmark Harness")
    parser.add_argument("--models", default="all", help="Comma-separated model IDs or 'all'")
    parser.add_argument("--modes", default="cpu,metal", help="Comma-separated modes: cpu,metal")
    parser.add_argument("--long-doc-model", default="none", help="Model ID for 3,000+ word chunked benchmark or 'none'")
    parser.add_argument("--long-doc-only", action="store_true", help="Skip short_tasks and run only long_doc_3000 for --long-doc-model")
    parser.add_argument("--keep-model", default="none", help="Optional model ID whose verified GGUF stays in benchmarks/tmp/")
    parser.add_argument("--skip-download", action="store_true", help="Reuse already downloaded GGUF without downloading")
    parser.add_argument("--task-timeout", type=float, default=300.0, help="Per-request HTTP timeout in seconds")
    parser.add_argument("--dry-run", action="store_true", help="Validate tasks, runtime binary, and disk plan without downloading")
    parser.add_argument("--recompute-longdoc", action="store_true", help="Recompute long-doc aggregates from saved *_longdoc.json files without modifying raw outputs or process_metrics")
    args = parser.parse_args()

    if not TASKS_PATH.exists():
        print(f"ERROR: Missing {TASKS_PATH}", file=sys.stderr)
        return 1
    tasks_doc = json.loads(TASKS_PATH.read_text(encoding="utf-8"))
    short_tasks = tasks_doc["short_tasks"]
    long_doc = tasks_doc["long_doc_3000"]

    selected_ids = [c["id"] for c in CANDIDATES] if args.models == "all" else [m.strip() for m in args.models.split(",") if m.strip()]
    for mid in selected_ids:
        if mid not in CANDIDATE_MAP:
            print(f"ERROR: Unknown model ID {mid!r}. Valid: {list(CANDIDATE_MAP)}", file=sys.stderr)
            return 1

    modes = [m.strip() for m in args.modes.split(",") if m.strip()]
    for m in modes:
        if m not in ("cpu", "metal"):
            print(f"ERROR: Invalid mode {m!r}. Valid: cpu, metal", file=sys.stderr)
            return 1

    if args.long_doc_model != "none" and args.long_doc_model not in CANDIDATE_MAP:
        print(f"ERROR: Unknown --long-doc-model {args.long_doc_model!r}", file=sys.stderr)
        return 1

    if args.recompute_longdoc:
        target_files: list[Path] = []
        if args.long_doc_model != "none":
            for m in modes:
                p = RESULTS_DIR / f"{args.long_doc_model}_{m}_longdoc.json"
                if p.exists():
                    target_files.append(p)
        else:
            for p in sorted(RESULTS_DIR.glob("*_longdoc.json")):
                if any(p.name.endswith(f"_{m}_longdoc.json") for m in modes):
                    target_files.append(p)
        if not target_files:
            print("ERROR: No matching *_longdoc.json files found to recompute", file=sys.stderr)
            return 1
        for ld_file in target_files:
            orig = json.loads(ld_file.read_text(encoding="utf-8"))
            updated = recompute_longdoc_payload(
                orig,
                long_doc.get("expected_invariants", {}),
                source_text=long_doc.get("text"),
            )
            ld_file.write_text(json.dumps(updated, indent=2), encoding="utf-8")
            print(
                f"[recompute-longdoc] {ld_file.name}: "
                f"total_inference_wall_ms={updated['total_inference_wall_ms']}, "
                f"accepted_inference_wall_ms={updated['accepted_inference_wall_ms']}, "
                f"mean_gen_tok_per_sec={updated['mean_gen_tok_per_sec']}, "
                f"per_chunk_verbatim_echo_count={updated['per_chunk_verbatim_echo_count']}, "
                f"per_chunk_verbatim_echo_indices={updated['per_chunk_verbatim_echo_indices']}"
            )
        return 0

    host = collect_host_info()
    print(f"Host: {host['cpu_brand']} ({host['hw_model']}), RAM={host['mem_bytes'] / 1073741824:.1f} GiB, "
          f"DiskFree={host['disk_free_bytes'] / 1073741824:.2f} GiB, llama-server={host['llama_server_version']}")
    print(f"Tasks: short={len(short_tasks)}, long_doc_words={len(long_doc['text'].split())}, modes={modes}")

    if args.dry_run:
        if not LLAMA_SERVER_BIN.exists():
            print(f"ERROR: Missing llama-server at {LLAMA_SERVER_BIN}", file=sys.stderr)
            return 1
        for mid in selected_ids:
            c = CANDIDATE_MAP[mid]
            print(f"  [dry-run] {c['id']} | {c['size_bytes'] / 1048576:.1f} MiB | {c['license']} | "
                  f"disable_thinking={c['disable_thinking']} | fold_sys={c['fold_system_into_user']}")
        print("Dry-run checks PASSED.")
        return 0

    RESULTS_DIR.mkdir(parents=True, exist_ok=True)
    for mid in selected_ids:
        candidate = CANDIDATE_MAP[mid]
        gguf_path: Path | None = None
        try:
            gguf_path = download_verify_gguf(candidate, skip_download=args.skip_download)
            for mode in modes:
                if not args.long_doc_only:
                    res = run_short_tasks(candidate, gguf_path, mode, short_tasks, timeout_s=args.task_timeout)
                    out_file = RESULTS_DIR / f"{mid}_{mode}.json"
                    out_file.write_text(json.dumps(res, indent=2), encoding="utf-8")
                    print(f"[saved] {out_file}")

                if args.long_doc_model == mid:
                    ld_res = run_long_doc_benchmark(candidate, gguf_path, mode, long_doc, timeout_s=args.task_timeout)
                    ld_file = RESULTS_DIR / f"{mid}_{mode}_longdoc.json"
                    ld_file.write_text(json.dumps(ld_res, indent=2), encoding="utf-8")
                    print(f"[saved] {ld_file}")
        finally:
            if gguf_path is not None and gguf_path.exists() and args.keep_model != mid:
                if gguf_path.parent == TMP_DIR:
                    print(f"[cleanup] Removing temporary GGUF {gguf_path}", flush=True)
                    gguf_path.unlink()

    rebuild_summaries()
    print(f"\n[done] Updated {RESULTS_DIR / 'summary.json'} and {RESULTS_DIR / 'summary.csv'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
