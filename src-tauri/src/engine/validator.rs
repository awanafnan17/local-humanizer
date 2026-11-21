use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use regex::Regex;

use crate::commands::RewriteOptions;
use crate::engine::chunker::ChunkSpec;
use crate::engine::prompts::RetryDirective;
use crate::engine::protect::contains_with_boundary;
use crate::engine::segmenter::{count_paragraphs, count_words};

#[derive(Debug, Clone)]
pub struct SanitizeResult {
    pub text: String,
    pub think_leaked: bool,
    pub preamble_stripped: bool,
}

#[derive(Debug, Clone)]
pub struct ChunkValidation {
    pub chunk_index: usize,
    pub empty_flag: bool,
    pub truncation_flag: bool,
    pub expansion_flag: bool,
    pub paragraph_mismatch: bool,
    pub dangling_end: bool,
    pub unbalanced_delimiters: bool,
    pub doubled_words: bool,
    pub max_4gram_count: usize,
    pub verbatim_echo_flag: bool,
    pub lex_dist: f64,
    pub retention_ratio: f64,
    pub missing_invariants: Vec<String>,
    pub leaked_placeholder: bool,
    pub length_advisory: bool,
    pub src_words: usize,
    pub out_words: usize,
    pub src_paragraphs: usize,
    pub out_paragraphs: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RetryAction {
    Accept,
    EchoRetry,
    InvariantOrTruncRetry {
        missing: Vec<String>,
        length_hint: Option<String>,
    },
    CombinedRetry {
        missing: Vec<String>,
        length_hint: Option<String>,
    },
}

impl RetryAction {
    pub fn to_directive(&self) -> RetryDirective {
        match self {
            RetryAction::Accept => RetryDirective::None,
            RetryAction::EchoRetry => RetryDirective::EchoRetry,
            RetryAction::InvariantOrTruncRetry {
                missing,
                length_hint,
            } => RetryDirective::StrictInvariant {
                missing_items: missing.clone(),
                length_hint: length_hint.clone(),
            },
            RetryAction::CombinedRetry {
                missing,
                length_hint,
            } => RetryDirective::Combined {
                missing_items: missing.clone(),
                length_hint: length_hint.clone(),
            },
        }
    }
}

fn think_entity_open_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(?:&lt;|&#0*60;|&#x0*3c;|\\u003c)\s*think\s*(?:&gt;|&#0*62;|&#x0*3e;|\\u003e)")
            .unwrap()
    })
}

fn think_entity_close_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(?:&lt;|&#0*60;|&#x0*3c;|\\u003c)\s*/\s*think\s*(?:&gt;|&#0*62;|&#x0*3e;|\\u003e)")
            .unwrap()
    })
}

fn think_closed_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)<think(?:\s[^>]*)?>.*?</think\s*>").unwrap())
}

fn think_unclosed_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)<think(?:\s[^>]*)?>.*$").unwrap())
}

fn think_orphan_close_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)</think\s*>").unwrap())
}

fn preamble_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)^(?:sure,?\s+|okay,?\s+|certainly,?\s+|of course,?\s+)?(?:here(?:'s|’s| is| are)\s+(?:the\s+|a\s+)?(?:rewritten|paraphrased|humanized|revised|edited|updated)\s+(?:text|version|passage|paragraph|output|draft)|(?:rewritten|paraphrased|humanized|revised)\s+(?:text|version|output))\s*:\s*\n+",
        )
        .unwrap()
    })
}

fn strip_nested_think_tags(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    let mut depth = 0usize;
    let bytes = lower.as_bytes();

    while i < input.len() {
        if bytes[i..].starts_with(b"<think")
            && bytes
                .get(i + 6)
                .map(|b| *b == b'>' || b.is_ascii_whitespace())
                .unwrap_or(true)
        {
            if let Some(gt_rel) = input[i..].find('>') {
                depth += 1;
                i += gt_rel + 1;
                continue;
            } else {
                // Unclosed `<think` without `>` -> drop remainder
                break;
            }
        }
        if bytes[i..].starts_with(b"</think") {
            let mut k = i + 7;
            while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                k += 1;
            }
            if k < bytes.len() && bytes[k] == b'>' {
                depth = depth.saturating_sub(1);
                i = k + 1;
                continue;
            }
        }
        let ch = input[i..].chars().next().unwrap();
        if depth == 0 {
            out.push(ch);
        }
        i += ch.len_utf8();
    }
    out
}

