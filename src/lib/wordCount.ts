const DASH_SPLIT_RE = /[\u2013\u2014]+/g;
const NON_BOM_WHITESPACE_RE = /[^\S\ufeff]+/;
const STRIP_PUNCT_RE = /^[`*_~"'“”‘’()[\]{}<>.,;:!?•·]+|[`*_~"'“”‘’()[\]{}<>.,;:!?•·]+$/g;
const HAS_LETTER_OR_NUMBER_RE = /[\p{L}\p{N}]/u;
const CJK_RE = /[\u4e00-\u9fff\u3400-\u4dbf\u3040-\u309f\u30a0-\u30ff]/g;
const NON_CJK_RUN_RE = /[^\u4e00-\u9fff\u3400-\u4dbf\u3040-\u309f\u30a0-\u30ff]+/g;

function stripPunct(token: string): string {
  return token.replace(STRIP_PUNCT_RE, "");
}

/**
 * Count words accurately across English, accented Latin, CJK, RTL, and Markdown.
 * Matches `tests/e2e/metrics.py::count_words` semantics (including non-splitting on \ufeff).
 */
export function countWords(text: string): number {
  if (!text) return 0;
  const normalized = text.replace(DASH_SPLIT_RE, " ");
  if (!normalized) return 0;

  let total = 0;
  const rawTokens = normalized.split(NON_BOM_WHITESPACE_RE);
  for (let i = 0; i < rawTokens.length; i += 1) {
    const tok = stripPunct(rawTokens[i]);
    if (!tok || !HAS_LETTER_OR_NUMBER_RE.test(tok)) {
      continue;
    }
    const cjkMatches = tok.match(CJK_RE);
    if (cjkMatches && cjkMatches.length > 0) {
      total += cjkMatches.length;
      const nonCjkRuns = tok.match(NON_CJK_RUN_RE);
      if (nonCjkRuns) {
        for (let j = 0; j < nonCjkRuns.length; j += 1) {
          const cleaned = stripPunct(nonCjkRuns[j]);
          if (cleaned && HAS_LETTER_OR_NUMBER_RE.test(cleaned)) {
            total += 1;
          }
        }
      }
    } else {
      total += 1;
    }
  }
  return total;
}

/**
 * Count Unicode codepoints (matching Python `len(str)`).
 */
export function countChars(text: string): number {
  if (!text) return 0;
  let count = 0;
  for (let i = 0; i < text.length; i += 1) {
    const code = text.charCodeAt(i);
    if (code >= 0xd800 && code <= 0xdbff && i + 1 < text.length) {
      const next = text.charCodeAt(i + 1);
      if (next >= 0xdc00 && next <= 0xdfff) {
        i += 1;
      }
    }
    count += 1;
  }
  return count;
}

export function utf8ByteLength(text: string): number {
  if (!text) return 0;
  return new TextEncoder().encode(text).length;
}

export interface WordDelta {
  deltaWords: number;
  deltaPercent: number;
  formatted: string;
}

export function computeWordDeltaPercent(
  inputWords: number,
  outputWords: number,
): WordDelta | null {
  if (inputWords <= 0 || outputWords <= 0) {
    return null;
  }
  const deltaWords = outputWords - inputWords;
  const deltaPercent = Math.round((deltaWords / inputWords) * 100);
  const sign = deltaPercent > 0 ? "+" : "";
  return {
    deltaWords,
    deltaPercent,
    formatted: `${sign}${deltaPercent}%`,
  };
}

const HARD_CAP_WORDS = 350;
const KNOWN_ABBREVS = [
  "dr.",
  "mr.",
  "ms.",
  "mrs.",
  "prof.",
  "sr.",
  "jr.",
  "vs.",
  "fig.",
  "no.",
  "pp.",
  "vol.",
  "inc.",
  "ltd.",
  "corp.",
  "st.",
  "jan.",
  "feb.",
  "mar.",
  "apr.",
  "aug.",
  "sept.",
  "sep.",
  "oct.",
  "nov.",
  "dec.",
  "et al.",
  "e.g.",
  "i.e.",
  "u.s.",
  "u.k.",
  "cf.",
  "approx.",
  "dept.",
  "est.",
  "rev.",
  "ed.",
];

function detectFenceOpen(trimmedStart: string): { ch: string; len: number } | null {
  const first = trimmedStart[0];
  if (first !== "`" && first !== "~") {
    return null;
  }
  let count = 0;
  while (count < trimmedStart.length && trimmedStart[count] === first) {
    count += 1;
  }
  if (count >= 3) {
    const rest = trimmedStart.slice(count);
    if (first === "`" && rest.includes("`")) {
      return null;
    }
    return { ch: first, len: count };
  }
  return null;
}

function isFenceClose(trimmed: string, openCh: string, openLen: number): boolean {
  let count = 0;
  while (count < trimmed.length && trimmed[count] === openCh) {
    count += 1;
  }
  return count >= openLen && trimmed.slice(count).trim().length === 0;
}

function extractProseParagraphs(normalized: string): string[] {
  const paragraphs: string[] = [];
  let proseLines: string[] = [];
  let inFence: { ch: string; len: number } | null = null;
  let inMath = false;

  const flushProse = () => {
    if (proseLines.length === 0) return;
    const joined = proseLines.join("\n").trim();
    proseLines = [];
    if (joined.length > 0) {
      paragraphs.push(joined);
    }
  };

  for (const line of normalized.split("\n")) {
    const trimmedStart = line.trimStart();
    const trimmed = line.trim();

    if (inFence) {
      if (isFenceClose(trimmed, inFence.ch, inFence.len)) {
        inFence = null;
      }
      continue;
    }

    if (inMath) {
      if (trimmed === "$$") {
        inMath = false;
      }
      continue;
    }

    const fence = detectFenceOpen(trimmedStart);
    if (fence) {
      flushProse();
      inFence = fence;
      continue;
    }

    if (trimmed === "$$") {
      flushProse();
      inMath = true;
      continue;
    }

    if (trimmed.length === 0) {
      flushProse();
    } else {
      proseLines.push(line);
    }
  }

  if (!inFence && !inMath) {
    flushProse();
  }

  return paragraphs;
}

function endsWithKnownAbbreviation(prefix: string): boolean {
  const trimmed = prefix.trimEnd();
  const lower = trimmed.toLowerCase();
  if (KNOWN_ABBREVS.some((a) => lower.endsWith(a))) {
    return true;
  }
  if (trimmed.length >= 2 && trimmed.endsWith(".")) {
    const prev = trimmed[trimmed.length - 2];
    const before = trimmed.length > 2 ? trimmed[trimmed.length - 3] : "";
    if (/[A-Z]/.test(prev) && (trimmed.length === 2 || !/[A-Za-z0-9]/.test(before))) {
      return true;
    }
  }
  return false;
}

function splitSentencesSafe(text: string): string[] {
  if (!text.trim()) return [];
  const protectedRanges: Array<[number, number]> = [];
  const urlRe = /https?:\/\/\S+/gi;
  let match: RegExpExecArray | null;
  while ((match = urlRe.exec(text)) !== null) {
    protectedRanges.push([match.index, match.index + match[0].length]);
  }

  const isInsideProtected = (pos: number) =>
    protectedRanges.some(([s, e]) => pos > s && pos < e);

  const sentences: string[] = [];
  let start = 0;

  for (let i = 0; i < text.length; i += 1) {
    const ch = text[i];
    if (ch !== "." && ch !== "!" && ch !== "?" && ch !== "。" && ch !== "！" && ch !== "？") {
      continue;
    }
    const endPunct = i + 1;
    if (isInsideProtected(i) || isInsideProtected(endPunct)) {
      continue;
    }
    if (ch === ".") {
      const prevDigit = i > 0 && /[0-9]/.test(text[i - 1]);
      const nextDigit = i + 1 < text.length && /[0-9]/.test(text[i + 1]);
      if (prevDigit && nextDigit) {
        continue;
      }
    }

    let splitAt = endPunct;
    while (splitAt < text.length && /["'”’)\]]/.test(text[splitAt])) {
      splitAt += 1;
    }

    const nextIsBoundary =
      splitAt >= text.length ||
      /\s/.test(text[splitAt]) ||
      ch === "。" ||
      ch === "！" ||
      ch === "？";
    if (!nextIsBoundary) {
      continue;
    }
    if (ch === "." && endsWithKnownAbbreviation(text.slice(start, endPunct))) {
      continue;
    }

    const piece = text.slice(start, splitAt).trim();
    if (piece.length > 0) {
      sentences.push(piece);
    }
    start = splitAt;
    while (start < text.length && /\s/.test(text[start])) {
      start += 1;
    }
    i = Math.max(i, start - 1);
  }

  if (start < text.length) {
    const tail = text.slice(start).trim();
    if (tail.length > 0) {
      sentences.push(tail);
    }
  }
  return sentences;
}

function splitGiantParagraph(text: string, targetWordsPerChunk: number): string[] {
  const sentences = splitSentencesSafe(text);
  const minTarget = Math.min(240, Math.floor(targetWordsPerChunk * 0.75));
  if (sentences.length === 0) {
    return [text];
  }

  const pieces: string[] = [];
  let buf: string[] = [];
  let bufWords = 0;

  const flushBuf = () => {
    if (buf.length > 0) {
      pieces.push(buf.join(" "));
      buf = [];
      bufWords = 0;
    }
  };

  for (const sent of sentences) {
    const sw = countWords(sent);
    if (sw > HARD_CAP_WORDS) {
      flushBuf();
      const words = sent.split(/\s+/).filter(Boolean);
      let cur: string[] = [];
      for (const w of words) {
        if (cur.length > 0 && cur.length >= targetWordsPerChunk) {
          pieces.push(cur.join(" "));
          cur = [];
        }
        cur.push(w);
      }
      if (cur.length > 0) {
        pieces.push(cur.join(" "));
      }
      continue;
    }

    if (
      buf.length > 0 &&
      bufWords + sw > targetWordsPerChunk &&
      (bufWords >= minTarget || bufWords + sw > HARD_CAP_WORDS)
    ) {
      flushBuf();
    }
    buf.push(sent);
    bufWords += sw;
  }

  flushBuf();
  return pieces.length > 0 ? pieces : [text];
}

/**
 * Segment document into paragraph-aware chunks matching backend chunking boundaries
 * (splitting giant >350-word paragraphs at sentence boundaries and skipping fenced
 * code blocks / `$$` display math) so client-side chunk retry aligns with M3 `chunks_total`.
 */
export function segmentParagraphChunks(
  text: string,
  targetWordsPerChunk = 320,
): string[] {
  const normalized = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").trim();
  if (!normalized) return [];

  const paragraphs = extractProseParagraphs(normalized);
  if (paragraphs.length === 0) return [];

  const chunks: string[] = [];
  let currentParts: string[] = [];
  let currentWords = 0;

  const flushCurrent = () => {
    if (currentParts.length > 0) {
      chunks.push(currentParts.join("\n\n"));
      currentParts = [];
      currentWords = 0;
    }
  };

  for (const para of paragraphs) {
    const paraWords = countWords(para);
    if (paraWords > HARD_CAP_WORDS) {
      flushCurrent();
      for (const sub of splitGiantParagraph(para, targetWordsPerChunk)) {
        chunks.push(sub);
      }
      continue;
    }
    if (
      currentParts.length > 0 &&
      currentWords + paraWords > targetWordsPerChunk
    ) {
      flushCurrent();
      currentParts = [para];
      currentWords = paraWords;
    } else {
      currentParts.push(para);
      currentWords += paraWords;
    }
  }

  flushCurrent();
  return chunks;
}
