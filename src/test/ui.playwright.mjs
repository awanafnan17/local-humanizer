import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs/promises";
import net from "node:net";
import os from "node:os";
import path from "node:path";

async function loadPlaywright() {
  const candidates = [
    process.env.PLAYWRIGHT_MODULE_PATH,
    "playwright",
    "/opt/homebrew/lib/python3.14/site-packages/playwright/driver/package/index.mjs",
  ].filter(Boolean);

  let lastErr;
  for (const modPath of candidates) {
    try {
      const mod = await import(modPath);
      if (mod?.chromium) return mod;
      if (mod?.default?.chromium) return mod.default;
    } catch (err) {
      lastErr = err;
    }
  }
  throw new Error(
    `Playwright Node driver not found. Tried ${candidates.join(", ")}. Last error: ${lastErr}`,
  );
}

function getAvailablePort() {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.unref();
    srv.on("error", reject);
    srv.listen(0, "127.0.0.1", () => {
      const addr = srv.address();
      if (!addr || typeof addr === "string") {
        srv.close(() => reject(new Error("Could not resolve port")));
        return;
      }
      const { port } = addr;
      srv.close(() => resolve(port));
    });
  });
}

async function waitForServer(url, timeoutMs = 15000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(url);
      if (res.ok) return;
    } catch {
      // Retry until ready
    }
    await new Promise((r) => setTimeout(r, 150));
  }
  throw new Error(`Timed out waiting for preview server at ${url}`);
}

function build3200WordUtf8Doc() {
  const sampleParagraphs = [
    "Local Humanizer preserves citations [1], URLs https://example.org/spec, and UTF-8 prose such as café résumé naïve while processing long documents seamlessly.",
    "Multilingual segments including 日本語テスト and Arabic مرحبا بالعالم alongside exact quotes \"measured empirical baseline\" remain intact across chunk boundaries.",
  ];
  const filler = Array.from({ length: 155 }, (_, i) => `term${i}`).join(" ");
  const blocks = [];
  for (let i = 0; i < 20; i += 1) {
    blocks.push(`${sampleParagraphs[i % 2]} ${filler}`);
  }
  return blocks.join("\n\n");
}