pub fn sanitize_model_output(
    raw: &str,
    prev_context: Option<&str>,
    delimiter_tag: &str,
) -> SanitizeResult {
    let mut s = raw.replace("\r\n", "\n").replace('\r', "\n");

    let think_leaked = think_entity_open_re().is_match(&s)
        || think_entity_close_re().is_match(&s)
        || s.to_ascii_lowercase().contains("<think")
        || think_orphan_close_re().is_match(&s);

    if think_leaked {
        s = think_entity_open_re().replace_all(&s, "<think>").into_owned();
        s = think_entity_close_re().replace_all(&s, "</think>").into_owned();
        s = strip_nested_think_tags(&s);
        // Clean up any edge-case malformed tags
        s = think_closed_re().replace_all(&s, "").into_owned();
        s = think_unclosed_re().replace_all(&s, "").into_owned();
        s = think_orphan_close_re().replace_all(&s, "").into_owned();
    }

    // Strip echoed <SOURCE_TEXT_DATA...> delimiters
    let open_delim = format!("<{delimiter_tag}>");
    let close_delim = format!("</{delimiter_tag}>");
    s = s.replace(&open_delim, "").replace(&close_delim, "");
    if delimiter_tag != "SOURCE_TEXT_DATA" {
        s = s
            .replace("<SOURCE_TEXT_DATA>", "")
            .replace("</SOURCE_TEXT_DATA>", "");
    }

    let mut trimmed = s.trim().to_string();
    let mut preamble_stripped = false;
    if let Some(m) = preamble_re().find(&trimmed) {
        let rest = trimmed[m.end()..].trim();
        if !rest.is_empty() {
            trimmed = rest.to_string();
            preamble_stripped = true;
        }
    }

    // Strip accidental leading copy of prev_context
    if let Some(ctx) = prev_context {
        let ctx_trim = ctx.trim();
        if !ctx_trim.is_empty() && trimmed.starts_with(ctx_trim) {
            let after = trimmed[ctx_trim.len()..].trim_start();
            if !after.is_empty() {
                trimmed = after.to_string();
            }
        }
    }

    SanitizeResult {
        text: trimmed.trim().to_string(),
        think_leaked,
        preamble_stripped,
    }
}

fn tokenize_alnum_lower(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for lc in ch.to_lowercase() {
                cur.push(lc);
            }
        } else if !cur.is_empty() {
            tokens.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

pub fn lexical_distance(source: &str, output: &str) -> f64 {
    let src_tokens = tokenize_alnum_lower(source);
    let out_tokens = tokenize_alnum_lower(output);
    if src_tokens.is_empty() && out_tokens.is_empty() {
        return 0.0;
    }
    if src_tokens.is_empty() || out_tokens.is_empty() {
        return 1.0;
    }

    let src_uni: HashSet<&str> = src_tokens.iter().map(String::as_str).collect();
    let out_uni: HashSet<&str> = out_tokens.iter().map(String::as_str).collect();
    let uni_inter = src_uni.intersection(&out_uni).count() as f64;
    let uni_union = src_uni.union(&out_uni).count().max(1) as f64;
    let uni_dist = 1.0 - (uni_inter / uni_union);

    let src_bi: HashSet<(&str, &str)> = src_tokens
        .windows(2)
        .map(|w| (w[0].as_str(), w[1].as_str()))
        .collect();
    let out_bi: HashSet<(&str, &str)> = out_tokens
        .windows(2)
        .map(|w| (w[0].as_str(), w[1].as_str()))
        .collect();
    let bi_dist = if src_bi.is_empty() || out_bi.is_empty() {
        uni_dist
    } else {
        let bi_inter = src_bi.intersection(&out_bi).count() as f64;
        let bi_union = src_bi.union(&out_bi).count().max(1) as f64;
        1.0 - (bi_inter / bi_union)
    };

    let len_delta = (src_tokens.len() as f64 - out_tokens.len() as f64).abs()
        / (src_tokens.len().max(out_tokens.len()).max(1) as f64);

    (0.45 * uni_dist + 0.40 * bi_dist + 0.15 * len_delta).clamp(0.0, 1.0)
}

fn delimiter_imbalance(text: &str) -> (i32, i32, i32, i32) {
    let paren = text.matches('(').count() as i32 - text.matches(')').count() as i32;
    let brack = text.matches('[').count() as i32 - text.matches(']').count() as i32;
    let curly_q = text.matches('“').count() as i32 - text.matches('”').count() as i32;
    let straight_q = (text.matches('"').count() % 2) as i32;
    (paren.abs(), brack.abs(), curly_q.abs(), straight_q)
}

fn has_doubled_consecutive_words(text: &str) -> bool {
    let mut prev: Option<String> = None;
    for raw in text.split_whitespace() {
        let clean = raw
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_ascii_lowercase();
        if clean.len() < 2 || clean.chars().all(|c| c.is_ascii_digit()) {
            prev = None;
            continue;
        }
        if prev.as_deref() == Some(clean.as_str()) {
            return true;
        }
        prev = Some(clean);
    }
    false
}

fn max_4gram_frequency(text: &str) -> usize {
    let tokens = tokenize_alnum_lower(text);
    if tokens.len() < 20 {
        return 0;
    }
    let mut counts: HashMap<(&str, &str, &str, &str), usize> = HashMap::new();
    let mut max_c = 0usize;
    for w in tokens.windows(4) {
        let key = (w[0].as_str(), w[1].as_str(), w[2].as_str(), w[3].as_str());
        let c = counts.entry(key).or_insert(0);
        *c += 1;
        if *c > max_c {
            max_c = *c;
        }
    }
    max_c
}

fn ends_with_terminal_punct(s: &str) -> bool {
    matches!(
        s.trim_end().chars().next_back(),
        Some('.' | '!' | '?' | '”' | '"' | ')' | '。' | '！' | '？')
    )
}

fn has_dangling_end(source: &str, output: &str, out_words: usize) -> bool {
    if out_words < 8 || !ends_with_terminal_punct(source) {
        return false;
    }
    let trimmed = output.trim_end();
    if trimmed.ends_with(',') || trimmed.ends_with(';') || trimmed.ends_with(':') {
        return true;
    }
    if ends_with_terminal_punct(trimmed) {
        return false;
    }
    let last_word = trimmed
        .split_whitespace()
        .next_back()
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        last_word.as_str(),
        "and" | "or" | "but" | "the" | "a" | "an" | "of" | "to" | "in" | "for" | "with" | "that" | "which" | "because" | "while"
    )
}

