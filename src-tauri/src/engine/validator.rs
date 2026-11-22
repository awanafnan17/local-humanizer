use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use regex::Regex;

use crate::commands::{Intensity, RewriteMode, RewriteOptions};
use crate::engine::chunker::{split_sentences_safe, ChunkSpec};
use crate::engine::prompts::RetryDirective;
use crate::engine::protect::{contains_with_boundary, extract_invariants};
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
    pub ngram4_overlap_ratio: f64,
    pub sequence_similarity: f64,
    pub opening_copy_ratio: f64,
    pub polarity_mismatch_flag: bool,
    pub polarity_reason: Option<String>,
    pub hallucination_flag: bool,
    pub hallucinated_items: Vec<String>,
    pub lex_dist: f64,
    pub retention_ratio: f64,
    pub missing_invariants: Vec<String>,
    pub leaked_placeholder: bool,
    pub duplicated_invariants: Vec<String>,
    pub malformed_syntax_flag: bool,
    pub placeholder_error_reason: Option<String>,
    pub length_advisory: bool,
    pub src_words: usize,
    pub out_words: usize,
    pub src_paragraphs: usize,
    pub out_paragraphs: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RetryAction {
    Accept,
    EchoRetry {
        hint: Option<String>,
    },
    InvariantOrTruncRetry {
        missing: Vec<String>,
        polarity_hint: Option<String>,
        hallucination_hint: Option<String>,
        length_hint: Option<String>,
    },
    CombinedRetry {
        missing: Vec<String>,
        polarity_hint: Option<String>,
        hallucination_hint: Option<String>,
        length_hint: Option<String>,
        structural_pivot: bool,
    },
}