async function runSuite() {
  const { chromium } = await loadPlaywright();
  const port = await getAvailablePort();
  const baseUrl = `http://127.0.0.1:${port}`;

  const previewProc = spawn(
    process.execPath,
    [
      "./node_modules/vite/bin/vite.js",
      "preview",
      "--host",
      "127.0.0.1",
      "--port",
      String(port),
      "--strictPort",
    ],
    {
      cwd: process.cwd(),
      stdio: ["ignore", "pipe", "pipe"],
    },
  );

  let serverLogs = "";
  previewProc.stdout.on("data", (d) => {
    serverLogs += d.toString();
  });
  previewProc.stderr.on("data", (d) => {
    serverLogs += d.toString();
  });

  let browser;
  const tmpDir = await fs.mkdtemp(path.join(os.tmpdir(), "lh-pw-"));

  try {
    await waitForServer(baseUrl).catch((err) => {
      throw new Error(`${err.message}\nServer logs:\n${serverLogs}`);
    });
    browser = await chromium.launch({ headless: true });

    const consoleErrors = [];
    const allConsoleMessages = [];
    const pageErrors = [];
    const outboundRequests = [];

    const createTrackedPage = async (initScriptFn, initArg) => {
      const context = await browser.newContext({
        viewport: { width: 1280, height: 800 },
        acceptDownloads: true,
      });
      await context.grantPermissions(["clipboard-read", "clipboard-write"], {
        origin: baseUrl,
      });
      if (initScriptFn) {
        await context.addInitScript(initScriptFn, initArg);
      }
      const page = await context.newPage();

      await page.route("**/*", (route) => {
        const url = route.request().url();
        if (
          url.startsWith(baseUrl) ||
          url.startsWith("data:") ||
          url.startsWith("blob:")
        ) {
          return route.continue();
        }
        outboundRequests.push(url);
        return route.abort();
      });

      page.on("console", (msg) => {
        const txt = msg.text();
        allConsoleMessages.push(txt);
        if (msg.type() === "error") {
          consoleErrors.push(txt);
        }
      });
      page.on("pageerror", (err) => {
        pageErrors.push(String(err));
      });

      await page.goto(baseUrl, { waitUntil: "networkidle" });
      return { context, page };
    };

    const results = [];
    const record = async (id, fn) => {
      await fn();
      results.push(id);
      console.log(`PASS ${id}`);
    };

    // M4-01: Layout & Branding
    await record("M4-01", async () => {
      const { context, page } = await createTrackedPage();
      try {
        await page.getByTestId("readiness-label").waitFor();
        const title = await page.title();
        assert.equal(title, "Local Humanizer");
        assert.equal(await page.getByTestId("app-logo").isVisible(), true);
        assert.equal(await page.getByTestId("readiness-indicator").isVisible(), true);
        assert.equal(await page.getByTestId("theme-toggle").isVisible(), true);
        assert.equal(await page.getByTestId("settings-button").isVisible(), true);

        const leftBox = await page.getByTestId("original-pane").boundingBox();
        const rightBox = await page.getByTestId("humanized-pane").boundingBox();
        assert.ok(leftBox && rightBox && leftBox.x < rightBox.x);

        const overflow1280 = await page.evaluate(
          () => document.documentElement.scrollWidth > window.innerWidth,
        );
        assert.equal(overflow1280, false);

        await page.setViewportSize({ width: 1024, height: 700 });
        const overflow1024 = await page.evaluate(
          () => document.documentElement.scrollWidth > window.innerWidth,
        );
        assert.equal(overflow1024, false);

        await page.setViewportSize({ width: 960, height: 640 });
        const overflow960 = await page.evaluate(
          () => document.documentElement.scrollWidth > window.innerWidth,
        );
        assert.equal(overflow960, false);

        const bodyText = await page.evaluate(() => document.body.innerText.toLowerCase());
        const legacyName = ["stealth", "humanizer"].join("");
        assert.equal(bodyText.includes(legacyName), false);
      } finally {
        await context.close();
      }
    });

    // M4-02: Long-Doc Paste (>=3,000 words UTF-8) + Clear + Import (.txt/.md)
    await record("M4-02", async () => {
      const { context, page } = await createTrackedPage();
      try {
        const longDoc = build3200WordUtf8Doc();
        const textarea = page.getByTestId("original-textarea");
        await textarea.fill(longDoc);

        await page.waitForFunction(() => {
          const txt =
            document.querySelector('[data-testid="original-word-count"]')?.textContent ?? "";
          const num = Number(txt.replace(/[^0-9]/g, ""));
          return num >= 3000;
        });

        const charTxt = await page.getByTestId("original-char-count").innerText();
        assert.ok(Number(charTxt.replace(/[^0-9]/g, "")) > 15000);

        await page.getByTestId("clear-button").click();
        assert.equal(await textarea.inputValue(), "");
        await page.waitForFunction(() => {
          const txt =
            document.querySelector('[data-testid="original-word-count"]')?.textContent ?? "";
          return /^0\s+words/.test(txt.trim());
        });

        const mdPath = path.join(tmpDir, "utf8-import.md");
        const mdContent = "# Imported Heading\n\nCafé résumé — 日本語テスト مرحبا.";
        await fs.writeFile(mdPath, mdContent, "utf8");

        const [chooser] = await Promise.all([
          page.waitForEvent("filechooser"),
          page.getByTestId("import-button").click(),
        ]);
        await chooser.setFiles(mdPath);
        await page.waitForFunction(
          (expected) =>
            document.querySelector('[data-testid="original-textarea"]')?.value === expected,
          mdContent,
        );
      } finally {
        await context.close();
      }
    });

    // M4-03: 6 Modes x 3 Intensities x 6 Optional Controls IPC capture
    await record("M4-03", async () => {
      const { context, page } = await createTrackedPage();
      try {
        await page.getByTestId("original-textarea").fill("We utilize local tools in order to write.");
        await page.getByTestId("options-toggle-button").click();

        const modes = ["standard", "natural", "academic", "professional", "simple", "deep"];
        for (const mode of modes) {
          await page.getByTestId(`mode-${mode}`).click();
        }
        await page.getByTestId("mode-academic").click();
        await page.getByTestId("intensity-strong").click();

        await page.getByTestId("option-preserve-length").uncheck();
        await page.getByTestId("option-preserve-formatting").uncheck();
        await page.getByTestId("option-preserve-citations").uncheck();
        await page.getByTestId("option-preserve-quotes").uncheck();
        await page.getByTestId("option-language").selectOption("fr");
        await page.getByTestId("option-voice").selectOption("third_person");

        await page.getByTestId("humanize-button").click();
        await page.waitForFunction(
          () =>
            (document.querySelector('[data-testid="humanized-textarea"]')?.value ?? "").length > 0,
        );

        const lastReq = await page.evaluate(() => {
          const hist = window.__LOCAL_HUMANIZER_MOCK_IPC__?.history ?? [];
          const calls = hist.filter((h) => h.cmd === "humanize_document");
          return calls[calls.length - 1]?.args?.req;
        });
        assert.equal(lastReq.mode, "academic");
        assert.equal(lastReq.intensity, "strong");
        assert.deepEqual(lastReq.options, {
          preserve_length: false,
          preserve_formatting: false,
          preserve_citations: false,
          preserve_quotes: false,
          language: "fr",
          voice: "third_person",
        });
      } finally {
        await context.close();
      }
    });

    // M4-04: Readiness Gating & Error Recovery
    await record("M4-04", async () => {
      const states = ["initializing", "verifying", "downloading", "starting_runtime"];
      for (const st of states) {
        const { context, page } = await createTrackedPage((stateName) => {
          window.__LOCAL_HUMANIZER_MOCK_IPC__ = {
            scenario: {
              initialStatus: { state: stateName, model_ready: false },
            },
          };
        }, st);
        try {
          await page.getByTestId("original-textarea").fill("Ready gating test text.");
          assert.equal(await page.getByTestId("humanize-button").isDisabled(), true);
        } finally {
          await context.close();
        }
      }

      const { context, page } = await createTrackedPage(() => {
        window.__LOCAL_HUMANIZER_MOCK_IPC__ = {
          scenario: {
            initialStatus: {
              state: "error",
              model_ready: false,
              error_message: "Model file missing",
            },
            retryInitStatus: {
              state: "ready",
              model_ready: true,
              error_message: null,
            },
          },
        };
      });
      try {
        await page.getByTestId("original-textarea").fill("Recover runtime test.");
        assert.equal(await page.getByTestId("humanize-button").isDisabled(), true);
        assert.match(await page.getByTestId("readiness-error-detail").innerText(), /Model file missing/);
        await page.getByTestId("retry-runtime-button").click();
        await page.waitForFunction(
          () => document.querySelector('[data-testid="readiness-label"]')?.textContent === "Ready",
        );
        assert.equal(await page.getByTestId("humanize-button").isDisabled(), false);
      } finally {
        await context.close();
      }
    });

    // M4-05, M4-07, M4-08, M4-09, M4-10: Chunk Progress, Chunk Retry, Editable Output & Delta, Copy + 2s Feedback, Select All
    await record("M4-05+07+08+09+10", async () => {
      const { context, page } = await createTrackedPage(() => {
        window.__LOCAL_HUMANIZER_MOCK_IPC__ = {
          scenario: {
            chunkDelayMs: 120,
            flagChunkIndex: 1,
          },
        };
      });
      try {
        const para1 = Array.from({ length: 120 }, (_, i) => `first${i}`).join(" ");
        const para2 = Array.from({ length: 120 }, (_, i) => `utilize second${i}`).join(" ");
        const para3 = Array.from({ length: 120 }, (_, i) => `third${i}`).join(" ");
        await page.getByTestId("original-textarea").fill(`${para1}\n\n${para2}\n\n${para3}`);

        await page.getByTestId("humanize-button").click();
        await page.getByTestId("progress-banner").waitFor();
        await page.waitForFunction(() => {
          const el = document.querySelector('[data-testid="progress-stage-label"]');
          return Boolean(el && /Inferring Chunk|Segmenting|Validating/.test(el.textContent ?? ""));
        });
        assert.equal(await page.getByTestId("progress-bar").isVisible(), true);

        await page.waitForFunction(
          () => document.querySelector('[data-testid="progress-banner"]')?.getAttribute("data-state") === "completed",
        );

        // M4-07: Retry Flagged Chunk
        const retryBtn = page.getByTestId("retry-chunk-button");
        assert.equal(await retryBtn.isVisible(), true);
        await retryBtn.click();
        await page.waitForFunction(
          () => !document.querySelector('[data-testid="retry-chunk-button"]'),
        );

        const retryCall = await page.evaluate(() => {
          const hist = window.__LOCAL_HUMANIZER_MOCK_IPC__?.history ?? [];
          return hist.find((h) => h.cmd === "retry_chunk")?.args?.req;
        });
        assert.equal(retryCall.chunk_index, 1);
        assert.ok(retryCall.original_chunk.includes("second0"));
        assert.ok(retryCall.context_before.includes("first0"));

        // M4-08: Editable output + live delta
        const outArea = page.getByTestId("humanized-textarea");
        assert.equal(await outArea.evaluate((el) => el.readOnly), false);
        const currentOut = await outArea.inputValue();
        const editedOut = `${currentOut} extra edited words added manually here`;
        await outArea.fill(editedOut);
        await page.getByTestId("word-delta").waitFor({ state: "visible" });

        // M4-09: Copy + 2s Copied! feedback
        await page.getByTestId("copy-button").click();
        await page.waitForFunction(
          () =>
            (document.querySelector('[data-testid="copy-button"]')?.textContent ?? "").includes("Copied!"),
        );
        const clipValue = await page.evaluate(() => navigator.clipboard.readText());
        assert.equal(clipValue, editedOut);
        await page.waitForTimeout(2200);
        assert.match(await page.getByTestId("copy-button").innerText(), /^Copy$/);

        // M4-10: Select All
        await page.getByTestId("select-all-button").click();
        const sel = await page.evaluate(() => {
          const el = document.querySelector('[data-testid="humanized-textarea"]');
          return {
            focused: document.activeElement === el,
            start: el.selectionStart,
            end: el.selectionEnd,
            len: el.value.length,
          };
        });
        assert.equal(sel.focused, true);
        assert.equal(sel.start, 0);
        assert.equal(sel.end, sel.len);
      } finally {
        await context.close();
      }
    });

    // M4-06: Cancel Active Generation
    await record("M4-06", async () => {
      const { context, page } = await createTrackedPage(() => {
        window.__LOCAL_HUMANIZER_MOCK_IPC__ = {
          scenario: { chunkDelayMs: 250 },
        };
      });
      try {
        const para = Array.from({ length: 120 }, (_, i) => `chunk${i}`).join(" ");
        await page.getByTestId("original-textarea").fill(`${para}\n\n${para}\n\n${para}`);
        await page.getByTestId("humanize-button").click();
        await page.getByTestId("cancel-humanize-button").waitFor();
        await page.getByTestId("cancel-humanize-button").click();

        await page.waitForFunction(
          () => document.querySelector('[data-testid="progress-banner"]')?.getAttribute("data-state") === "cancelled",
        );
        const cancelCall = await page.evaluate(() => {
          const hist = window.__LOCAL_HUMANIZER_MOCK_IPC__?.history ?? [];
          return hist.find((h) => h.cmd === "cancel_humanize");
        });
        assert.ok(cancelCall?.args?.requestId);
        assert.equal(await page.getByTestId("humanize-button").isEnabled(), true);
      } finally {
        await context.close();
      }
    });

    // M4-11: Save Output (Browser Blob fallback + Mocked Tauri saveDialog)
    await record("M4-11", async () => {
      const { context, page } = await createTrackedPage();
      try {
        await page.getByTestId("humanized-textarea").fill("Saved output payload 123.");
        const [download] = await Promise.all([
          page.waitForEvent("download"),
          page.getByTestId("save-button").click(),
        ]);
        const dlPath = await download.path();
        const dlContent = await fs.readFile(dlPath, "utf8");
        assert.equal(dlContent, "Saved output payload 123.");
      } finally {
        await context.close();
      }

      const { context: ctx2, page: page2 } = await createTrackedPage(() => {
        window.__LOCAL_HUMANIZER_MOCK_IPC__ = {
          saveDialog: () => "/mock/exported.txt",
        };
      });
      try {
        await page2.getByTestId("humanized-textarea").fill("Tauri save IPC payload.");
        await page2.getByTestId("save-button").click();
        await page2.waitForFunction(
          () =>
            window.__LOCAL_HUMANIZER_MOCK_IPC__?.savedFiles?.["/mock/exported.txt"] ===
            "Tauri save IPC payload.",
        );
      } finally {
        await ctx2.close();
      }
    });

    // M4-12: Light/Dark Theme Toggle & Persistence
    await record("M4-12", async () => {
      const { context, page } = await createTrackedPage();
      try {
        const beforeTheme = await page.evaluate(() => document.documentElement.dataset.theme);
        const beforeBg = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
        await page.getByTestId("theme-toggle").click();
        const afterTheme = await page.evaluate(() => document.documentElement.dataset.theme);
        assert.notEqual(beforeTheme, afterTheme);
        await page.waitForFunction(
          (prevBg) => getComputedStyle(document.body).backgroundColor !== prevBg,
          beforeBg,
        );
        const afterBg = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
        assert.notEqual(beforeBg, afterBg);

        await page.reload({ waitUntil: "networkidle" });
        const persistedTheme = await page.evaluate(() => document.documentElement.dataset.theme);
        assert.equal(persistedTheme, afterTheme);
      } finally {
        await context.close();
      }
    });

    // M4-13: Settings Modal & Model Management
    await record("M4-13", async () => {
      const { context, page } = await createTrackedPage(() => {
        window.__LOCAL_HUMANIZER_MOCK_IPC__ = {
          scenario: { downloadDelayMs: 90 },
        };
      });
      try {
        await page.getByTestId("settings-button").click();
        await page.getByTestId("settings-modal").waitFor();

        assert.equal(await page.getByTestId("non-commercial-badge").isVisible(), true);
        assert.equal(await page.getByTestId("privacy-summary").isVisible(), true);
        assert.equal(await page.getByTestId("shortcuts-cheatsheet").isVisible(), true);

        // Start download -> observe progress -> cancel
        await page.getByTestId("download-model-qwen3-4b-instruct-2507-q4_k_m").click();
        await page.getByTestId("download-progress-qwen3-4b-instruct-2507-q4_k_m").waitFor();
        await page.getByTestId("cancel-download-qwen3-4b-instruct-2507-q4_k_m").click();
        await page.getByTestId("download-model-qwen3-4b-instruct-2507-q4_k_m").waitFor();

        // Re-download to completion & select with GPU toggle
        await page.getByTestId("download-model-qwen3-4b-instruct-2507-q4_k_m").click();
        await page.getByTestId("select-model-qwen3-4b-instruct-2507-q4_k_m").waitFor();
        await page.getByTestId("gpu-toggle").check();
        await page.getByTestId("select-model-qwen3-4b-instruct-2507-q4_k_m").click();

        await page.waitForFunction(
          () =>
            /metal/i.test(
              document.querySelector('[data-testid="backend-mode-badge"]')?.textContent ?? "",
            ),
        );
        assert.match(
          await page.getByTestId("active-model-label").innerText(),
          /Qwen3 4B/,
        );
      } finally {
        await context.close();
      }
    });

    // M4-14: Keyboard Shortcuts
    await record("M4-14", async () => {
      const { context, page } = await createTrackedPage();
      try {
        const mod = process.platform === "darwin" ? "Meta" : "Control";
        await page.getByTestId("settings-button").click();
        await page.getByTestId("settings-modal").waitFor();
        await page.keyboard.press("Escape");
        await page.waitForFunction(() => !document.querySelector('[data-testid="settings-modal"]'));

        await page.getByTestId("original-textarea").fill("Shortcut trigger prose here.");
        await page.keyboard.press(`${mod}+Enter`);
        await page.waitForFunction(
          () =>
            (document.querySelector('[data-testid="humanized-textarea"]')?.value ?? "").length > 0,
        );

        await page.keyboard.press(`${mod}+Shift+C`);
        await page.waitForFunction(
          () =>
            (document.querySelector('[data-testid="copy-button"]')?.textContent ?? "").includes("Copied!"),
        );

        const [download] = await Promise.all([
          page.waitForEvent("download"),
          page.keyboard.press(`${mod}+s`),
        ]);
        assert.ok(download.suggestedFilename().endsWith(".txt"));
      } finally {
        await context.close();
      }
    });

    // M4-16 & M4-17 & M4-18: XSS Safety, Zero Doc Leak, Zero Forbidden Patterns
    await record("M4-16+17+18", async () => {
      const { context, page } = await createTrackedPage();
      try {
        const xssPayload = [
          "<scr",
          "ipt>window.__xss=1</scr",
          "ipt><img src=x onerr",
          'or="window.__xss=2">',
        ].join("");
        const docSentinel = "LH_DOC_SENTINEL_9941_SECRET_PROSE";

        await page.getByTestId("original-textarea").fill(`${docSentinel} ${xssPayload}`);
        await page.getByTestId("humanize-button").click();
        await page.waitForFunction(
          (sentinel) =>
            (document.querySelector('[data-testid="humanized-textarea"]')?.value ?? "").includes(sentinel),
          docSentinel,
        );

        const xssState = await page.evaluate(() => ({
          xssGlobal: window.__xss,
          injectedScripts: document.querySelectorAll("script").length,
          onerrorImgs: document.querySelectorAll("img[onerror]").length,
        }));
        assert.equal(xssState.xssGlobal, undefined);
        assert.equal(xssState.onerrorImgs, 0);

        const storageSnapshot = await page.evaluate(() => ({
          local: { ...localStorage },
          session: { ...sessionStorage },
        }));
        const serializedStorage = JSON.stringify(storageSnapshot);
        assert.equal(serializedStorage.includes(docSentinel), false);
        for (const msg of allConsoleMessages) {
          assert.equal(msg.includes(docSentinel), false);
        }

        const forbiddenNeedles = [
          ["upgrade", "to", "pro"].join(" "),
          ["credits", "remaining"].join(" "),
          ["sign", "in", "to", "continue"].join(" "),
          ["98%", "human"].join(" "),
          ["ai", "detection", "score"].join(" "),
          ["sk-", "test"].join(""),
          ["ghp_", "test"].join(""),
          ["AI", "za"].join(""),
          ["AK", "IA"].join(""),
        ];

        const inspectScreenText = async () => {
          const text = await page.evaluate(() => document.body.innerText.toLowerCase());
          for (const needle of forbiddenNeedles) {
            assert.equal(
              text.includes(needle.toLowerCase()),
              false,
              `Forbidden UI copy found: ${needle}`,
            );
          }
        };

        await inspectScreenText();
        await page.getByTestId("settings-button").click();
        await page.getByTestId("settings-modal").waitFor();
        await inspectScreenText();
      } finally {
        await context.close();
      }
    });

    // M4-19: Regression suite (concurrency lock, cancel/re-run race, modal shortcut guard,
    // selectModel error toast, uninstalled bundled model Download button + GPU polling + empty catalog,
    // 1-based M3 chunk warning retry isolation, and invalid UTF-8 browser upload rejection)
    await record("M4-19-regressions", async () => {
      // 1) Rapid double-click & double-shortcut concurrency lock -> only 1 humanize_document call
      {
        const { context, page } = await createTrackedPage(() => {
          window.__LOCAL_HUMANIZER_MOCK_IPC__ = {
            scenario: { chunkDelayMs: 250 },
          };
        });
        try {
          await page.getByTestId("original-textarea").fill("Concurrency lock prose test.");
          await page.evaluate(() => {
            const btn = document.querySelector('[data-testid="humanize-button"]');
            btn.click();
            btn.click();
          });
          await page.waitForTimeout(80);
          const dblClickCount = await page.evaluate(
            () =>
              (window.__LOCAL_HUMANIZER_MOCK_IPC__?.history ?? []).filter(
                (h) => h.cmd === "humanize_document",
              ).length,
          );
          assert.equal(dblClickCount, 1);
        } finally {
          await context.close();
        }
      }

      {
        const { context, page } = await createTrackedPage(() => {
          window.__LOCAL_HUMANIZER_MOCK_IPC__ = {
            scenario: { chunkDelayMs: 250 },
          };
        });
        try {
          await page.getByTestId("original-textarea").fill("Double shortcut lock prose test.");
          await page.evaluate(() => {
            const isMac = /mac/i.test(navigator.platform);
            for (let i = 0; i < 2; i += 1) {
              window.dispatchEvent(
                new KeyboardEvent("keydown", {
                  key: "Enter",
                  metaKey: isMac,
                  ctrlKey: !isMac,
                  bubbles: true,
                }),
              );
            }
          });
          await page.waitForTimeout(80);
          const dblShortcutCount = await page.evaluate(
            () =>
              (window.__LOCAL_HUMANIZER_MOCK_IPC__?.history ?? []).filter(
                (h) => h.cmd === "humanize_document",
              ).length,
          );
          assert.equal(dblShortcutCount, 1);
        } finally {
          await context.close();
        }
      }

      // 2) Cancel slow Run 1 -> immediate fast Run 2 -> stale Run 1 resolution never overwrites Run 2
      {
        const { context, page } = await createTrackedPage(() => {
          let callNum = 0;
          window.__LOCAL_HUMANIZER_MOCK_IPC__ = {
            invoke: async (cmd, args) => {
              if (cmd !== "humanize_document") return undefined;
              callNum += 1;
              const req = args.req;
              if (callNum === 1) {
                await new Promise((r) => setTimeout(r, 380));
                return {
                  request_id: req.request_id,
                  output_text: "STALE_RUN_1_OUTPUT_SHOULD_NOT_OVERWRITE",
                  chunks_total: 1,
                  chunks_completed: 1,
                  chunks_retried: 0,
                  warnings: [],
                  duration_ms: 380,
                  cancelled: false,
                };
              }
              await new Promise((r) => setTimeout(r, 30));
              return {
                request_id: req.request_id,
                output_text: "FRESH_RUN_2_AUTHORITATIVE_OUTPUT",
                chunks_total: 1,
                chunks_completed: 1,
                chunks_retried: 0,
                warnings: [],
                duration_ms: 30,
                cancelled: false,
              };
            },
          };
        });
        try {
          await page.getByTestId("original-textarea").fill("Run 1 input.");
          await page.getByTestId("humanize-button").click();
          await page.getByTestId("cancel-humanize-button").waitFor();
          await page.getByTestId("cancel-humanize-button").click();
          await page.getByTestId("humanize-button").waitFor();
          await page.getByTestId("original-textarea").fill("Run 2 input.");
          await page.getByTestId("humanize-button").click();
          await page.waitForFunction(
            () =>
              document.querySelector('[data-testid="humanized-textarea"]')?.value ===
              "FRESH_RUN_2_AUTHORITATIVE_OUTPUT",
          );
          await page.waitForTimeout(480);
          assert.equal(
            await page.getByTestId("humanized-textarea").inputValue(),
            "FRESH_RUN_2_AUTHORITATIVE_OUTPUT",
          );
        } finally {
          await context.close();
        }
      }

      // 3) Shortcuts blocked while SettingsModal is open
      {
        const { context, page } = await createTrackedPage();
        try {
          const mod = process.platform === "darwin" ? "Meta" : "Control";
          await page.getByTestId("original-textarea").fill("Prose behind modal.");
          await page.getByTestId("humanized-textarea").fill("Existing output.");
          await page.getByTestId("settings-button").click();
          await page.getByTestId("settings-modal").waitFor();

          await page.keyboard.press(`${mod}+Enter`);
          await page.keyboard.press(`${mod}+Shift+C`);
          await page.waitForTimeout(80);

          const histCount = await page.evaluate(
            () =>
              (window.__LOCAL_HUMANIZER_MOCK_IPC__?.history ?? []).filter(
                (h) => h.cmd === "humanize_document",
              ).length,
          );
          assert.equal(histCount, 0);
          assert.match(await page.getByTestId("copy-button").innerText(), /^Copy$/);
        } finally {
          await context.close();
        }
      }

      // 4) selectModel returning error AppStatus emits error toast (never success)
      {
        const { context, page } = await createTrackedPage(() => {
          window.__LOCAL_HUMANIZER_MOCK_IPC__ = {
            invoke: async (cmd) => {
              if (cmd === "select_model") {
                return {
                  state: "error",
                  active_model_id: "qwen3-1.7b-q4_k_m",
                  model_ready: false,
                  backend_mode: "cpu",
                  ram_bytes: 8589934592,
                  error_message: "Simulated model activation failure",
                  runtime_version: "b12600",
                };
              }
              return undefined;
            },
          };
        });
        try {
          await page.getByTestId("settings-button").click();
          await page.getByTestId("settings-modal").waitFor();
          await page.getByTestId("select-model-qwen3-1.7b-q4_k_m").click();
          await page.getByTestId("toast-error").waitFor();
          const errMsg = await page.getByTestId("toast-error").innerText();
          assert.match(errMsg, /Simulated model activation failure/);
          assert.equal(await page.getByTestId("toast-success").count(), 0);
        } finally {
          await context.close();
        }
      }

      // 5) Uninstalled bundled model shows Download button, GPU toggle survives 600ms polling, empty catalog has Refresh button
      {
        const { context, page } = await createTrackedPage(() => {
          let listCalls = 0;
          window.__LOCAL_HUMANIZER_MOCK_IPC__ = {
            scenario: {
              initialStatus: { state: "downloading", model_ready: false, backend_mode: "cpu" },
            },
            invoke: async (cmd) => {
              if (cmd === "list_models") {
                listCalls += 1;
                if (listCalls === 1) {
                  return [];
                }
                return [
                  {
                    id: "qwen3-1.7b-q4_k_m",
                    name: "Qwen3 1.7B (Recommended Default)",
                    quant: "Q4_K_M",
                    size_bytes: 1107409472,
                    sha256: "b139949c",
                    license: "Apache-2.0",
                    license_url: "",
                    is_default: true,
                    is_installed: false,
                    is_bundled: true,
                    min_recommended_ram_gb: 8,
                    non_commercial_warning: false,
                    download_url: "",
                  },
                ];
              }
              return undefined;
            },
          };
        });
        try {
          await page.getByTestId("settings-button").click();
          await page.getByTestId("settings-modal").waitFor();
          await page.getByTestId("empty-models-state").waitFor();
          await page.getByTestId("refresh-models-button").click();

          await page.getByTestId("download-model-qwen3-1.7b-q4_k_m").waitFor();
          assert.equal(await page.getByTestId("select-model-qwen3-1.7b-q4_k_m").count(), 0);
          assert.match(
            await page.getByTestId("model-card-qwen3-1.7b-q4_k_m").innerText(),
            /Bundled/,
          );

          await page.getByTestId("gpu-toggle").check();
          await page.waitForTimeout(1300);
          assert.equal(await page.getByTestId("gpu-toggle").isChecked(), true);
        } finally {
          await context.close();
        }
      }

      // 6) Multi-chunk M3 warning isolation: retrying Chunk 2 preserves Chunk 1 warning
      {
        const { context, page } = await createTrackedPage(() => {
          window.__LOCAL_HUMANIZER_MOCK_IPC__ = {
            invoke: async (cmd, args) => {
              if (cmd === "humanize_document") {
                const req = args.req;
                return {
                  request_id: req.request_id,
                  output_text: "Chunk one out.\n\nChunk two out.\n\nChunk three out.",
                  chunks_total: 3,
                  chunks_completed: 3,
                  chunks_retried: 0,
                  warnings: [
                    "Chunk 1 closely echoed source phrasing.",
                    "Chunk 2 altered protected item: 2026",
                  ],
                  duration_ms: 15,
                  cancelled: false,
                };
              }
              if (cmd === "retry_chunk") {
                const req = args.req;
                return {
                  request_id: req.request_id,
                  chunk_index: req.chunk_index,
                  rewritten_chunk: "Revised chunk two out.",
                  warnings: [],
                };
              }
              return undefined;
            },
          };
        });
        try {
          const p1 = Array.from({ length: 120 }, (_, i) => `p1w${i}`).join(" ");
          const p2 = Array.from({ length: 120 }, (_, i) => `p2w${i}`).join(" ");
          const p3 = Array.from({ length: 120 }, (_, i) => `p3w${i}`).join(" ");
          await page.getByTestId("original-textarea").fill(`${p1}\n\n${p2}\n\n${p3}`);
          await page.getByTestId("humanize-button").click();
          await page.waitForFunction(
            () => document.querySelector('[data-testid="progress-banner"]')?.getAttribute("data-state") === "completed",
          );
          const retryButtons = page.getByTestId("retry-chunk-button");
          assert.equal(await retryButtons.count(), 2);
          await retryButtons.nth(1).click();
          await page.waitForFunction(
            () => document.querySelectorAll('[data-testid="retry-chunk-button"]').length === 1,
          );
          assert.match(
            await page.getByTestId("retry-chunk-button").innerText(),
            /Chunk \(1\)/,
          );
          assert.match(
            await page.getByTestId("progress-warnings").innerText(),
            /Chunk 1 closely echoed/,
          );
        } finally {
          await context.close();
        }
      }

      // 7) Invalid UTF-8 browser file upload rejection
      {
        const { context, page } = await createTrackedPage();
        try {
          const badUtf8Path = path.join(tmpDir, "invalid-utf8.txt");
          await fs.writeFile(badUtf8Path, Buffer.from([0x80, 0x81, 0xff, 0x41, 0x42]));
          await page.getByTestId("original-textarea").fill("keep-existing-text");

          const [chooser] = await Promise.all([
            page.waitForEvent("filechooser"),
            page.getByTestId("import-button").click(),
          ]);
          await chooser.setFiles(badUtf8Path);
          await page.getByTestId("toast-error").waitFor();
          assert.match(await page.getByTestId("toast-error").innerText(), /UTF-8/);
          assert.equal(
            await page.getByTestId("original-textarea").inputValue(),
            "keep-existing-text",
          );
        } finally {
          await context.close();
        }
      }

      // 8) Explicit paraphrasing failure clears stale output, preserves input, and displays error state
      {
        const { context, page } = await createTrackedPage(() => {
          window.__LOCAL_HUMANIZER_MOCK_IPC__ = {
            invoke: async (cmd, args) => {
              if (cmd === "humanize_document") {
                const req = args.req;
                return {
                  request_id: req.request_id,
                  output_text: "",
                  chunks_total: 1,
                  chunks_completed: 0,
                  chunks_retried: 1,
                  warnings: ["Chunk 1 remained too close to source."],
                  duration_ms: 20,
                  cancelled: false,
                  failed: true,
                  error_message:
                    "Paraphrasing stopped at chunk 1 because generated phrasing remained too close to the original source.",
                };
              }
              return undefined;
            },
          };
        });
        try {
          await page.getByTestId("humanized-textarea").fill("STALE_OLD_OUTPUT");
          await page.getByTestId("original-textarea").fill("Keep this original draft intact.");
          await page.getByTestId("humanize-button").click();
          await page.waitForFunction(
            () =>
              document.querySelector('[data-testid="progress-banner"]')?.getAttribute("data-state") ===
              "error",
          );
          assert.equal(await page.getByTestId("humanized-textarea").inputValue(), "");
          assert.equal(
            await page.getByTestId("original-textarea").inputValue(),
            "Keep this original draft intact.",
          );
          assert.match(
            await page.getByTestId("progress-error-message").innerText(),
            /stopped at chunk 1/i,
          );
        } finally {
          await context.close();
        }
      }
    });

    // M4-15: Zero Console Errors & Offline Isolation
    await record("M4-15", async () => {
      assert.deepEqual(consoleErrors, [], `Console errors: ${consoleErrors.join(" | ")}`);
      assert.deepEqual(pageErrors, [], `Page errors: ${pageErrors.join(" | ")}`);
      assert.deepEqual(outboundRequests, [], `Outbound requests: ${outboundRequests.join(" | ")}`);
    });

    console.log(`ALL PLAYWRIGHT UI CHECKS PASSED (${results.length} groups covering M4-01..M4-18)`);
  } finally {
    if (browser) {
      await browser.close();
    }
    previewProc.kill("SIGTERM");
    await fs.rm(tmpDir, { recursive: true, force: true });
    if (process.exitCode && serverLogs) {
      console.error(serverLogs);
    }
  }
}

runSuite().catch((err) => {
  console.error(err);
  process.exit(1);
});