pub fn validate_chunk(
    chunk: &ChunkSpec,
    output: &str,
    opts: &RewriteOptions,
) -> ChunkValidation {
    let src_words = chunk.word_count;
    let out_words = count_words(output);
    let empty_flag = output.trim().is_empty() || (src_words > 0 && out_words == 0);

    let (truncation_flag, expansion_flag, length_advisory) = if empty_flag {
        (true, false, false)
    } else if src_words <= 5 {
        let exp = out_words > (src_words * 4).max(12);
        (false, exp, false)
    } else {
        let ratio = out_words as f64 / src_words.max(1) as f64;
        if opts.preserve_length {
            let trunc = ratio < 0.70;
            let exp = ratio > 1.45;
            let adv = src_words >= 20 && !(0.80..=1.25).contains(&ratio);
            (trunc, exp, adv)
        } else {
            (ratio < 0.55, ratio > 1.75, false)
        }
    };

    let src_paragraphs = chunk.paragraph_count.max(1);
    let out_paragraphs = if empty_flag { 0 } else { count_paragraphs(output) };
    let paragraph_mismatch = opts.preserve_formatting && !empty_flag && out_paragraphs != src_paragraphs;

    let enforced = chunk.invariants.enforced_items(opts);
    let mut missing_invariants = Vec::new();
    let mut matched = 0usize;
    for &(cat, val, exact) in &enforced {
        if contains_with_boundary(val, output, exact) {
            matched += 1;
        } else {
            missing_invariants.push(format!("{cat}:{val}"));
        }
    }
    let retention_ratio = if enforced.is_empty() {
        1.0
    } else {
        matched as f64 / enforced.len() as f64
    };

    let src_bal = delimiter_imbalance(&chunk.text);
    let out_bal = delimiter_imbalance(output);
    let unbalanced_delimiters = out_bal.0 > src_bal.0
        || out_bal.1 > src_bal.1
        || out_bal.2 > src_bal.2
        || out_bal.3 > src_bal.3;

    let doubled_words = has_doubled_consecutive_words(output);
    let src_4gram_count = max_4gram_frequency(&chunk.text);
    let raw_out_4gram = max_4gram_frequency(output);
    let max_4gram_count = if raw_out_4gram >= 4 && raw_out_4gram > src_4gram_count {
        raw_out_4gram
    } else {
        0
    };
    let dangling_end = !empty_flag && has_dangling_end(&chunk.text, output, out_words);

    let lex_dist = if empty_flag {
        0.0
    } else {
        lexical_distance(&chunk.text, output)
    };
    let verbatim_echo_flag = if src_words >= 8 {
        lex_dist < 0.08
    } else if src_words >= 2 {
        lex_dist == 0.0
    } else {
        false
    };

    let leaked_placeholder = output.contains("__LH_PROT_") || output.contains("[[CITE_");

    ChunkValidation {
        chunk_index: chunk.chunk_index,
        empty_flag,
        truncation_flag,
        expansion_flag,
        paragraph_mismatch,
        dangling_end,
        unbalanced_delimiters,
        doubled_words,
        max_4gram_count,
        verbatim_echo_flag,
        lex_dist,
        retention_ratio,
        missing_invariants,
        leaked_placeholder,
        length_advisory,
        src_words,
        out_words,
        src_paragraphs,
        out_paragraphs,
    }
}