impl RetryAction {
    pub fn to_directive(&self) -> RetryDirective {
        match self {
            RetryAction::Accept => RetryDirective::None,
            RetryAction::EchoRetry { hint } => RetryDirective::EchoRetry { hint: hint.clone() },
            RetryAction::InvariantOrTruncRetry {
                missing,
                polarity_hint,
                hallucination_hint,
                length_hint,
            } => RetryDirective::StrictInvariant {
                missing_items: missing.clone(),
                polarity_hint: polarity_hint.clone(),
                hallucination_hint: hallucination_hint.clone(),
                length_hint: length_hint.clone(),
            },
            RetryAction::CombinedRetry {
                missing,
                polarity_hint,
                hallucination_hint,
                length_hint,
                structural_pivot,
            } => RetryDirective::Combined {
                missing_items: missing.clone(),
                polarity_hint: polarity_hint.clone(),
                hallucination_hint: hallucination_hint.clone(),
                length_hint: length_hint.clone(),
                structural_pivot: *structural_pivot,
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

fn strip_protected_for_similarity(
    text: &str,
    chunk: &ChunkSpec,
    opts: &RewriteOptions,
) -> String {
    let mut masked = text.to_string();
    let inv = &chunk.invariants;
    let mut spans: Vec<&str> = Vec::new();
    for s in &inv.code_blocks {
        spans.push(s.as_str());
    }
    for s in &inv.latex_spans {
        spans.push(s.as_str());
    }
    for s in &inv.urls {
        spans.push(s.as_str());
    }
    for s in &inv.emails {
        spans.push(s.as_str());
    }
    for s in &inv.dois {
        spans.push(s.as_str());
    }
    if opts.preserve_quotes {
        for s in &inv.quoted_strings {
            spans.push(s.as_str());
        }
    }
    if opts.preserve_citations {
        for s in &inv.citations {
            spans.push(s.as_str());
        }
    }
    for s in &inv.currencies {
        spans.push(s.as_str());
    }
    for s in &inv.dates {
        spans.push(s.as_str());
    }
    // Sort longest first so longer spans mask cleanly before shorter substrings
    spans.sort_by_key(|s| std::cmp::Reverse(s.len()));
    for span in spans {
        if !span.is_empty() && masked.contains(span) {
            masked = masked.replace(span, " ");
        }
    }
    if opts.preserve_quotes {
        for q in &inv.quoted_strings {
            let q_norm = normalize_quote_chars(q);
            if q_norm.len() >= 4 && q_norm.starts_with('"') && q_norm.ends_with('"') {
                let inner = &q_norm[1..q_norm.len() - 1];
                for variant in [
                    format!("\"{inner}\""),
                    format!("\"{inner}.\""),
                    format!("\"{inner},\""),
                    format!("\u{201c}{inner}\u{201d}"),
                    format!("\u{201c}{inner}.\u{201d}"),
                    format!("\u{201c}{inner},\u{201d}"),
                ] {
                    if masked.contains(&variant) {
                        masked = masked.replace(&variant, " ");
                    }
                }
            }
        }
    }
    masked
}

fn tokenize_prose_sim_tokens(masked_text: &str) -> Vec<String> {
    tokenize_alnum_lower(masked_text)
        .into_iter()
        .filter(|t| !t.chars().all(|c| c.is_ascii_digit()) && t != "doi")
        .collect()
}

fn token_lexical_distance(src_tokens: &[String], out_tokens: &[String]) -> f64 {
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

pub fn source_output_4gram_overlap(src_tokens: &[String], out_tokens: &[String]) -> f64 {
    if src_tokens.len() < 4 || out_tokens.len() < 4 {
        return 0.0;
    }
    let src_4grams: HashSet<(&str, &str, &str, &str)> = src_tokens
        .windows(4)
        .map(|w| (w[0].as_str(), w[1].as_str(), w[2].as_str(), w[3].as_str()))
        .collect();
    let out_total = out_tokens.len() - 3;
    let matches = out_tokens
        .windows(4)
        .filter(|w| src_4grams.contains(&(w[0].as_str(), w[1].as_str(), w[2].as_str(), w[3].as_str())))
        .count();
    matches as f64 / out_total as f64
}

pub fn token_sequence_similarity(src_tokens: &[String], out_tokens: &[String]) -> f64 {
    if src_tokens.is_empty() && out_tokens.is_empty() {
        return 1.0;
    }
    if src_tokens.is_empty() || out_tokens.is_empty() {
        return 0.0;
    }
    // Cap DP length on pathological oversized inputs so runtime stays O(1) ms
    let a = &src_tokens[..src_tokens.len().min(300)];
    let b = &out_tokens[..out_tokens.len().min(300)];
    let mut prev = vec![0usize; b.len() + 1];
    let mut curr = vec![0usize; b.len() + 1];
    for ta in a {
        for (j, tb) in b.iter().enumerate() {
            curr[j + 1] = if ta == tb {
                prev[j] + 1
            } else {
                prev[j + 1].max(curr[j])
            };
        }
        std::mem::swap(&mut prev, &mut curr);
        curr.fill(0);
    }
    let lcs = prev[b.len()];
    (2.0 * lcs as f64) / ((a.len() + b.len()) as f64)
}

fn sentence_opening_copy_ratio(
    masked_src: &str,
    masked_out: &str,
) -> f64 {
    let src_sents = split_sentences_safe(masked_src);
    let out_sents = split_sentences_safe(masked_out);
    if src_sents.is_empty() || out_sents.is_empty() {
        return 0.0;
    }
    let src_openings: Vec<Vec<String>> = src_sents
        .iter()
        .map(|s| {
            tokenize_alnum_lower(s)
                .into_iter()
                .take(2)
                .collect::<Vec<_>>()
        })
        .filter(|v| v.len() >= 2)
        .collect();
    if src_openings.is_empty() {
        return 0.0;
    }
    let out_openings: Vec<Vec<String>> = out_sents
        .iter()
        .map(|s| {
            tokenize_alnum_lower(s)
                .into_iter()
                .take(2)
                .collect::<Vec<_>>()
        })
        .filter(|v| v.len() >= 2)
        .collect();
    if out_openings.is_empty() {
        return 0.0;
    }
    let src_set: HashSet<&[String]> = src_openings.iter().map(Vec::as_slice).collect();
    let copied = out_openings
        .iter()
        .enumerate()
        .filter(|(i, open)| {
            src_openings.get(*i).is_some_and(|so| so == *open)
                || src_set.contains(open.as_slice())
        })
        .count();
    copied as f64 / out_openings.len() as f64
}

fn strip_quotes_if_needed(text: &str, opts: &RewriteOptions) -> String {
    let mut out = text.to_string();
    if opts.preserve_quotes {
        let inv = extract_invariants(text, opts);
        for q in inv.quoted_strings {
            out = out.replace(&q, " ");
        }
    }
    out
}

fn tokenize_polarity_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    for raw in text.split_whitespace() {
        let clean = raw
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’')
            .replace('’', "'")
            .to_ascii_lowercase();
        if !clean.is_empty() {
            words.push(clean);
        }
    }
    words
}

fn is_strong_negation_word(w: &str) -> bool {
    matches!(
        w,
        "without"
            | "never"
            | "cannot"
            | "can't"
            | "won't"
            | "don't"
            | "doesn't"
            | "didn't"
            | "isn't"
            | "aren't"
            | "wasn't"
            | "weren't"
            | "couldn't"
            | "wouldn't"
            | "shouldn't"
            | "hasn't"
            | "haven't"
            | "hadn't"
            | "mustn't"
            | "needn't"
            | "shan't"
            | "mightn't"
            | "neither"
            | "nor"
            | "none"
            | "nothing"
            | "nowhere"
            | "nobody"
            | "not"
            | "no"
    )
}

fn is_privative_negation_replacement(w: &str) -> bool {
    is_strong_negation_word(w)
        || matches!(
            w,
            "lack"
                | "lacks"
                | "lacking"
                | "lacked"
                | "absent"
                | "absence"
                | "unable"
                | "impossible"
                | "improbable"
                | "unlikely"
                | "fail"
                | "fails"
                | "failed"
                | "failing"
                | "failure"
                | "failures"
                | "hardly"
                | "scarcely"
                | "barely"
                | "seldom"
                | "rarely"
                | "zero"
                | "devoid"
                | "free"
                | "exempt"
                | "exclude"
                | "excludes"
                | "excluded"
                | "excluding"
                | "except"
                | "unless"
                | "insufficient"
                | "inadequate"
                | "incomplete"
                | "incorrect"
                | "invalid"
                | "inability"
                | "incapable"
                | "ineligible"
                | "unavailability"
                | "unavailable"
                | "unnecessary"
                | "unacceptable"
                | "nonexistent"
                | "missing"
                | "omitted"
                | "omit"
                | "omits"
                | "forgo"
                | "forgoes"
        )
}

fn is_broad_negation_or_privative(w: &str) -> bool {
    is_privative_negation_replacement(w)
        || matches!(
            w,
            "prevent"
                | "prevents"
                | "prevented"
                | "preventing"
                | "prohibit"
                | "prohibits"
                | "prohibited"
                | "forbid"
                | "forbids"
                | "forbidden"
                | "refuse"
                | "refuses"
                | "refused"
                | "deny"
                | "denies"
                | "denied"
                | "reject"
                | "rejects"
                | "rejected"
                | "decline"
                | "declines"
                | "declined"
                | "avoid"
                | "avoids"
                | "avoided"
                | "avoiding"
                | "cease"
                | "ceased"
                | "halt"
                | "halted"
                | "stop"
                | "stopped"
        )
}

fn has_negative_affix_word(w: &str) -> bool {
    if w.starts_with("non-") && w.len() >= 6 {
        return true;
    }
    if matches!(
        w,
        "unable"
            | "inability"
            | "incapable"
            | "impossible"
            | "impossibility"
            | "improbable"
            | "unlikely"
            | "unnecessary"
            | "unavailable"
            | "unavailability"
            | "unacceptable"
            | "unaware"
            | "uncertain"
            | "unclear"
            | "uncommon"
            | "unequal"
            | "unfair"
            | "unfit"
            | "unfortunate"
            | "unfounded"
            | "unhelpful"
            | "unknown"
            | "unlawful"
            | "unlimited"
            | "unlisted"
            | "unnoticed"
            | "unpaid"
            | "unplanned"
            | "unpredictable"
            | "unprepared"
            | "unproductive"
            | "unproven"
            | "unqualified"
            | "unreadable"
            | "unreal"
            | "unreasonable"
            | "unreliable"
            | "unresolved"
            | "unsafe"
            | "unsatisfactory"
            | "unseen"
            | "unstable"
            | "unsuccessful"
            | "unsuitable"
            | "unsure"
            | "untimely"
            | "untrue"
            | "unused"
            | "unusual"
            | "unverified"
            | "unwanted"
            | "unwarranted"
            | "unwilling"
            | "unworkable"
            | "insufficient"
            | "insufficiency"
            | "inadequate"
            | "inadequacy"
            | "incomplete"
            | "inconsistent"
            | "incorrect"
            | "inaccurate"
            | "inaccessible"
            | "inactive"
            | "inappropriate"
            | "incompatible"
            | "incompetent"
            | "ineffective"
            | "inefficient"
            | "ineligible"
            | "inexact"
            | "inexperienced"
            | "infrequent"
            | "insecure"
            | "insensitive"
            | "insignificant"
            | "insoluble"
            | "insufficiently"
            | "invalid"
            | "invisible"
            | "involuntary"
            | "imbalance"
            | "imbalanced"
            | "immature"
            | "immobile"
            | "imperfect"
            | "impermissible"
            | "impractical"
            | "imprecise"
            | "improper"
            | "impure"
            | "nonexistent"
            | "noncompliance"
            | "noncompliant"
            | "nonfunctional"
            | "nondisclosure"
            | "nonprofit"
            | "nonsense"
            | "nonstop"
            | "nonstandard"
            | "disable"
            | "disabled"
            | "disables"
            | "disabling"
            | "disadvantage"
            | "disagree"
            | "disagreed"
            | "disagrees"
            | "disappear"
            | "disappeared"
            | "disappears"
            | "disapprove"
            | "disbelief"
            | "discard"
            | "discarded"
            | "disconnect"
            | "disconnected"
            | "discontinue"
            | "discontinued"
            | "discontinues"
            | "discourage"
            | "discouraged"
            | "discredit"
            | "discrepancy"
            | "dishonest"
            | "dislike"
            | "disloyal"
            | "dismiss"
            | "dismissed"
            | "disobey"
            | "disorder"
            | "disorganized"
            | "disprove"
            | "disproved"
            | "disqualify"
            | "disqualified"
            | "disregard"
            | "disregarded"
            | "disrupt"
            | "disrupted"
            | "disrespect"
            | "dissatisfied"
            | "dissent"
            | "dissimilar"
            | "distrust"
            | "disuse"
    ) {
        return true;
    }
    w.len() >= 6
        && w.ends_with("less")
        && !matches!(
            w,
            "unless"
                | "priceless"
                | "regardless"
                | "countless"
                | "timeless"
                | "effortless"
                | "flawless"
                | "seamless"
                | "wireless"
                | "endless"
                | "boundless"
                | "matchless"
                | "peerless"
                | "dauntless"
                | "fearless"
                | "tireless"
                | "selfless"
        )
}

fn is_source_necessity_word(w: &str) -> bool {
    matches!(
        w,
        "necessary"
            | "unnecessary"
            | "need"
            | "needed"
            | "needs"
            | "needless"
            | "require"
            | "requires"
            | "required"
            | "requiring"
            | "requirement"
            | "requirements"
            | "essential"
            | "must"
            | "mandatory"
            | "vital"
            | "indispensable"
            | "necessitate"
            | "necessitates"
            | "necessitated"
            | "demand"
            | "demands"
            | "optional"
    )
}

fn has_negated_necessity_phrase(words: &[String]) -> bool {
    for (idx, w) in words.iter().enumerate() {
        if w == "necessary" {
            let start = idx.saturating_sub(6);
            if words[start..idx].iter().any(|prev| {
                matches!(
                    prev.as_str(),
                    "no" | "not" | "never" | "none" | "neither" | "nor" | "nothing" | "hardly" | "scarcely" | "barely"
                )
            }) {
                return true;
            }
        }
    }
    false
}

fn is_without_loss_gerund(w: &str) -> bool {
    matches!(
        w,
        "sacrificing"
            | "compromising"
            | "losing"
            | "harming"
            | "hurting"
            | "damaging"
            | "degrading"
            | "undermining"
            | "eroding"
            | "impairing"
            | "weakening"
            | "diminishing"
    )
}

fn is_preservation_verb(w: &str) -> bool {
    matches!(
        w,
        "preserve"
            | "preserves"
            | "preserved"
            | "preserving"
            | "maintain"
            | "maintains"
            | "maintained"
            | "maintaining"
            | "keep"
            | "keeps"
            | "kept"
            | "keeping"
            | "retain"
            | "retains"
            | "retained"
            | "retaining"
            | "protect"
            | "protects"
            | "protected"
            | "protecting"
            | "uphold"
            | "upholds"
            | "upholding"
            | "safeguard"
            | "safeguards"
            | "safeguarding"
    )
}

pub fn detect_polarity_mismatch(
    source: &str,
    output: &str,
    opts: &RewriteOptions,
) -> Option<String> {
    if output.trim().is_empty() || source.trim().is_empty() {
        return None;
    }
    let src_clean = strip_quotes_if_needed(source, opts);
    let out_clean = strip_quotes_if_needed(output, opts);
    let src_words = tokenize_polarity_words(&src_clean);
    let out_words = tokenize_polarity_words(&out_clean);

    let out_has_preservation = out_words.iter().any(|w| is_preservation_verb(w));
    let out_has_contrastive = out_words
        .windows(2)
        .any(|pair| matches!((pair[0].as_str(), pair[1].as_str()), ("instead", "of") | ("rather", "than")));
    let mut src_strong = 0usize;
    let mut src_broad = 0usize;
    for (idx, w) in src_words.iter().enumerate() {
        let is_correlative_not_only = w == "not"
            && src_words.get(idx + 1).map(String::as_str) == Some("only")
            && src_words[idx + 2..].iter().any(|t| t == "but");
        if is_correlative_not_only {
            continue;
        }
        if is_broad_negation_or_privative(w) {
            src_broad += 1;
        }
        if is_strong_negation_word(w) {
            if w == "without" {
                let next_w = src_words.get(idx + 1).map(String::as_str);
                if out_has_preservation && next_w.is_some_and(is_without_loss_gerund) {
                    continue;
                }
                if out_has_contrastive && next_w.is_some_and(|nw| nw.ends_with("ing") && nw.len() >= 5) {
                    continue;
                }
            }
            src_strong += 1;
        }
    }
    let out_privative = out_words
        .iter()
        .filter(|w| is_privative_negation_replacement(w))
        .count();
    let out_strong = out_words.iter().filter(|w| is_strong_negation_word(w)).count();

    // 1. Specific Qwen3-1.7B failure: "without X" -> "with no X necessary" / "unnecessary"
    // Scoped so legitimate paraphrases of "requires"/"essential"/"must" -> "it is necessary"
    // in the same chunk do NOT false-positive when "without" remains intact.
    let src_without_count = src_words.iter().filter(|w| *w == "without").count();
    if src_without_count > 0 {
        let out_without_count = out_words.iter().filter(|w| *w == "without").count();
        let src_has_neg_necessity = src_words
            .iter()
            .any(|w| matches!(w.as_str(), "unnecessary" | "needless" | "optional"))
            || has_negated_necessity_phrase(&src_words);
        let out_introduces_neg_necessity_word = out_words
            .iter()
            .any(|w| matches!(w.as_str(), "unnecessary" | "needless" | "optional"));
        let out_sents = split_sentences_safe(&out_clean);
        let out_has_local_neg_necessity = out_sents.iter().any(|s| {
            let sw = tokenize_polarity_words(s);
            has_negated_necessity_phrase(&sw)
        });

        if !src_has_neg_necessity && (out_introduces_neg_necessity_word || out_has_local_neg_necessity)
        {
            return Some(
                "Polarity distortion: source 'without ...' was altered into necessity phrasing ('necessary'/'unnecessary').".to_string(),
            );
        }

        if out_without_count < src_without_count && out_words.iter().any(|w| w == "necessary") {
            let src_sents = split_sentences_safe(&src_clean);
            let src_necessity_sents = src_sents
                .iter()
                .filter(|s| tokenize_polarity_words(s).iter().any(|w| is_source_necessity_word(w)))
                .count();
            let out_necessary_sents = out_sents
                .iter()
                .filter(|s| {
                    tokenize_polarity_words(s)
                        .iter()
                        .any(|w| matches!(w.as_str(), "necessary" | "unnecessary" | "needless" | "optional"))
                })
                .count();
            if src_necessity_sents == 0 || out_necessary_sents > src_necessity_sents {
                return Some(
                    "Polarity distortion: source 'without ...' was altered into necessity phrasing ('necessary'/'unnecessary').".to_string(),
                );
            }
        }
    }

    // 2. Dropped critical negation: source had strong negation, output has zero negation/privative markers
    if src_strong > 0 && out_privative == 0 && !out_words.iter().any(|w| has_negative_affix_word(w)) {
        let missing_neg: Vec<&str> = src_words
            .iter()
            .filter(|w| is_strong_negation_word(w))
            .map(String::as_str)
            .collect();
        return Some(format!(
            "Dropped critical negation marker present in source (keep: {}).",
            missing_neg.join(", ")
        ));
    }

    // 3. Invented negation: source had zero negations/negative affixes, output added a strong negation
    let src_any_neg_hint = src_broad > 0 || src_words.iter().any(|w| has_negative_affix_word(w));
    if !src_any_neg_hint && out_strong > 0 {
        return Some("Introduced a new negation absent from the source text.".to_string());
    }

    // 4. Dropped conditional restriction ("unless")
    if src_words.iter().any(|w| w == "unless")
        && !out_words.iter().any(|w| {
            matches!(
                w.as_str(),
                "unless" | "except" | "only" | "provided" | "without" | "if" | "until"
            )
        })
    {
        return Some("Dropped conditional restriction ('unless') present in source.".to_string());
    }

    // 5. Dropped restrictive "only" qualifier (excluding additive "not only")
    let src_has_restrictive_only = src_words
        .iter()
        .enumerate()
        .any(|(idx, w)| w == "only" && (idx == 0 || src_words[idx - 1] != "not"));
    if src_has_restrictive_only && !has_restrictive_only_or_equiv(&out_words) {
        return Some(
            "Dropped restrictive qualifier ('only') present in source (keep 'only', 'solely', 'exclusively', 'at most', or 'no more than').".to_string(),
        );
    }

    None
}

fn has_restrictive_only_or_equiv(out_words: &[String]) -> bool {
    let has_single = out_words.iter().enumerate().any(|(idx, w)| {
        if w == "only" {
            return idx == 0 || out_words[idx - 1] != "not";
        }
        matches!(
            w.as_str(),
            "solely" | "exclusively" | "merely" | "just" | "sole" | "alone"
        )
    });
    if has_single {
        return true;
    }
    let has_bigram = out_words.windows(2).any(|w| {
        matches!(
            (w[0].as_str(), w[1].as_str()),
            ("at", "most")
                | ("up", "to")
                | ("capped", "at")
                | ("limited", "to")
                | ("restricted", "to")
                | ("confined", "to")
                | ("at", "best")
                | ("maximum", "of")
                | ("nothing", "beyond")
                | ("cannot", "exceed")
                | ("never", "exceed")
        )
    });
    if has_bigram {
        return true;
    }
    out_words.windows(3).any(|w| {
        matches!(
            (w[0].as_str(), w[1].as_str(), w[2].as_str()),
            ("no", "more", "than")
                | ("not", "more", "than")
                | ("no", "higher", "than")
                | ("a", "maximum", "of")
                | ("at", "a", "maximum")
                | ("nothing", "more", "than")
                | ("no", "greater", "than")
                | ("does", "not", "exceed")
                | ("do", "not", "exceed")
        )
    })
}

fn identifier_subject_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"^(?:[\("'\u{201c}\u{2018}\[]\s*)*(?:\[\[(?:LH_)?(?:DOI|URL|EMAIL)_\d+\]\]|https?://\S+|10\.\d{4,9}/\S+)"#).unwrap()
    })
}

fn is_reporting_verb(word: &str) -> bool {
    let clean = word
        .trim_matches(|c: char| !c.is_ascii_alphabetic())
        .to_ascii_lowercase();
    matches!(
        clean.as_str(),
        "documents"
            | "document"
            | "documented"
            | "states"
            | "state"
            | "stated"
            | "notes"
            | "note"
            | "noted"
            | "reports"
            | "report"
            | "reported"
            | "conducted"
            | "conducts"
            | "conduct"
            | "highlights"
            | "highlight"
            | "highlighted"
            | "shows"
            | "show"
            | "showed"
            | "demonstrates"
            | "demonstrate"
            | "demonstrated"
            | "confirms"
            | "confirm"
            | "confirmed"
            | "indicates"
            | "indicate"
            | "indicated"
            | "reveals"
            | "reveal"
            | "revealed"
            | "explains"
            | "explain"
            | "explained"
            | "observes"
            | "observe"
            | "observed"
            | "argues"
            | "argue"
            | "argued"
            | "claims"
            | "claim"
            | "claimed"
            | "suggests"
            | "suggest"
            | "suggested"
            | "concludes"
            | "conclude"
            | "concluded"
            | "found"
            | "finds"
            | "find"
            | "writes"
            | "wrote"
            | "mentions"
            | "mentioned"
            | "presents"
            | "present"
            | "presented"
            | "summarizes"
            | "summarize"
            | "summarized"
            | "proves"
            | "prove"
            | "proved"
            | "establishes"
            | "establish"
            | "established"
    )
}

pub fn has_identifier_subject_verb(text: &str) -> bool {
    for sent in split_sentences_safe(text) {
        let trimmed = sent.trim_start();
        if let Some(m) = identifier_subject_re().find(trimmed) {
            let rest = trimmed[m.end()..].trim_start();
            let mut words = rest.split_whitespace();
            if let Some(w1) = words.next() {
                if is_reporting_verb(w1) {
                    return true;
                }
                let w1_clean = w1
                    .trim_matches(|c: char| !c.is_ascii_alphabetic())
                    .to_ascii_lowercase();
                if matches!(
                    w1_clean.as_str(),
                    "also" | "clearly" | "explicitly" | "further" | "specifically" | "directly"
                ) && words.next().is_some_and(is_reporting_verb)
                {
                    return true;
                }
            }
        }
    }
    false
}

fn normalize_quote_chars(s: &str) -> String {
    s.replace(['\u{201c}', '\u{201d}'], "\"")
        .replace(['\u{2018}', '\u{2019}'], "'")
}

fn count_opaque_occurrences(haystack: &str, needle: &str, case_insensitive: bool) -> usize {
    if needle.is_empty() {
        return 0;
    }
    if case_insensitive {
        haystack
            .to_ascii_lowercase()
            .matches(&needle.to_ascii_lowercase())
            .count()
    } else {
        let h_norm = normalize_quote_chars(haystack);
        let n_norm = normalize_quote_chars(needle);
        if n_norm.len() >= 4 && n_norm.starts_with('"') && n_norm.ends_with('"') {
            let inner = &n_norm[1..n_norm.len() - 1];
            let stem = inner.strip_suffix('.').unwrap_or(inner);
            h_norm.matches(&format!("\"{stem}\"")).count()
                + h_norm.matches(&format!("\"{stem}.\"")).count()
                + h_norm.matches(&format!("\"{stem},\"")).count()
        } else {
            h_norm.matches(&n_norm).count()
        }
    }
}

fn number_word_present_in_source(num_token: &str, src_lower_words: &HashSet<String>) -> bool {
    let clean = num_token.trim_end_matches('%');
    let word_equiv: Option<&[&str]> = match clean {
        "0" => Some(&["zero"]),
        "1" => Some(&["one", "single", "first"]),
        "2" => Some(&["two", "double", "twice", "second"]),
        "3" => Some(&["three", "triple", "third"]),
        "4" => Some(&["four", "fourth"]),
        "5" => Some(&["five", "fifth"]),
        "6" => Some(&["six", "sixth"]),
        "7" => Some(&["seven", "seventh"]),
        "8" => Some(&["eight", "eighth"]),
        "9" => Some(&["nine", "ninth"]),
        "10" => Some(&["ten", "tenth", "decade"]),
        "12" => Some(&["twelve", "dozen"]),
        "100" => Some(&["hundred"]),
        "1000" | "1,000" => Some(&["thousand"]),
        "1000000" | "1,000,000" => Some(&["million"]),
        _ => None,
    };
    if let Some(equivs) = word_equiv {
        return equivs.iter().any(|w| src_lower_words.contains(*w));
    }
    false
}

pub fn detect_hallucinations(
    source: &str,
    output: &str,
    opts: &RewriteOptions,
) -> Vec<String> {
    if output.trim().is_empty() {
        return Vec::new();
    }
    let src_inv = extract_invariants(source, opts);
    let out_inv = extract_invariants(output, opts);

    let mut hallucinated = Vec::new();
    let src_lower_words: HashSet<String> = tokenize_alnum_lower(source).into_iter().collect();

    for u in &out_inv.urls {
        if !contains_with_boundary(u, source, true) {
            hallucinated.push(format!("url:{u}"));
        }
    }
    for e in &out_inv.emails {
        if !contains_with_boundary(e, source, true) {
            hallucinated.push(format!("email:{e}"));
        }
    }
    for d in &out_inv.dois {
        if !contains_with_boundary(d, source, true) {
            hallucinated.push(format!("doi:{d}"));
        }
    }
    for c in &out_inv.citations {
        if !src_inv.citations.iter().any(|sc| sc.eq_ignore_ascii_case(c))
            && !contains_with_boundary(c, source, true)
        {
            // Allow APA parenthetical <-> narrative format shift when all author/year tokens exist in source citations
            let is_apa_format_shift = !c.trim_start().starts_with('[') && {
                let c_toks: Vec<String> = tokenize_alnum_lower(c)
                    .into_iter()
                    .filter(|t| {
                        !matches!(t.as_str(), "et" | "al" | "and" | "pp" | "p" | "ch" | "fig" | "sec" | "para" | "see" | "also" | "e" | "g" | "i" | "cf")
                    })
                    .collect();
                !c_toks.is_empty()
                    && c_toks
                        .iter()
                        .all(|t| src_lower_words.contains(t))
            };
            if !is_apa_format_shift {
                hallucinated.push(format!("citation:{c}"));
            }
        }
    }
    for curr in &out_inv.currencies {
        if !contains_with_boundary(curr, source, false) {
            hallucinated.push(format!("currency:{curr}"));
        }
    }
    for num in &out_inv.numbers {
        if !contains_with_boundary(num, source, false)
            && !number_word_present_in_source(num, &src_lower_words)
        {
            // Also check if this bare number appears inside a source date/citation/range string
            let bare = num.trim_end_matches('%');
            let in_src_date_or_cite = src_inv.dates.iter().any(|d| d.contains(bare))
                || src_inv.citations.iter().any(|c| c.contains(bare))
                || src_inv.currencies.iter().any(|c| c.contains(bare))
                || src_inv.numbers.iter().any(|n| n.contains(bare));
            if !in_src_date_or_cite {
                hallucinated.push(format!("number:{num}"));
            }
        }
    }

    hallucinated
}

pub fn validate_chunk(
    chunk: &ChunkSpec,
    output: &str,
    opts: &RewriteOptions,
) -> ChunkValidation {
    validate_chunk_with_mode(
        chunk,
        output,
        opts,
        RewriteMode::Standard,
        Intensity::Balanced,
    )
}

pub fn validate_chunk_with_mode(
    chunk: &ChunkSpec,
    output: &str,
    opts: &RewriteOptions,
    mode: RewriteMode,
    intensity: Intensity,
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
        if mode == RewriteMode::Simple || !opts.preserve_length {
            (ratio < 0.50, ratio > 1.75, false)
        } else {
            let trunc = ratio < 0.65;
            let exp = ratio > 1.45;
            let adv = src_words >= 20 && !(0.78..=1.28).contains(&ratio);
            (trunc, exp, adv)
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

    let masked_src = strip_protected_for_similarity(&chunk.text, chunk, opts);
    let masked_out = strip_protected_for_similarity(output, chunk, opts);
    let src_sim_tokens = tokenize_prose_sim_tokens(&masked_src);
    let out_sim_tokens = tokenize_prose_sim_tokens(&masked_out);
    let effective_lex_dist = if empty_flag {
        0.0
    } else if src_sim_tokens.len() >= 5 {
        token_lexical_distance(&src_sim_tokens, &out_sim_tokens)
    } else {
        lex_dist
    };

    let (ngram4_overlap_ratio, sequence_similarity, opening_copy_ratio) = if empty_flag {
        (1.0, 1.0, 1.0)
    } else {
        (
            source_output_4gram_overlap(&src_sim_tokens, &out_sim_tokens),
            token_sequence_similarity(&src_sim_tokens, &out_sim_tokens),
            sentence_opening_copy_ratio(&masked_src, &masked_out),
        )
    };

    let non_prot_words = src_sim_tokens.len();
    let protected_invariant_count = enforced.len();
    let ultra_dense_invariant = non_prot_words <= 24 && protected_invariant_count >= 3;
    let dense_multi_invariant_chunk = (non_prot_words <= 34
        && protected_invariant_count >= 6
        && src_words >= non_prot_words + 10)
        || (non_prot_words <= 42
            && protected_invariant_count >= 8
            && src_words >= non_prot_words + 20);
    let high_invariant_dense = (non_prot_words <= 34 && src_words >= non_prot_words + 8)
        || (src_words >= 12 && non_prot_words * 10 <= src_words * 6);
    let verbatim_echo_flag = if empty_flag {
        false
    } else if src_words >= 6 && non_prot_words >= 5 {
        let strict_deep = mode == RewriteMode::Deep && intensity != Intensity::Light;
        if ultra_dense_invariant {
            let n4_cap = if non_prot_words <= 12 { 1.0 } else { 0.62 };
            if intensity == Intensity::Light && mode != RewriteMode::Deep {
                effective_lex_dist < 0.14
                    || sequence_similarity > 0.86
                    || (non_prot_words > 12 && ngram4_overlap_ratio > 0.65)
            } else {
                effective_lex_dist < 0.20
                    || sequence_similarity > 0.84
                    || (non_prot_words > 12 && ngram4_overlap_ratio > n4_cap)
            }
        } else if dense_multi_invariant_chunk {
            if intensity == Intensity::Light && mode != RewriteMode::Deep {
                effective_lex_dist < 0.14
                    || sequence_similarity > 0.86
                    || ngram4_overlap_ratio > 0.62
            } else {
                effective_lex_dist < 0.20
                    || sequence_similarity > 0.84
                    || ngram4_overlap_ratio > 0.58
            }
        } else {
            match (intensity, strict_deep, high_invariant_dense) {
                (Intensity::Light, false, false) if mode != RewriteMode::Deep => {
                    effective_lex_dist < 0.14
                        || sequence_similarity > 0.85
                        || ngram4_overlap_ratio > 0.55
                }
                (Intensity::Light, false, true) if mode != RewriteMode::Deep => {
                    effective_lex_dist < 0.14
                        || sequence_similarity > 0.86
                        || ngram4_overlap_ratio > 0.60
                }
                (Intensity::Strong, _, false) | (_, true, false) => {
                    effective_lex_dist < 0.26
                        || sequence_similarity > 0.74
                        || ngram4_overlap_ratio > 0.30
                        || (sequence_similarity > 0.56
                            && (ngram4_overlap_ratio >= 0.20
                                || opening_copy_ratio >= 0.66
                                || effective_lex_dist < 0.32))
                }
                (Intensity::Strong, _, true) | (_, true, true) => {
                    let n4_cap = if non_prot_words <= 12 { 0.55 } else { 0.44 };
                    effective_lex_dist < 0.25
                        || sequence_similarity > 0.75
                        || ngram4_overlap_ratio > n4_cap
                        || (sequence_similarity > 0.66
                            && (ngram4_overlap_ratio >= 0.32 || effective_lex_dist < 0.30))
                }
                (_, _, false) => {
                    effective_lex_dist < 0.20
                        || sequence_similarity > 0.78
                        || ngram4_overlap_ratio > 0.40
                        || (sequence_similarity > 0.65
                            && (ngram4_overlap_ratio >= 0.25
                                || opening_copy_ratio >= 0.66
                                || effective_lex_dist < 0.26))
                }
                (_, _, true) => {
                    let n4_cap = if non_prot_words <= 12 { 0.55 } else { 0.48 };
                    effective_lex_dist < 0.22
                        || sequence_similarity > 0.77
                        || ngram4_overlap_ratio > n4_cap
                        || (sequence_similarity > 0.70
                            && (ngram4_overlap_ratio >= 0.38 || effective_lex_dist < 0.28))
                }
            }
        }
    } else if src_words >= 3 {
        lex_dist == 0.0
    } else {
        false
    };

    let polarity_reason = if empty_flag {
        None
    } else {
        detect_polarity_mismatch(&chunk.text, output, opts)
    };
    let polarity_mismatch_flag = polarity_reason.is_some();

    let hallucinated_items = if empty_flag {
        Vec::new()
    } else {
        detect_hallucinations(&chunk.text, output, opts)
    };
    let hallucination_flag = !hallucinated_items.is_empty();

    let mut duplicated_invariants = Vec::new();
    if !empty_flag {
        for u in &chunk.invariants.urls {
            if count_opaque_occurrences(output, u, true)
                > count_opaque_occurrences(&chunk.text, u, true)
            {
                duplicated_invariants.push(format!("url:{u}"));
            }
        }
        for d in &chunk.invariants.dois {
            if count_opaque_occurrences(output, d, true)
                > count_opaque_occurrences(&chunk.text, d, true)
            {
                duplicated_invariants.push(format!("doi:{d}"));
            }
        }
        for e in &chunk.invariants.emails {
            if count_opaque_occurrences(output, e, true)
                > count_opaque_occurrences(&chunk.text, e, true)
            {
                duplicated_invariants.push(format!("email:{e}"));
            }
        }
        if opts.preserve_quotes {
            for q in &chunk.invariants.quoted_strings {
                if count_opaque_occurrences(output, q, false)
                    > count_opaque_occurrences(&chunk.text, q, false)
                {
                    duplicated_invariants.push(format!("quote:{q}"));
                }
            }
        }
    }

    let malformed_syntax_flag = !empty_flag
        && !has_identifier_subject_verb(&chunk.text)
        && has_identifier_subject_verb(output);

    let leaked_placeholder = (!chunk.text.contains("__LH_PROT_") && output.contains("__LH_PROT_"))
        || (!chunk.text.contains("[[CITE_") && output.contains("[[CITE_"))
        || (!chunk.text.contains("[[QUOTED_") && output.contains("[[QUOTED_"))
        || (!chunk.text.contains("[URL_") && output.contains("[URL_"))
        || (!chunk.text.contains("[DOI_") && output.contains("[DOI_"))
        || (!chunk.text.contains("[EMAIL_") && output.contains("[EMAIL_"))
        || (!chunk.text.contains("[QUOTE_") && output.contains("[QUOTE_"))
        || (!chunk.text.contains("[LH_URL_") && output.contains("[LH_URL_"))
        || (!chunk.text.contains("[LH_DOI_") && output.contains("[LH_DOI_"))
        || (!chunk.text.contains("[LH_EMAIL_") && output.contains("[LH_EMAIL_"))
        || (!chunk.text.contains("[LH_QUOTE_") && output.contains("[LH_QUOTE_"))
        || duplicated_invariants
            .iter()
            .any(|item| item.starts_with("url:") || item.starts_with("doi:") || item.starts_with("email:"));

    let placeholder_error_reason = if !duplicated_invariants.is_empty() {
        Some(format!(
            "Duplicated protected item(s) ({}); include each URL, DOI, email, and quote only once.",
            duplicated_invariants.join(", ")
        ))
    } else if leaked_placeholder {
        Some("Internal or corrupted placeholder token appeared in output.".to_string())
    } else {
        None
    };

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
        ngram4_overlap_ratio,
        sequence_similarity,
        opening_copy_ratio,
        polarity_mismatch_flag,
        polarity_reason,
        hallucination_flag,
        hallucinated_items,
        lex_dist,
        retention_ratio,
        missing_invariants,
        leaked_placeholder,
        duplicated_invariants,
        malformed_syntax_flag,
        placeholder_error_reason,
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
        || val.polarity_mismatch_flag
        || val.hallucination_flag
        || val.truncation_flag
        || val.expansion_flag
        || val.dangling_end
        || val.max_4gram_count >= 4
        || val.paragraph_mismatch
        || val.leaked_placeholder
        || !val.duplicated_invariants.is_empty()
        || val.malformed_syntax_flag
        || think_leaked_trunc;

    if !has_echo && !has_other {
        return RetryAction::Accept;
    }

    let echo_hint = if has_echo {
        if val.opening_copy_ratio >= 0.50 {
            Some("Your sentence openings matched the source too closely; begin each sentence with a different grammatical element.".to_string())
        } else if val.ngram4_overlap_ratio > 0.25 || val.sequence_similarity > 0.52 {
            Some("You kept too much of the source clause skeleton; reorder clauses and replace non-technical wording.".to_string())
        } else {
            None
        }
    } else {
        None
    };

    let polarity_hint = val.polarity_reason.as_ref().map(|r| {
        format!("Fix polarity error ({r}) so negations and conditions match the source meaning exactly.")
    });

    let mut extra_hints = Vec::new();
    if !val.hallucinated_items.is_empty() {
        extra_hints.push(format!(
            "Remove invented/ungrounded items not in the source: {}.",
            val.hallucinated_items.join(", ")
        ));
    }
    if val.leaked_placeholder || !val.duplicated_invariants.is_empty() {
        extra_hints.push(
            "Use each [[URL_N]], [[DOI_N]], [[EMAIL_N]] placeholder and verbatim quote EXACTLY ONCE matching the source—never repeat or invent placeholder tokens."
                .to_string(),
        );
    }
    if val.malformed_syntax_flag {
        extra_hints.push(
            "Never start a sentence with a URL or DOI (e.g., [[URL_1]] or [[DOI_1]]) followed by a reporting verb like 'documents' or 'states'; place it in a passive reference phrase such as 'at [[URL_1]]' or 'in [[DOI_1]]'."
                .to_string(),
        );
    }
    let hallucination_hint = if extra_hints.is_empty() {
        None
    } else {
        Some(extra_hints.join(" "))
    };

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
        (true, false) => RetryAction::EchoRetry { hint: echo_hint },
        (false, true) => RetryAction::InvariantOrTruncRetry {
            missing: val.missing_invariants.clone(),
            polarity_hint,
            hallucination_hint,
            length_hint,
        },
        (true, true) => RetryAction::CombinedRetry {
            missing: val.missing_invariants.clone(),
            polarity_hint,
            hallucination_hint,
            length_hint,
            structural_pivot: true,
        },
        (false, false) => RetryAction::Accept,
    }
}

pub fn is_fatal_chunk_failure(val: &ChunkValidation) -> bool {
    val.empty_flag
        || val.verbatim_echo_flag
        || val.polarity_mismatch_flag
        || val.hallucination_flag
        || val.leaked_placeholder
        || !val.duplicated_invariants.is_empty()
        || val.malformed_syntax_flag
        || !val.missing_invariants.is_empty()
        || val.dangling_end
        || (val.truncation_flag && val.out_words == 0)
}

/// Higher tuple = better attempt. Prioritizes non-empty, meaning/placeholder safety
/// (`meaning_safe`), and genuine paraphrasing (`not_echo`) over invariant retention alone so
/// a meaning-broken/duplicated-placeholder draft never outranks a clean attempt, while a
/// verbatim echo never outranks a clean paraphrase.
pub fn attempt_score(val: &ChunkValidation) -> (u8, u8, u8, u8, u32, u8, u8, u32) {
    let non_empty = u8::from(!val.empty_flag);
    let meaning_safe = u8::from(
        !val.polarity_mismatch_flag
            && !val.hallucination_flag
            && !val.leaked_placeholder
            && val.duplicated_invariants.is_empty()
            && !val.malformed_syntax_flag,
    );
    let not_echo = u8::from(!val.verbatim_echo_flag);
    let no_trunc_or_loop =
        u8::from(!val.truncation_flag && !val.dangling_end && val.max_4gram_count < 4);
    let retention_bps = (val.retention_ratio * 10_000.0).round() as u32;
    let para_ok = u8::from(!val.paragraph_mismatch);
    let len_ok = u8::from(!val.expansion_flag);
    let style_div = (1.0 - val.ngram4_overlap_ratio.clamp(0.0, 1.0)) * 0.5
        + val.lex_dist.min(0.75) * 0.5;
    let style_bps = (style_div * 10_000.0).round() as u32;
    (
        non_empty,
        meaning_safe,
        not_echo,
        no_trunc_or_loop,
        retention_bps,
        para_ok,
        len_ok,
        style_bps,
    )
}

pub fn collect_chunk_warnings(val: &ChunkValidation) -> Vec<String> {
    let mut warnings = Vec::new();
    let display_idx = val.chunk_index + 1;
    if val.empty_flag {
        warnings.push(format!(
            "Chunk {display_idx} produced empty output."
        ));
    }
    if val.verbatim_echo_flag {
        warnings.push(format!(
            "Chunk {display_idx} remained too close to the source text (lexical distance {:.2}, 4-gram overlap {:.0}%, sequence similarity {:.0}%).",
            val.lex_dist,
            val.ngram4_overlap_ratio * 100.0,
            val.sequence_similarity * 100.0
        ));
    }
    if let Some(ref reason) = val.polarity_reason {
        warnings.push(format!("Chunk {display_idx} polarity check failed: {reason}"));
    }
    if val.hallucination_flag {
        warnings.push(format!(
            "Chunk {display_idx} introduced ungrounded item(s): {}",
            val.hallucinated_items.join(", ")
        ));
    }
    if !val.missing_invariants.is_empty() {
        warnings.push(format!(
            "Chunk {display_idx} altered or omitted protected item(s): {}",
            val.missing_invariants.join(", ")
        ));
    }
    if !val.duplicated_invariants.is_empty() {
        warnings.push(format!(
            "Chunk {display_idx} duplicated protected item(s): {}",
            val.duplicated_invariants.join(", ")
        ));
    }
    if val.malformed_syntax_flag {
        warnings.push(format!(
            "Chunk {display_idx} used a URL or DOI as a grammatical sentence subject."
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
        warnings.push(
            val.placeholder_error_reason
                .as_ref()
                .map(|r| format!("Chunk {display_idx} placeholder error: {r}"))
                .unwrap_or_else(|| {
                    format!("Chunk {display_idx} contained an internal placeholder token.")
                }),
        );
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
        assert!(matches!(decide_retry(&v_echo, false), RetryAction::EchoRetry { .. }));
        assert!(!collect_chunk_warnings(&v_echo).is_empty());

        // Missing invariant with genuine paraphrase (restructured opening & clauses)
        let bad_inv = "Across a dozen regional hubs, Dr. Vance documented cost reductions during November 2026 in the published specification.";
        let v_inv = validate_chunk(&ch, bad_inv, &o);
        assert!(!v_inv.verbatim_echo_flag);
        assert!(!v_inv.missing_invariants.is_empty());
        assert!(matches!(
            decide_retry(&v_inv, false),
            RetryAction::InvariantOrTruncRetry { .. }
        ));

        // Scorer strictly prefers genuine paraphrase over verbatim echo even if retention < 1.0
        assert!(attempt_score(&v_inv) > attempt_score(&v_echo));

        // Scorer prefers invariant-preserving genuine paraphrase over invariant-dropping draft
        let good = "Across 12 regional facilities, Dr. Vance documented $1,420 in savings on 2026-11-21, as detailed at https://example.org/spec.";
        let v_good = validate_chunk(&ch, good, &o);
        assert!(!v_good.verbatim_echo_flag);
        assert!(v_good.missing_invariants.is_empty());
        assert!(attempt_score(&v_good) > attempt_score(&v_inv));
        assert!(attempt_score(&v_good) > attempt_score(&v_echo));
    }

    #[test]
    fn validator_rejects_shallow_one_to_one_synonym_swap_under_balanced_and_strong() {
        let src = "Artificial intelligence has emerged as one of the most transformative technological developments in modern human history.";
        let ch = make_chunk(src);
        let o = opts();

        // Pure 1-to-1 thesaurus swap keeping identical clause skeleton
        let shallow = "Artificial intelligence has become one of the most significant technological advancements in contemporary human history.";
        let v_bal = validate_chunk_with_mode(&ch, shallow, &o, RewriteMode::Standard, Intensity::Balanced);
        assert!(
            v_bal.verbatim_echo_flag,
            "1-to-1 swap should be rejected under Balanced: seq_sim={:.3}, 4gram={:.3}, open={:.3}",
            v_bal.sequence_similarity,
            v_bal.ngram4_overlap_ratio,
            v_bal.opening_copy_ratio
        );
        assert!(v_bal.sequence_similarity > 0.62);

        // Genuine structural paraphrase passes Balanced & Strong
        let genuine = "Modern human history has witnessed artificial intelligence reshape technological progress on an unprecedented scale.";
        let v_gen = validate_chunk_with_mode(&ch, genuine, &o, RewriteMode::Standard, Intensity::Balanced);
        assert!(!v_gen.verbatim_echo_flag);
        assert!(attempt_score(&v_gen) > attempt_score(&v_bal));
    }

    #[test]
    fn validator_polarity_and_hallucination_guards_catch_distortions_without_false_positives() {
        let src = "Researchers warned that model accuracy dropped from 96.2% to 88.5% without quarterly recalibration.";
        let ch = make_chunk(src);
        let o = opts();

        // Qwen3-1.7B observed meaning flip: "without X" -> "with no X being necessary"
        let flipped = "Accuracy fell from 96.2% to 88.5%, with no quarterly recalibration being necessary, researchers cautioned.";
        let v_flip = validate_chunk_with_mode(&ch, flipped, &o, RewriteMode::Academic, Intensity::Balanced);
        assert!(v_flip.polarity_mismatch_flag);
        assert!(!matches!(decide_retry(&v_flip, false), RetryAction::Accept));

        // Dropped negation entirely
        let dropped_neg = "Researchers cautioned that quarterly recalibration caused accuracy to drop from 96.2% to 88.5%.";
        let v_drop = validate_chunk_with_mode(&ch, dropped_neg, &o, RewriteMode::Standard, Intensity::Balanced);
        assert!(v_drop.polarity_mismatch_flag);

        // Hallucinated percentage & year absent from source
        let hallucinated = "By 2027, researchers warned that accuracy dropped from 96.2% to 88.5% (a 12% loss) unless quarterly recalibration occurred.";
        let v_hall = validate_chunk_with_mode(&ch, hallucinated, &o, RewriteMode::Standard, Intensity::Balanced);
        assert!(v_hall.hallucination_flag);
        assert!(v_hall.hallucinated_items.iter().any(|x| x.contains("2027") || x.contains("12%")));

        // APA parenthetical -> narrative format shift is NOT a hallucination, while invented citation IS
        let cite_src = "According to (Vance et al., 2024), microgrids saved $1,420.";
        let cite_ch = make_chunk(cite_src);
        let narrative_shift = "Vance et al. (2024) demonstrated that microgrids saved $1,420.";
        let v_shift = validate_chunk_with_mode(&cite_ch, narrative_shift, &o, RewriteMode::Academic, Intensity::Balanced);
        assert!(!v_shift.hallucination_flag, "format shift should not flag hallucination: {:?}", v_shift.hallucinated_items);

        let fake_cite = "According to (Vance et al., 2024) and (FakeAuthor, 2029) [99], microgrids saved $1,420.";
        let v_fake = validate_chunk_with_mode(&cite_ch, fake_cite, &o, RewriteMode::Academic, Intensity::Balanced);
        assert!(v_fake.hallucination_flag);

        // Faithful polarity-preserving paraphrase passes cleanly
        let faithful = "Unless teams performed quarterly recalibration, researchers cautioned, model accuracy declined from 96.2% to 88.5%.";
        let v_ok = validate_chunk_with_mode(&ch, faithful, &o, RewriteMode::Standard, Intensity::Balanced);
        assert!(!v_ok.polarity_mismatch_flag);
        assert!(!v_ok.hallucination_flag);
        assert!(!v_ok.verbatim_echo_flag);
        assert_eq!(decide_retry(&v_ok, false), RetryAction::Accept);

        // Regression 1: "requires ... without ..." where "requires" -> "it is necessary" while "without" stays intact
        let req_without_src = "Achieving high analytical reliability under strictly offline constraints requires rethinking both model architecture and systems engineering. Even so, smaller models cannot compensate for weak orchestration: without deterministic input segmentation, lightweight pipelines risk silently distorting clinical summaries.";
        let req_without_ok = "To achieve high analytical reliability under strict offline constraints, it is necessary to reevaluate both the model architecture and the systems engineering approach. Nevertheless, smaller models are still limited by weak orchestration: without deterministic input segmentation, lightweight pipelines may inadvertently distort clinical summaries.";
        let req_ch = make_chunk(req_without_src);
        let v_req_ok = validate_chunk_with_mode(&req_ch, req_without_ok, &o, RewriteMode::Standard, Intensity::Balanced);
        assert!(
            !v_req_ok.polarity_mismatch_flag,
            "legitimate 'requires' -> 'it is necessary' with preserved 'without' must not trigger polarity mismatch: {:?}",
            v_req_ok.polarity_reason
        );

        // Multi-sentence where Sentence 1 has "requires" -> "necessary" and Sentence 2 distorts "without" -> "no ... necessary" MUST still fail
        let req_without_flipped = "To achieve high analytical reliability, it is necessary to reevaluate architecture. Smaller models cannot compensate for weak orchestration: with no deterministic input segmentation necessary, pipelines risk distorting clinical summaries.";
        let v_req_flip = validate_chunk_with_mode(&req_ch, req_without_flipped, &o, RewriteMode::Standard, Intensity::Balanced);
        assert!(v_req_flip.polarity_mismatch_flag);

        // Regression 2: benign words like "industry"/"information"/"infrastructure"/"implement"/"university" must NEVER mask a dropped negation or suppress invented negation
        let ind_src = "Automated routing cannot prevent cascading outages across the energy industry without verified infrastructure.";
        let ind_dropped = "Automated routing prevents cascading outages across the energy industry with verified infrastructure.";
        let ind_ch = make_chunk(ind_src);
        let v_ind_drop = validate_chunk_with_mode(&ind_ch, ind_dropped, &o, RewriteMode::Standard, Intensity::Balanced);
        assert!(
            v_ind_drop.polarity_mismatch_flag,
            "benign 'industry'/'infrastructure' must not mask dropped 'cannot'/'without'"
        );

        let pos_src = "University researchers implemented new information workflows across the energy industry.";
        let pos_invented_neg = "University researchers did not implement new information workflows across the energy industry.";
        let pos_ch = make_chunk(pos_src);
        let v_pos_neg = validate_chunk_with_mode(&pos_ch, pos_invented_neg, &o, RewriteMode::Standard, Intensity::Balanced);
        assert!(
            v_pos_neg.polarity_mismatch_flag,
            "benign 'university'/'implement'/'information'/'industry' must not mask invented 'not'"
        );

        // High-invariant dense chunk: near-copy still fails, genuine clause reorder passes
        let dense_src = "On January 14, 2025, researchers at Northbridge Labs reported that distributed edge nodes reduced inference latency by 18.4% while trimming operating costs from $4.8 million to -$250,000 net variance (Smith & Patel, 2024, pp. 42–45). According to the audit [12], \"exact quoted statements must remain unaltered during automated processing\" as documented at https://example.org/audits/2025 and 10.1038/s41586-024-07123-4.";
        let dense_ch = make_chunk(dense_src);
        let dense_near_copy = "On January 14, 2025, researchers at Northbridge Labs noted that distributed edge nodes reduced inference latency by 18.4% while decreasing operating costs from $4.8 million to a net variance of -$250,000 (Smith & Patel, 2024, pp. 42–45). According to the audit [12], \"exact quoted statements must remain unaltered during automated processing\" as documented at https://example.org/audits/2025 and 10.1038/s41586-024-07123-4.";
        let v_dense_copy = validate_chunk_with_mode(&dense_ch, dense_near_copy, &o, RewriteMode::Academic, Intensity::Balanced);
        assert!(v_dense_copy.verbatim_echo_flag, "near-copy of dense invariant chunk must still fail echo check");

        let dense_good = "Distributed edge nodes lowered inference latency by 18.4% and narrowed operating expenses from $4.8 million to a -$250,000 net variance, Northbridge Labs reported on January 14, 2025 (Smith & Patel, 2024, pp. 42–45). As recorded under https://example.org/audits/2025 and 10.1038/s41586-024-07123-4, audit [12] specifies that \"exact quoted statements must remain unaltered during automated processing\".";
        let v_dense_good = validate_chunk_with_mode(&dense_ch, dense_good, &o, RewriteMode::Academic, Intensity::Balanced);
        assert!(
            !v_dense_good.verbatim_echo_flag,
            "genuine clause-inverted rewrite of dense invariant chunk must pass: lex={:.3}, 4gram={:.3}, seq={:.3}",
            v_dense_good.lex_dist,
            v_dense_good.ngram4_overlap_ratio,
            v_dense_good.sequence_similarity
        );
        assert!(v_dense_good.missing_invariants.is_empty());

        // Double-negative preservation idiom ("without sacrificing X" -> "while maintaining X") passes,
        // whereas dropping it to "sacrificed X" or dropping "without touching" still fails.
        let idiom_src = "The pipeline preserved factual accuracy without sacrificing natural cadence.";
        let idiom_ch = make_chunk(idiom_src);
        let idiom_ok = "Factual accuracy stayed intact while natural cadence was maintained by the pipeline.";
        let v_idiom_ok = validate_chunk_with_mode(&idiom_ch, idiom_ok, &o, RewriteMode::Standard, Intensity::Balanced);
        assert!(!v_idiom_ok.polarity_mismatch_flag, "without sacrificing -> maintaining should pass: {:?}", v_idiom_ok.polarity_reason);

        let idiom_bad = "Factual accuracy declined as the pipeline sacrificed natural cadence.";
        let v_idiom_bad = validate_chunk_with_mode(&idiom_ch, idiom_bad, &o, RewriteMode::Standard, Intensity::Balanced);
        assert!(v_idiom_bad.polarity_mismatch_flag);
    }

    #[test]
    fn validator_excludes_required_verbatim_quotes_from_4gram_copy_penalty() {
        let src = "According to the charter, \"automated triage systems must remain auditable by attending physicians\" before hospital deployment.";
        let ch = make_chunk(src);
        let o = opts();
        let rewritten = "Before hospitals deploy them, \"automated triage systems must remain auditable by attending physicians\", the charter mandates.";
        let val = validate_chunk_with_mode(&ch, rewritten, &o, RewriteMode::Professional, Intensity::Balanced);
        assert!(!val.verbatim_echo_flag, "required verbatim quote must not trigger 4-gram copy rejection");
        assert!(val.missing_invariants.is_empty());
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

    #[test]
    fn restrictive_only_qualifier_is_enforced_and_equivalent_restrictors_allowed() {
        let o = opts();
        let src = "Deploying clinical decision models without continuous post-market auditing violates hospital safety protocols, because these systems only maintain 94.2% sensitivity when trained on balanced cohorts from 2023–2025.";
        let dropped_only = "Hospital safety rules prohibit launching clinical decision tools without ongoing post-deployment audits, since these platforms achieve a sensitivity rate of 94.2% when built on balanced cohorts from 2023–2025.";
        let ch = make_chunk(src);
        let val_drop = validate_chunk_with_mode(&ch, dropped_only, &o, RewriteMode::Professional, Intensity::Balanced);
        assert!(val_drop.polarity_mismatch_flag, "dropping 'only' must trigger polarity mismatch");
        assert!(is_fatal_chunk_failure(&val_drop));

        for valid_equiv in [
            "Hospital safety rules prohibit launching clinical decision tools without ongoing post-deployment audits, since these platforms solely achieve 94.2% sensitivity when trained on balanced 2023–2025 cohorts.",
            "Hospital safety rules prohibit launching clinical decision tools without ongoing post-deployment audits, since these platforms reach no more than 94.2% sensitivity when trained on balanced 2023–2025 cohorts.",
            "Hospital safety rules prohibit launching clinical decision tools without ongoing post-deployment audits, since these platforms reach at most 94.2% sensitivity when trained on balanced 2023–2025 cohorts.",
            "Hospital safety rules prohibit launching clinical decision tools without ongoing post-deployment audits, since these platforms exclusively retain 94.2% sensitivity when trained on balanced 2023–2025 cohorts.",
            "Hospital safety rules prohibit launching clinical decision tools without ongoing post-deployment audits, since these platforms remain capped at 94.2% sensitivity when trained on balanced 2023–2025 cohorts.",
        ] {
            let val_ok = validate_chunk_with_mode(&ch, valid_equiv, &o, RewriteMode::Professional, Intensity::Balanced);
            assert!(
                !val_ok.polarity_mismatch_flag,
                "equivalent restrictor should pass: {valid_equiv:?}, reason={:?}",
                val_ok.polarity_reason
            );
        }

        // Additive "not only ... but also" must NOT trigger false positive when paraphrased to "both ... and"
        let additive_src = "The update not only lowered inference latency by 12% but also reduced memory usage.";
        let additive_out = "Both inference latency (down 12%) and memory consumption improved after the update.";
        assert!(detect_polarity_mismatch(additive_src, additive_out, &o).is_none());
    }

    #[test]
    fn missing_invariants_duplicated_items_and_identifier_subject_syntax_are_fatal() {
        let o = opts();
        let src = "According to the audit [12], \"exact quoted statements must remain unaltered during automated processing\" as documented at https://example.org/audits/2025 and 10.1038/s41586-024-07123-4.";
        let ch = make_chunk(src);

        // Missing [12] must be fatal
        let dropped_cite = "As documented at https://example.org/audits/2025 and 10.1038/s41586-024-07123-4, the audit confirms that \"exact quoted statements must remain unaltered during automated processing\".";
        let v_drop = validate_chunk_with_mode(&ch, dropped_cite, &o, RewriteMode::Standard, Intensity::Balanced);
        assert!(v_drop.missing_invariants.iter().any(|m| m.contains("[12]")));
        assert!(is_fatal_chunk_failure(&v_drop));

        // Duplicated DOI must flag leaked_placeholder + duplicated_invariants and be fatal
        let dup_doi = "In 10.1038/s41586-024-07123-4 and https://example.org/audits/2025, the audit [12] notes that \"exact quoted statements must remain unaltered during automated processing\", per 10.1038/s41586-024-07123-4.";
        let v_dup = validate_chunk_with_mode(&ch, dup_doi, &o, RewriteMode::Standard, Intensity::Balanced);
        assert!(v_dup.leaked_placeholder);
        assert!(!v_dup.duplicated_invariants.is_empty());
        assert!(is_fatal_chunk_failure(&v_dup));

        // Duplicated quote must also be flagged and fatal
        let dup_quote = "As noted in \"exact quoted statements must remain unaltered during automated processing\", the audit [12] at https://example.org/audits/2025 and 10.1038/s41586-024-07123-4 highlights \"exact quoted statements must remain unaltered during automated processing\".";
        let v_dup_q = validate_chunk_with_mode(&ch, dup_quote, &o, RewriteMode::Standard, Intensity::Balanced);
        assert!(!v_dup_q.duplicated_invariants.is_empty());
        assert!(is_fatal_chunk_failure(&v_dup_q));

        // Malformed DOI/URL subject + reporting verb must be flagged and fatal
        let bad_grammar = "10.1038/s41586-024-07123-4 documents \"exact quoted statements must remain unaltered during automated processing\", as recorded at https://example.org/audits/2025 in audit [12].";
        let v_bad_syn = validate_chunk_with_mode(&ch, bad_grammar, &o, RewriteMode::Academic, Intensity::Balanced);
        assert!(v_bad_syn.malformed_syntax_flag);
        assert!(is_fatal_chunk_failure(&v_bad_syn));

        // Clean passive reordering passes with ultra-dense invariant calibration
        let clean = "As documented at https://example.org/audits/2025 and 10.1038/s41586-024-07123-4, the audit [12] notes that \"exact quoted statements must remain unaltered during automated processing\".";
        let v_clean = validate_chunk_with_mode(&ch, clean, &o, RewriteMode::Standard, Intensity::Balanced);
        assert!(!v_clean.verbatim_echo_flag);
        assert!(!v_clean.malformed_syntax_flag);
        assert!(!v_clean.leaked_placeholder);
        assert!(v_clean.missing_invariants.is_empty());
        assert!(!is_fatal_chunk_failure(&v_clean));
    }
}