pub fn decide_retry(val: &ChunkValidation, think_leaked_trunc: bool) -> RetryAction {
    let has_echo = val.verbatim_echo_flag;
    let has_other = val.empty_flag
        || !val.missing_invariants.is_empty()
        || val.truncation_flag
        || val.expansion_flag
        || val.dangling_end
        || val.max_4gram_count >= 4
        || val.paragraph_mismatch
        || val.leaked_placeholder
        || think_leaked_trunc;

    if !has_echo && !has_other {
        return RetryAction::Accept;
    }

    let length_hint = if val.truncation_flag {
        Some(format!(
            "Do NOT summarize or truncate; produce roughly {} words matching all details.",
            val.src_words
        ))
    } else if val.expansion_flag {
        Some(format!(
            "Keep output concise near {} words without padding.",
            val.src_words
        ))
    } else if val.paragraph_mismatch {
        Some(format!(
            "Preserve the exact {} paragraph(s) separated by blank lines.",
            val.src_paragraphs
        ))
    } else {
        None
    };

    match (has_echo, has_other) {
        (true, false) => RetryAction::EchoRetry,
        (false, true) => RetryAction::InvariantOrTruncRetry {
            missing: val.missing_invariants.clone(),
            length_hint,
        },
        (true, true) => RetryAction::CombinedRetry {
            missing: val.missing_invariants.clone(),
            length_hint,
        },
        (false, false) => RetryAction::Accept,
    }
}

/// Higher tuple = better attempt. Prioritizes non-empty & invariant retention strictly over echo/style.
pub fn attempt_score(val: &ChunkValidation) -> (u8, u32, u8, u8, u8, u8, u32) {
    let non_empty = u8::from(!val.empty_flag);
    let retention_bps = (val.retention_ratio * 10_000.0).round() as u32;
    let no_trunc_or_loop =
        u8::from(!val.truncation_flag && !val.dangling_end && val.max_4gram_count < 4);
    let not_echo = u8::from(!val.verbatim_echo_flag);
    let para_ok = u8::from(!val.paragraph_mismatch);
    let len_ok = u8::from(!val.expansion_flag);
    let lex_bps = (val.lex_dist.min(0.60) * 10_000.0).round() as u32;
    (
        non_empty,
        retention_bps,
        no_trunc_or_loop,
        not_echo,
        para_ok,
        len_ok,
        lex_bps,
    )
}

pub fn collect_chunk_warnings(val: &ChunkValidation) -> Vec<String> {
    let mut warnings = Vec::new();
    let display_idx = val.chunk_index + 1;
    if val.empty_flag {
        warnings.push(format!(
            "Chunk {display_idx} produced empty output; please retry this chunk."
        ));
    }
    if val.verbatim_echo_flag {
        warnings.push(format!(
            "Chunk {display_idx} closely echoed the source text (lexical distance {:.2} < 0.08); you may retry this chunk for stronger rephrasing.",
            val.lex_dist
        ));
    }
    if !val.missing_invariants.is_empty() {
        warnings.push(format!(
            "Chunk {display_idx} altered or omitted protected item(s): {}",
            val.missing_invariants.join(", ")
        ));
    }
    if val.truncation_flag || val.dangling_end {
        warnings.push(format!(
            "Chunk {display_idx} appears shorter or cut off ({} -> {} words); consider retrying.",
            val.src_words, val.out_words
        ));
    } else if val.expansion_flag {
        warnings.push(format!(
            "Chunk {display_idx} expanded significantly ({} -> {} words).",
            val.src_words, val.out_words
        ));
    }
    if val.paragraph_mismatch {
        warnings.push(format!(
            "Chunk {display_idx} changed paragraph count ({} -> {}).",
            val.src_paragraphs, val.out_paragraphs
        ));
    }
    if val.max_4gram_count >= 4 {
        warnings.push(format!(
            "Chunk {display_idx} showed repetitive 4-gram phrasing (count {}).",
            val.max_4gram_count
        ));
    }
    if val.unbalanced_delimiters {
        warnings.push(format!(
            "Chunk {display_idx} introduced unbalanced brackets or quotation marks."
        ));
    }
    if val.doubled_words {
        warnings.push(format!(
            "Chunk {display_idx} contains consecutive duplicate words."
        ));
    }
    if val.leaked_placeholder {
        warnings.push(format!(
            "Chunk {display_idx} contained an internal placeholder token."
        ));
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::VoiceOption;
    use crate::engine::chunker::ChunkJoin;
    use crate::engine::protect::extract_invariants;

    fn opts() -> RewriteOptions {
        RewriteOptions {
            preserve_length: true,
            preserve_formatting: true,
            preserve_citations: true,
            preserve_quotes: true,
            language: "en".to_string(),
            voice: VoiceOption::Auto,
        }
    }

    fn make_chunk(text: &str) -> ChunkSpec {
        let o = opts();
        ChunkSpec {
            chunk_index: 0,
            text: text.to_string(),
            word_count: count_words(text),
            paragraph_count: count_paragraphs(text),
            join_with_prev: ChunkJoin::First,
            invariants: extract_invariants(text, &o),
        }
    }

    #[test]
    fn sanitizer_strips_all_think_variants_preambles_and_delimiters_while_keeping_benign_prose() {
        let raw = concat!(
            "<THINK>outer<think>nested</think>more</THINK>",
            "&lt;think&gt;entity&lt;/think&gt;",
            "&#60;think&#62;dec&#60;/think&#62;",
            "&#x3c;think&#x3e;hex&#x3c;/think&#x3e;",
            "\\u003cthink\\u003euni\\u003c/think\\u003e",
            "Here is the rewritten text:\n\n",
            "<SOURCE_TEXT_DATA>\nCritical thinking remains essential.\n</SOURCE_TEXT_DATA>",
            "<think>unclosed tail"
        );
        let res = sanitize_model_output(raw, None, "SOURCE_TEXT_DATA");
        assert!(res.think_leaked);
        assert!(res.preamble_stripped);
        assert_eq!(res.text, "Critical thinking remains essential.");
    }

    #[test]
    fn validator_routes_echo_vs_invariant_vs_combined_retries_separately() {
        let src = "On 2026-11-21, Dr. Vance reported $1,420 savings across 12 regional facilities using https://example.org/spec.";
        let ch = make_chunk(src);
        let o = opts();

        // Pure echo
        let v_echo = validate_chunk(&ch, src, &o);
        assert!(v_echo.verbatim_echo_flag);
        assert!(v_echo.missing_invariants.is_empty());
        assert_eq!(decide_retry(&v_echo, false), RetryAction::EchoRetry);
        assert!(!collect_chunk_warnings(&v_echo).is_empty());

        // Missing invariant with genuine paraphrase
        let bad_inv = "During November 2026, Dr. Vance documented cost reductions across a dozen regional facilities using the published specification.";
        let v_inv = validate_chunk(&ch, bad_inv, &o);
        assert!(!v_inv.verbatim_echo_flag);
        assert!(!v_inv.missing_invariants.is_empty());
        assert!(matches!(
            decide_retry(&v_inv, false),
            RetryAction::InvariantOrTruncRetry { .. }
        ));

        // Scorer prefers invariant-preserving paraphrase over higher-lex_dist invariant-dropping draft
        let good = "On 2026-11-21, Dr. Vance documented savings of $1,420 throughout 12 regional facilities via https://example.org/spec.";
        let v_good = validate_chunk(&ch, good, &o);
        assert!(attempt_score(&v_good) > attempt_score(&v_inv));
        assert!(attempt_score(&v_good) > attempt_score(&v_echo));
    }

    #[test]
    fn validator_detects_grammar_loops_and_dangling_cutoff() {
        let src = "The distributed pipeline processes each incoming document reliably and safely.";
        let ch = make_chunk(src);
        let o = opts();

        let dangling = "This distributed workflow handles every arriving file reliably and";
        let v_dang = validate_chunk(&ch, dangling, &o);
        assert!(v_dang.dangling_end);

        let doubled = "The the distributed workflow handles every arriving file reliably.";
        let v_dbl = validate_chunk(&ch, doubled, &o);
        assert!(v_dbl.doubled_words);

        let loopy = "The system runs well ".repeat(6);
        let v_loop = validate_chunk(&ch, &loopy, &o);
        assert!(v_loop.max_4gram_count >= 4);
    }

    #[test]
    fn sanitizer_handles_closing_think_tag_with_whitespace() {
        let res1 = sanitize_model_output("<think>secret</think >Visible text.", None, "SOURCE_TEXT_DATA");
        assert!(res1.think_leaked);
        assert_eq!(res1.text, "Visible text.");

        let res2 = sanitize_model_output("<think>a</think\t\n>Hello", None, "SOURCE_TEXT_DATA");
        assert!(res2.think_leaked);
        assert_eq!(res2.text, "Hello");

        let res3 = sanitize_model_output("</think >Orphan tail.", None, "SOURCE_TEXT_DATA");
        assert!(res3.think_leaked);
        assert_eq!(res3.text, "Orphan tail.");
    }

    #[test]
    fn validator_handles_punctuation_only_segment_without_spurious_empty_warning() {
        let ch = make_chunk("---");
        let o = opts();
        let val = validate_chunk(&ch, "---", &o);
        assert!(!val.empty_flag);
        assert!(!val.truncation_flag);
        assert_eq!(decide_retry(&val, false), RetryAction::Accept);
        assert!(collect_chunk_warnings(&val).is_empty());
    }

    #[test]
    fn sanitizer_strips_unclosed_eof_think_tag_while_keeping_thinking_tag() {
        let eof_think = sanitize_model_output("Visible<think", None, "SOURCE_TEXT_DATA");
        assert!(eof_think.think_leaked);
        assert_eq!(eof_think.text, "Visible");

        let thinking_kept = sanitize_model_output("Visible<thinking>kept", None, "SOURCE_TEXT_DATA");
        assert_eq!(thinking_kept.text, "Visible<thinking>kept");
    }

    #[test]
    fn validator_passes_verbatim_r3_regression_and_adversarial_inputs() {
        let cases = [
            "Net loss was -$10, -€50, -US$10, -USD 100, and -10 USD across $10-$20, $10M-$20M, USD 100-200, and 100 USD-200 USD tiers.",
            "Upgrade package tauri-2.12.0, ec2-2026-11-21, backup-2026-11-21, cost-$10, and foo-1.5e-4 now.",
            "Call 555-123-4567, reference 1-2-3, or ISBN 978-3-16-148410-0 and 0-306-40615-2 for details.",
            "Release 2024.10.1.2, OID 1.3.6.1.4, range 192.168.1.1-192.168.1.10, and -1.5e-4 shipped cleanly.",
            "Change was -15%-5% and 10%-20%-30% at 2024-01-15T10:30:00.123Z in window 2024-11-21-2024-11-22 and code 2024-11-21-01.",
            "Compare `foo`-5 and (2024)-5 alongside (de la Cruz, 2024, p. 12) and \"-$10\" and \"-15\".",
            "Submit résumé-15 by Friday.",
            "Section 字-15 applies.",
            "Tiers $10-$20-$30 apply.",
            "Range 1.5e-4-2.5e-4 holds.",
            "Test p < 0.05-1 holds.",
            "Formula $x$-5 applies.",
            "Contact a@b.com-5 now.",
            "Price 10 USD-5 applies.",
            "Upgrade über-2.12.0 and naïve-2024-11-21 cleanly.",
            "Budget 100-200 USD applies.",
            "Budget 1,000-2,000 EUR applies.",
            "Upgrade re\u{0301}sume\u{0301}-2.12.0 today.",
            "Backup re\u{0301}sume\u{0301}-2024-11-21 today.",
            "Range 1.5e-4-2.5 holds.",
            "Formula $x$-2026-11-21 applies.",
            "2024-11-21 USD",
            "Date 2024-11-21 USD 100 applies.",
        ];
        let o = opts();
        for src in cases {
            let val = validate_chunk(&make_chunk(src), src, &o);
            assert!(
                val.missing_invariants.is_empty(),
                "unexpected missing invariants for {src:?}: {:?}",
                val.missing_invariants
            );
            assert_eq!(val.retention_ratio, 1.0);
        }
    }
}
