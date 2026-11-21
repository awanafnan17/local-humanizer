use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;

use crate::commands::RewriteOptions;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InvariantSet {
    pub code_blocks: Vec<String>,
    pub latex_spans: Vec<String>,
    pub urls: Vec<String>,
    pub emails: Vec<String>,
    pub dois: Vec<String>,
    pub quoted_strings: Vec<String>,
    pub citations: Vec<String>,
    pub currencies: Vec<String>,
    pub dates: Vec<String>,
    pub numbers: Vec<String>,
    pub entities: Vec<String>,
}

impl InvariantSet {
    /// Returns `(category_label, value, exact_match_mode)` tuples enforced for the given options.
    pub fn enforced_items(&self, opts: &RewriteOptions) -> Vec<(&'static str, &str, bool)> {
        let mut out = Vec::new();
        for s in &self.code_blocks {
            out.push(("code", s.as_str(), true));
        }
        for s in &self.latex_spans {
            out.push(("latex", s.as_str(), true));
        }
        for s in &self.urls {
            out.push(("url", s.as_str(), true));
        }
        for s in &self.emails {
            out.push(("email", s.as_str(), true));
        }
        for s in &self.dois {
            out.push(("doi", s.as_str(), true));
        }
        if opts.preserve_quotes {
            for s in &self.quoted_strings {
                out.push(("quote", s.as_str(), true));
            }
        }
        if opts.preserve_citations {
            for s in &self.citations {
                out.push(("citation", s.as_str(), true));
            }
        }
        for s in &self.currencies {
            out.push(("currency", s.as_str(), false));
        }
        for s in &self.dates {
            out.push(("date", s.as_str(), false));
        }
        for s in &self.numbers {
            out.push(("number", s.as_str(), false));
        }
        out
    }

    pub fn is_empty_enforced(&self, opts: &RewriteOptions) -> bool {
        self.enforced_items(opts).is_empty()
    }
}

/// Byte intervals `(start, end)` that sentence splitters must NOT split inside.
pub fn protected_split_intervals(text: &str) -> Vec<(usize, usize)> {
    let mut intervals = Vec::new();
    let inline_code = scan_inline_code_intervals(text);
    intervals.extend(inline_code.iter().copied());
    for m in url_re().find_iter(text) {
        let trimmed = trim_url_trailing(&text[m.start()..m.end()]);
        intervals.push((m.start(), m.start() + trimmed.len()));
    }
    for m in doi_re().find_iter(text) {
        let trimmed = trim_url_trailing(&text[m.start()..m.end()]);
        intervals.push((m.start(), m.start() + trimmed.len()));
    }
    for (s, e, _) in scan_quoted_strings(text, &inline_code) {
        intervals.push((s, e));
    }
    for m in apa_paren_re().find_iter(text) {
        if !is_non_author_headword(apa_paren_headword(m.as_str())) {
            intervals.push((m.start(), m.end()));
        }
    }
    for m in ieee_re().find_iter(text) {
        intervals.push((m.start(), m.end()));
    }
    intervals.sort_by_key(|r| r.0);
    intervals
}

fn mask_bytes(buf: &mut [u8], start: usize, end: usize) {
    for b in &mut buf[start..end] {
        if *b != b'\n' {
            *b = b' ';
        }
    }
}

/// Replace any non-ASCII UTF-8 bytes in a shadow buffer with `' '`? No: UTF-8 regexes require valid
/// UTF-8! Instead, use a `String` shadow buffer where masked spans are replaced by ASCII spaces of
/// the exact same byte length (preserving byte offsets 100%).
fn mask_utf8_span(shadow: &mut String, start: usize, end: usize) {
    // Safety: caller guarantees `start` and `end` are valid UTF-8 char boundaries in `shadow`.
    let bytes = unsafe { shadow.as_bytes_mut() };
    mask_bytes(bytes, start, end);
}

fn push_unique(vec: &mut Vec<String>, seen: &mut HashSet<String>, val: String) {
    if !val.is_empty() && seen.insert(val.clone()) {
        vec.push(val);
    }
}

fn trim_url_trailing(raw: &str) -> &str {
    let mut s = raw;
    loop {
        let mut changed = false;
        while let Some(last) = s.chars().last() {
            if matches!(
                last,
                '.' | ',' | ';' | ':' | '!' | '?' | '"' | '\'' | '“' | '”' | '‘' | '’'
            ) {
                s = &s[..s.len() - last.len_utf8()];
                changed = true;
            } else if last == ')' && s.matches(')').count() > s.matches('(').count() {
                s = &s[..s.len() - 1];
                changed = true;
            } else if last == ']' && s.matches(']').count() > s.matches('[').count() {
                s = &s[..s.len() - 1];
                changed = true;
            } else if last == '}' && s.matches('}').count() > s.matches('{').count() {
                s = &s[..s.len() - 1];
                changed = true;
            } else {
                break;
            }
        }
        if !changed {
            break;
        }
    }
    s
}

fn scan_inline_code_intervals(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let mut ticks = 0usize;
            while i + ticks < bytes.len() && bytes[i + ticks] == b'`' {
                ticks += 1;
            }
            if ticks <= 2 {
                let start = i;
                let mut j = i + ticks;
                let mut found_end = None;
                while j < bytes.len() {
                    if bytes[j] == b'\n' {
                        break;
                    }
                    if bytes[j] == b'`' {
                        let mut end_ticks = 0usize;
                        while j + end_ticks < bytes.len() && bytes[j + end_ticks] == b'`' {
                            end_ticks += 1;
                        }
                        if end_ticks == ticks {
                            found_end = Some(j + end_ticks);
                            break;
                        }
                        j += end_ticks;
                        continue;
                    }
                    j += 1;
                }
                if let Some(end) = found_end {
                    if end > start + ticks * 2 {
                        out.push((start, end));
                    }
                    i = end;
                    continue;
                }
            }
            i += ticks;
        } else {
            i += 1;
        }
    }
    out
}

fn is_likely_inline_math(inner: &str) -> bool {
    let trimmed = inner.trim();
    if trimmed.is_empty() || trimmed.len() > 160 {
        return false;
    }
    // Avoid mistaking "$10 and $20" for math
    if inner.starts_with(char::is_whitespace) || inner.ends_with(char::is_whitespace) {
        return false;
    }
    // Avoid mistaking "$10+$20", "$10-$20", or "$10=$US$20" currency expressions for LaTeX
    if trimmed.starts_with(|c: char| c.is_ascii_digit())
        && trimmed.ends_with(|c: char| {
            matches!(c, '+' | '-' | '\u{2013}' | '*' | '/' | '=' | '<' | '>')
        })
    {
        return false;
    }
    let has_math_op = trimmed
        .chars()
        .any(|c| matches!(c, '\\' | '^' | '_' | '{' | '}' | '=' | '+' | '<' | '>' | '*'));
    if has_math_op {
        return true;
    }
    // Single-letter math variable like $x$ or $n$
    if trimmed.len() == 1 && trimmed.chars().all(|c| c.is_ascii_alphabetic()) {
        return true;
    }
    // Function calls / parenthesized math like $f(x)$, $g(x, y)$, $P(A|B)$
    let has_parens = trimmed.contains('(')
        && trimmed.contains(')')
        && !trimmed.starts_with(|c: char| c.is_ascii_digit())
        && !trimmed.ends_with('(')
        && trimmed.chars().any(|c| c.is_ascii_alphabetic())
        && trimmed.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(
                    c,
                    '(' | ')' | '[' | ']' | ',' | '.' | ' ' | '\'' | '+' | '-' | '*' | '/' | '|'
                )
        });
    if has_parens {
        return true;
    }
    // Binary expressions without '=' such as $a-b$, $x-1$, $x - y$, $a/b$, $x'$
    trimmed.chars().any(|c| matches!(c, '-' | '/' | '\''))
        && !trimmed.starts_with(|c: char| c.is_ascii_digit())
        && !trimmed.ends_with(|c: char| matches!(c, '+' | '-' | '\u{2013}' | '*' | '/'))
        && trimmed.chars().any(|c| c.is_ascii_alphabetic())
        && trimmed.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '-' | '/' | '\'' | ' ' | '.' | ',' | '(' | ')')
        })
}

fn scan_inline_latex_intervals(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            let escaped = i > 0 && bytes[i - 1] == b'\\';
            if escaped {
                i += 1;
                continue;
            }
            if i + 1 < bytes.len() && bytes[i + 1] == b'$' {
                if let Some(rel_end) = text[i + 2..].find("$$") {
                    let end = i + 2 + rel_end + 2;
                    out.push((i, end));
                    i = end;
                    continue;
                }
                i += 2;
                continue;
            }
            let mut j = i + 1;
            let mut close = None;
            while j < bytes.len() {
                if bytes[j] == b'\n' {
                    break;
                }
                if bytes[j] == b'$' && bytes[j - 1] != b'\\' {
                    close = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(c) = close {
                let inner = &text[i + 1..c];
                let followed_by_digit = bytes
                    .get(c + 1)
                    .map(|b| b.is_ascii_digit())
                    .unwrap_or(false);
                if !followed_by_digit && is_likely_inline_math(inner) {
                    out.push((i, c + 1));
                    i = c + 1;
                    continue;
                }
            }
            i += 1;
        } else {
            i += 1;
        }
    }
    out
}

fn in_intervals(pos: usize, intervals: &[(usize, usize)]) -> bool {
    intervals.iter().any(|&(s, e)| pos >= s && pos < e)
}

fn scan_quoted_strings(
    text: &str,
    code_intervals: &[(usize, usize)],
) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((start_idx, ch)) = chars.next() {
        if in_intervals(start_idx, code_intervals) {
            continue;
        }
        if ch == '"' {
            let prev_is_escape = start_idx > 0 && text.as_bytes()[start_idx - 1] == b'\\';
            if prev_is_escape {
                continue;
            }
            let mut end_opt = None;
            while let Some((idx, c2)) = chars.next() {
                if c2 == '\n' {
                    break;
                }
                if c2 == '\\' {
                    chars.next();
                    continue;
                }
                if c2 == '"' {
                    end_opt = Some(idx + 1);
                    break;
                }
            }
            if let Some(end_idx) = end_opt {
                let slice = &text[start_idx..end_idx];
                if slice.len() >= 4 {
                    out.push((start_idx, end_idx, slice.to_string()));
                }
            }
        } else if ch == '“' {
            let mut end_opt = None;
            while let Some((idx, c2)) = chars.next() {
                if c2 == '\n' {
                    break;
                }
                if c2 == '”' {
                    end_opt = Some(idx + c2.len_utf8());
                    break;
                }
            }
            if let Some(end_idx) = end_opt {
                let slice = &text[start_idx..end_idx];
                if slice.chars().count() >= 4 {
                    out.push((start_idx, end_idx, slice.to_string()));
                }
            }
        }
    }
    out
}

fn url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"https?://[^\s<>"'\u{201c}\u{201d}\u{2018}\u{2019}]+"#).unwrap())
}

fn email_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").unwrap())
}

fn doi_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"\b10\.\d{4,9}/[^\s"'<>\u{201c}\u{201d}\u{2018}\u{2019}]+"#).unwrap())
}

fn apa_paren_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"\(\s*(?:(?:e\.g\.|i\.e\.|cf\.|for\s+example),?\s+|see\s+(?:also\s+)?)?(?:(?:de|da|del|della|der|di|du|dos|das|van|von|la|le)\s+){0,2}[A-Z][A-Za-z'\u{2019}.-]+(?:\s+(?:[A-Z][A-Za-z'\u{2019}.-]*|et\s+al\.?|and|&|of|for|the|de|da|del|della|der|di|du|dos|das|van|von|la|le)|,\s*(?:[A-Z][A-Za-z'\u{2019}.-]*|&|and|et\s+al\.?)){0,8},\s*(?:19|20)\d{2}[a-z]?(?:\s*,\s*(?:pp?\.\s*\d+(?:\s*[-–]\s*\d+)?|ch\.\s*\d+|fig\.\s*\d+|sec\.\s*\d+|para\.\s*\d+|\d+[-–]\d+))?(?:\s*;\s*(?:(?:de|da|del|della|der|di|du|dos|das|van|von|la|le)\s+){0,2}[A-Z][A-Za-z'\u{2019}.-]+(?:\s+(?:[A-Z][A-Za-z'\u{2019}.-]*|et\s+al\.?|and|&|of|for|the|de|da|del|della|der|di|du|dos|das|van|von|la|le)|,\s*(?:[A-Z][A-Za-z'\u{2019}.-]*|&|and|et\s+al\.?)){0,8},\s*(?:19|20)\d{2}[a-z]?(?:\s*,\s*(?:pp?\.\s*\d+(?:\s*[-–]\s*\d+)?|ch\.\s*\d+|fig\.\s*\d+|sec\.\s*\d+|para\.\s*\d+|\d+[-–]\d+))?)*\s*\)",
        )
        .unwrap()
    })
}

fn apa_paren_headword(cite: &str) -> &str {
    let mut s = cite.trim().trim_start_matches('(').trim_start();
    for prefix in ["e.g.,", "e.g.", "i.e.,", "i.e.", "cf.,", "cf.", "for example,", "for example", "see also", "see"] {
        if let Some(rest) = s.get(..prefix.len()).filter(|p| p.eq_ignore_ascii_case(prefix)) {
            let _ = rest;
            s = s[prefix.len()..].trim_start();
            break;
        }
    }
    s
}

fn ieee_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\[\s*\d+(?:\s*[-–,]\s*\d+)*\s*\](?:\s*[-–]\s*\[\s*\d+\s*\])?").unwrap()
    })
}

fn narrative_apa_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"\b([A-Z][A-Za-z'\u{2019}-]+(?:\s+(?:and|&)\s+[A-Z][A-Za-z'\u{2019}-]+|\s+et\s+al\.?)?)\s*\(((?:19|20)\d{2}[a-z]?)\)",
        )
        .unwrap()
    })
}

fn currency_range_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?:[-\u{2013}+]?(?:US|CA|AU|NZ|HK|SG)?[$€£¥₹][ \t\u{00A0}]?\d+(?:,\d{3})*(?:\.\d+)?(?:[ \t\u{00A0}]?[kKmMbBtT]\b|[KMBT]\b)?(?:[ \t\u{00A0}]*[-\u{2013}][ \t\u{00A0}]*(?:[-\u{2013}+]?(?:US|CA|AU|NZ|HK|SG)?[$€£¥₹][ \t\u{00A0}]?)?\d+(?:,\d{3})*(?:\.\d+)?(?:[ \t\u{00A0}]?[kKmMbBtT]\b|[KMBT]\b)?)+|[-\u{2013}+]?\b(?:USD|EUR|GBP|JPY|CHF|CAD|AUD|INR|CNY)[ \t\u{00A0}]+[-\u{2013}+]?\d+(?:,\d{3})*(?:\.\d+)?(?:[ \t\u{00A0}]?[kKmMbBtT]\b)?(?:[ \t\u{00A0}]*[-\u{2013}][ \t\u{00A0}]*(?:[-\u{2013}+]?(?:USD|EUR|GBP|JPY|CHF|CAD|AUD|INR|CNY)[ \t\u{00A0}]+[-\u{2013}+]?)?\d+(?:,\d{3})*(?:\.\d+)?(?:[ \t\u{00A0}]?[kKmMbBtT]\b)?)+|[-\u{2013}+]?\b\d+(?:,\d{3})*(?:\.\d+)?(?:[ \t\u{00A0}]?[kKmMbBtT]\b)?(?:[ \t\u{00A0}]?(?:USD|EUR|GBP|JPY|CHF|CAD|AUD|INR|CNY))?(?:[ \t\u{00A0}]*[-\u{2013}][ \t\u{00A0}]*[-\u{2013}+]?\d+(?:,\d{3})*(?:\.\d+)?(?:[ \t\u{00A0}]?[kKmMbBtT]\b)?(?:[ \t\u{00A0}]?(?:USD|EUR|GBP|JPY|CHF|CAD|AUD|INR|CNY))?)*[ \t\u{00A0}]*[-\u{2013}][ \t\u{00A0}]*[-\u{2013}+]?\d+(?:,\d{3})*(?:\.\d+)?(?:[ \t\u{00A0}]?[kKmMbBtT]\b)?[ \t\u{00A0}]?(?:USD|EUR|GBP|JPY|CHF|CAD|AUD|INR|CNY)\b)",
        )
        .unwrap()
    })
}

fn currency_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?:[-\u{2013}+]?(?:US|CA|AU|NZ|HK|SG)?[$€£¥₹][ \t\u{00A0}]?\d+(?:,\d{3})*(?:\.\d+)?(?:[ \t\u{00A0}]?[kKmMbBtT]\b|[KMBT]\b)?|[-\u{2013}+]?\b(?:USD|EUR|GBP|JPY|CHF|CAD|AUD|INR|CNY)[ \t\u{00A0}]+\d+(?:,\d{3})*(?:\.\d+)?(?:[ \t\u{00A0}]?[kKmMbBtT]\b)?|\b(?:USD|EUR|GBP|JPY|CHF|CAD|AUD|INR|CNY)[ \t\u{00A0}]+[-\u{2013}+]?\d+(?:,\d{3})*(?:\.\d+)?(?:[ \t\u{00A0}]?[kKmMbBtT]\b)?|[-\u{2013}+]?\b\d+(?:,\d{3})*(?:\.\d+)?[ \t\u{00A0}]?(?:USD|EUR|GBP|JPY|CHF|CAD|AUD|INR|CNY)\b)",
        )
        .unwrap()
    })
}

fn numeric_range_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"[-\u{2013}+]?\b\d+(?:,\d{3})*(?:\.\d+)?%?(?:\s*[-\u{2013}]\s*\d+(?:,\d{3})*(?:\.\d+)?%?)*\s*[-\u{2013}]\s*\d+(?:,\d{3})*(?:\.\d+)?(?:%|\b)",
        )
        .unwrap()
    })
}

fn semver_ip_sci_pval_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?:\bv\d+\.\d+(?:\.\d+)*(?:[-\u{2013}](?:[A-Za-z.+_\u{2013}-]+(?:\d+(?:,\d{3})*[A-Za-z.+_\u{2013}-]+)*(?:\d+(?:,\d{3})*)?|\d+(?:,\d{3})*(?:[A-Za-z.+_\u{2013}-]+\d+(?:,\d{3})*)*[A-Za-z.+_\u{2013}-]*)(?:%[-\u{2013}](?:[A-Za-z.+_\u{2013}-]+(?:\d+(?:,\d{3})*[A-Za-z.+_\u{2013}-]+)*(?:\d+(?:,\d{3})*)?|\d+(?:,\d{3})*(?:[A-Za-z.+_\u{2013}-]+\d+(?:,\d{3})*)*[A-Za-z.+_\u{2013}-]*))*)?(?:%|\b)|\b\d+(?:\.\d+){2,}(?:[-\u{2013}](?:[A-Za-z.+_\u{2013}-]+(?:\d+(?:,\d{3})*[A-Za-z.+_\u{2013}-]+)*(?:\d+(?:,\d{3})*)?|\d+(?:,\d{3})*(?:[A-Za-z.+_\u{2013}-]+\d+(?:,\d{3})*)*[A-Za-z.+_\u{2013}-]*)(?:%[-\u{2013}](?:[A-Za-z.+_\u{2013}-]+(?:\d+(?:,\d{3})*[A-Za-z.+_\u{2013}-]+)*(?:\d+(?:,\d{3})*)?|\d+(?:,\d{3})*(?:[A-Za-z.+_\u{2013}-]+\d+(?:,\d{3})*)*[A-Za-z.+_\u{2013}-]*))*)?(?:%|\b)|[-\u{2013}+]?\b\d+(?:,\d{3})*(?:\.\d+)?(?:[eE][+-]?\d+)?(?:%?[-\u{2013}]\d+(?:,\d{3})*(?:\.\d+)?(?:[eE][+-]?\d+)?)*%?[-\u{2013}](?:v\d+\.\d+(?:\.\d+)*|\d+(?:\.\d+){2,})(?:[-\u{2013}](?:[A-Za-z.+_\u{2013}-]+(?:\d+(?:,\d{3})*[A-Za-z.+_\u{2013}-]+)*(?:\d+(?:,\d{3})*)?|\d+(?:,\d{3})*(?:[A-Za-z.+_\u{2013}-]+\d+(?:,\d{3})*)*[A-Za-z.+_\u{2013}-]*)(?:%[-\u{2013}](?:[A-Za-z.+_\u{2013}-]+(?:\d+(?:,\d{3})*[A-Za-z.+_\u{2013}-]+)*(?:\d+(?:,\d{3})*)?|\d+(?:,\d{3})*(?:[A-Za-z.+_\u{2013}-]+\d+(?:,\d{3})*)*[A-Za-z.+_\u{2013}-]*))*)?(?:%|\b)|[-\u{2013}+]?\b\d+(?:,\d{3})*(?:\.\d+)?[eE][+-]?\d+(?:%|\b)(?:\s*[-\u{2013}]\s*[-+]?\d+(?:,\d{3})*(?:\.\d+)?(?:[eE][+-]?\d+)?(?:%|\b))*|[-\u{2013}+]?\b\d+(?:,\d{3})*(?:\.\d+)?(?:%|\b)(?:\s*[-\u{2013}]\s*[-+]?\d+(?:,\d{3})*(?:\.\d+)?(?:%|\b))*\s*[-\u{2013}]\s*[-+]?\d+(?:,\d{3})*(?:\.\d+)?[eE][+-]?\d+(?:%|\b)(?:\s*[-\u{2013}]\s*[-+]?\d+(?:,\d{3})*(?:\.\d+)?(?:[eE][+-]?\d+)?(?:%|\b))*|\bp\s*[<=>≤≥]\s*0?\.\d+\b)",
        )
        .unwrap()
    })
}

fn date_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?:\b\d{4}-\d{2}-\d{2}(?:[T ]\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?:Z|[+-]\d{2}:?\d{2}|\s?(?:UTC|GMT|EST|EDT|PST|PDT))?)?(?:\s*[-\u{2013}]\s*\d{4}-\d{2}-\d{2}(?:[T ]\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?:Z|[+-]\d{2}:?\d{2}|\s?(?:UTC|GMT|EST|EDT|PST|PDT))?)?)?\b|\b\d{1,2}/\d{1,2}/\d{2,4}\b|\b(?:January|February|March|April|May|June|July|August|September|October|November|December|Jan|Feb|Mar|Apr|Jun|Jul|Aug|Sep|Sept|Oct|Nov|Dec)\.?\s+\d{1,2}(?:st|nd|rd|th)?(?:,\s*\d{4})?\b|\b\d{1,2}\s+(?:January|February|March|April|May|June|July|August|September|October|November|December|Jan|Feb|Mar|Apr|Jun|Jul|Aug|Sep|Sept|Oct|Nov|Dec)\.?(?:,?\s+\d{4})?\b|\b(?:January|February|March|April|May|June|July|August|September|October|November|December)\s+\d{4}\b)",
        )
        .unwrap()
    })
}

fn tech_ident_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b\p{L}[\p{L}\p{M}\p{N}_]*(?:\.[\p{L}\p{M}\p{N}_]+)*(?:[-\u{2013}][\p{L}\p{M}\p{N}_.]+)+\b").unwrap()
    })
}

fn number_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[-\u{2013}+]?\d+(?:,\d{3})*(?:\.\d+)?%?").unwrap())
}

fn entity_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"\b(?:(?:Dr|Prof|Mr|Ms|Mrs)\.\s+[A-Z][a-z]+(?:\s+[A-Z][a-z]+)?|[A-Z][a-z]{2,}(?:\s+(?:of|for|and|the|de|van|von)\s+|\s+)[A-Z][a-z]{2,}(?:\s+[A-Z][a-z]{2,})*)\b",
        )
        .unwrap()
    })
}

fn is_non_author_headword(head: &str) -> bool {
    let first = head
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        first.as_str(),
        "section"
            | "table"
            | "figure"
            | "fig"
            | "gate"
            | "step"
            | "phase"
            | "version"
            | "model"
            | "release"
            | "appendix"
            | "chapter"
            | "part"
            | "january"
            | "february"
            | "march"
            | "april"
            | "may"
            | "june"
            | "july"
            | "august"
            | "september"
            | "october"
            | "november"
            | "december"
    )
}

fn trailing_alpha_ident_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\p{L}[\p{L}\p{M}\p{N}_]*$").unwrap())
}

fn trim_range_sign_prefix(text: &str, shadow: &str, s: usize, e: usize) -> usize {
    let sign_len = if shadow[s..].starts_with('-') || shadow[s..].starts_with('+') {
        1
    } else if shadow[s..].starts_with('\u{2013}') {
        '\u{2013}'.len_utf8()
    } else {
        0
    };
    if sign_len > 0 {
        let prev_char = text[..s].chars().next_back();
        let prev_is_range_anchor = matches!(
            prev_char,
            Some(c) if c.is_alphanumeric() || matches!(c, '_' | '%' | ')' | ']')
        ) || trailing_alpha_ident_re().is_match(&text[..s])
            || (s > 0
                && shadow.as_bytes()[s - 1] == b' '
                && prev_char.map(|c| c != ' ' && c != '\n').unwrap_or(false));
        if prev_is_range_anchor {
            let trimmed = shadow[s + sign_len..e].trim();
            if is_left_boundary_ok(text, s + sign_len, trimmed) {
                return s + sign_len;
            }
        }
    }
    s
}

fn collapse_wrapped_hyphens_pair(text: &str, shadow: &str) -> (String, String) {
    debug_assert_eq!(text.len(), shadow.len());
    let mut out_text = String::with_capacity(text.len());
    let mut out_shadow = String::with_capacity(shadow.len());
    let mut i = 0usize;
    while i < text.len() {
        let Some(ch) = text[i..].chars().next() else {
            break;
        };
        if !ch.is_whitespace() {
            let next_i = i + ch.len_utf8();
            out_text.push(ch);
            out_shadow.push_str(&shadow[i..next_i]);
            i = next_i;
            continue;
        }
        let ws_start = i;
        let mut newlines = 0usize;
        while i < text.len() {
            let Some(wch) = text[i..].chars().next() else {
                break;
            };
            if !wch.is_whitespace() {
                break;
            }
            if wch == '\r' {
                newlines += 1;
                i += 1;
                if text[i..].starts_with('\n') {
                    i += 1;
                }
            } else if wch == '\n' {
                newlines += 1;
                i += 1;
            } else {
                i += wch.len_utf8();
            }
        }
        let dash_len = if out_text.ends_with('-') {
            Some(1)
        } else if out_text.ends_with('\u{2013}') {
            Some('\u{2013}'.len_utf8())
        } else {
            None
        };
        let wrapped_hyphen = newlines == 1
            && dash_len
                .and_then(|dl| out_text[..out_text.len() - dl].chars().next_back())
                .map(|prev| !prev.is_whitespace() && prev != '-' && prev != '\u{2013}')
                .unwrap_or(false);
        if !wrapped_hyphen {
            out_text.push_str(&text[ws_start..i]);
            out_shadow.push_str(&shadow[ws_start..i]);
        }
    }
    (out_text, out_shadow)
}

fn is_valid_non_exact_span(text: &str, start: usize, end: usize, val: &str) -> bool {
    !val.is_empty()
        && is_left_boundary_ok(text, start, val)
        && is_right_boundary_ok(text, end, val)
}

pub fn extract_invariants(text: &str, opts: &RewriteOptions) -> InvariantSet {
    let mut set = InvariantSet::default();
    if text.trim().is_empty() {
        return set;
    }

    let mut seen_code = HashSet::new();
    let mut seen_latex = HashSet::new();
    let mut seen_url = HashSet::new();
    let mut seen_email = HashSet::new();
    let mut seen_doi = HashSet::new();
    let mut seen_quote = HashSet::new();
    let mut seen_cite = HashSet::new();
    let mut seen_curr = HashSet::new();
    let mut seen_date = HashSet::new();
    let mut seen_num = HashSet::new();
    let mut seen_ent = HashSet::new();

    let mut shadow = text.to_string();

    // 1. Inline code spans
    let code_intervals = scan_inline_code_intervals(text);
    for &(s, e) in &code_intervals {
        push_unique(&mut set.code_blocks, &mut seen_code, text[s..e].to_string());
        mask_utf8_span(&mut shadow, s, e);
    }

    // 2. Quoted strings (scanned on original text skipping inline code, without masking shadow yet
    // so URLs/DOIs/numbers inside quotes are still checked too)
    let quote_intervals = scan_quoted_strings(text, &code_intervals);
    for (_, _, q) in &quote_intervals {
        push_unique(&mut set.quoted_strings, &mut seen_quote, q.clone());
    }

    // 3. Inline LaTeX
    let latex_intervals = scan_inline_latex_intervals(&shadow);
    for &(s, e) in &latex_intervals {
        push_unique(
            &mut set.latex_spans,
            &mut seen_latex,
            shadow[s..e].to_string(),
        );
        mask_utf8_span(&mut shadow, s, e);
    }

    // 4. URLs (plus embedded DOIs)
    let url_spans: Vec<(usize, usize)> = url_re()
        .find_iter(&shadow)
        .map(|m| {
            let trimmed = trim_url_trailing(&shadow[m.start()..m.end()]);
            (m.start(), m.start() + trimmed.len())
        })
        .collect();
    for (s, e) in url_spans {
        let u = shadow[s..e].to_string();
        for dm in doi_re().find_iter(&u) {
            let d = trim_url_trailing(&u[dm.start()..dm.end()]);
            push_unique(&mut set.dois, &mut seen_doi, d.to_string());
        }
        push_unique(&mut set.urls, &mut seen_url, u);
        mask_utf8_span(&mut shadow, s, e);
    }

    // 5. Emails
    let email_spans: Vec<(usize, usize)> = email_re()
        .find_iter(&shadow)
        .map(|m| (m.start(), m.end()))
        .collect();
    for (s, e) in email_spans {
        let em = shadow[s..e].to_string();
        push_unique(&mut set.emails, &mut seen_email, em);
        mask_utf8_span(&mut shadow, s, e);
    }

    // 6. Standalone DOIs
    let doi_spans: Vec<(usize, usize)> = doi_re()
        .find_iter(&shadow)
        .map(|m| {
            let trimmed = trim_url_trailing(&shadow[m.start()..m.end()]);
            (m.start(), m.start() + trimmed.len())
        })
        .collect();
    for (s, e) in doi_spans {
        let d = shadow[s..e].to_string();
        push_unique(&mut set.dois, &mut seen_doi, d);
        mask_utf8_span(&mut shadow, s, e);
    }

    // 7. Citations (parenthetical APA, IEEE brackets, narrative APA)
    let mut cite_spans: Vec<(usize, usize, bool)> = Vec::new();
    for m in apa_paren_re().find_iter(&shadow) {
        if !is_non_author_headword(apa_paren_headword(m.as_str())) {
            cite_spans.push((m.start(), m.end(), true));
        }
    }
    for m in ieee_re().find_iter(&shadow) {
        cite_spans.push((m.start(), m.end(), false));
    }
    for cap in narrative_apa_re().captures_iter(&shadow) {
        if let (Some(full), Some(head)) = (cap.get(0), cap.get(1))
            && !is_non_author_headword(head.as_str())
        {
            cite_spans.push((full.start(), full.end(), true));
        }
    }
    cite_spans.sort_by_key(|r| r.0);
    for (s, e, is_apa) in cite_spans {
        let slice = shadow[s..e].trim();
        if !slice.is_empty() {
            push_unique(&mut set.citations, &mut seen_cite, slice.to_string());
            // When preserve_citations=false, keep numbers/dates/currencies inside APA-like
            // parentheticals visible to numeric extraction; still mask IEEE brackets `[1, 2]`.
            if opts.preserve_citations || !is_apa {
                mask_utf8_span(&mut shadow, s, e);
            }
        }
    }

    // Align Steps 8-14 with `contains_with_boundary(..., false)` single-newline wrapped-hyphen collapsing
    let (collapsed_text, mut step8_shadow) = collapse_wrapped_hyphens_pair(text, &shadow);
    let step8_text = collapsed_text.as_str();

    // 8. Currency ranges & Currencies (guard against swallowing dates like "2024-11-21 USD")
    let curr_date_spans: Vec<(usize, usize)> = date_re()
        .find_iter(&step8_shadow)
        .filter(|m| {
            let prev_is_currency_sym = step8_shadow[..m.start()]
                .chars()
                .next_back()
                .map(|c| matches!(c, '$' | '€' | '£' | '¥' | '₹'))
                .unwrap_or(false);
            !prev_is_currency_sym
        })
        .map(|m| (m.start(), m.end()))
        .collect();
    let mut curr_shadow = step8_shadow.clone();
    for &(ds, de) in &curr_date_spans {
        mask_utf8_span(&mut curr_shadow, ds, de);
    }

    fn is_date_continuation_currency(text: &str, curr_shadow: &str, start: usize) -> bool {
        if curr_shadow[start..].starts_with(['-', '\u{2013}']) && start > 0 {
            let masked_before = curr_shadow.as_bytes()[start - 1] != text.as_bytes()[start - 1];
            let digit_before = text[..start]
                .chars()
                .next_back()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false);
            if masked_before && digit_before {
                return true;
            }
        }
        if let Some(before_dash) = text[..start].strip_suffix(['-', '\u{2013}']) {
            let b_len = before_dash.len();
            if b_len > 0 {
                let masked_before =
                    curr_shadow.as_bytes()[b_len - 1] != text.as_bytes()[b_len - 1];
                let digit_before = before_dash
                    .chars()
                    .next_back()
                    .map(|c| c.is_ascii_digit())
                    .unwrap_or(false);
                if masked_before && digit_before {
                    return true;
                }
            }
        }
        false
    }

    let mut rejected_curr_ranges: Vec<(usize, usize)> = Vec::new();
    let mut curr_range_spans: Vec<(usize, usize)> = Vec::new();
    for m in currency_range_re().find_iter(&curr_shadow) {
        if is_date_continuation_currency(step8_text, &curr_shadow, m.start()) {
            rejected_curr_ranges.push((m.start(), m.end()));
        } else {
            curr_range_spans.push((m.start(), m.end()));
        }
    }
    for (mut s, mut e) in curr_range_spans {
        s = trim_range_sign_prefix(step8_text, &step8_shadow, s, e);
        let raw = &step8_shadow[s..e];
        let trimmed = raw.trim();
        let lead = raw.len() - raw.trim_start().len();
        s += lead;
        e = s + trimmed.len();
        if !is_valid_non_exact_span(step8_text, s, e, trimmed) {
            continue;
        }
        push_unique(&mut set.currencies, &mut seen_curr, trimmed.to_string());
        mask_utf8_span(&mut step8_shadow, s, e);
        mask_utf8_span(&mut curr_shadow, s, e);
    }

    let curr_spans: Vec<(usize, usize)> = currency_re()
        .find_iter(&curr_shadow)
        .filter(|m| {
            if is_date_continuation_currency(step8_text, &curr_shadow, m.start()) {
                return false;
            }
            if rejected_curr_ranges
                .iter()
                .any(|&(rs, re)| m.start() >= rs && m.end() <= re)
            {
                return false;
            }
            true
        })
        .map(|m| (m.start(), m.end()))
        .collect();
    for (mut s, mut e) in curr_spans {
        s = trim_range_sign_prefix(step8_text, &step8_shadow, s, e);
        let raw = &step8_shadow[s..e];
        let trimmed = raw.trim();
        let lead = raw.len() - raw.trim_start().len();
        s += lead;
        e = s + trimmed.len();
        if !is_valid_non_exact_span(step8_text, s, e, trimmed) {
            continue;
        }
        push_unique(&mut set.currencies, &mut seen_curr, trimmed.to_string());
        mask_utf8_span(&mut step8_shadow, s, e);
    }

    // 9. Semver, IPv4, scientific notation, p-values (mask before dates/numbers)
    let mut special_spans: Vec<(usize, usize)> = Vec::new();
    let mut cursor = 0usize;
    while let Some(m) = semver_ip_sci_pval_re().find_at(&step8_shadow, cursor) {
        let slice = &step8_shadow[m.start()..m.end()];
        if slice.starts_with('p') {
            let next_is_dash_digit = step8_text[m.end()..]
                .strip_prefix(['-', '\u{2013}'])
                .and_then(|p| p.chars().next())
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false);
            if next_is_dash_digit {
                let num_offset = slice
                    .find(|c: char| c.is_ascii_digit() || c == '.')
                    .unwrap_or(1);
                cursor = m.start() + num_offset;
                continue;
            }
            special_spans.push((m.start(), m.end()));
            cursor = m.end();
            continue;
        }
        if let Some(dm) = date_re().find(slice)
            && dm.start() == 0
        {
            let next_is_dash_digit = step8_text[m.start() + dm.end()..]
                .strip_prefix(['-', '\u{2013}'])
                .and_then(|p| p.chars().next())
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false);
            if !next_is_dash_digit {
                let tail_offset = m.start() + dm.end();
                if let Some(sub) = semver_ip_sci_pval_re().find(&step8_shadow[tail_offset..m.end()]) {
                    special_spans.push((tail_offset + sub.start(), tail_offset + sub.end()));
                }
                cursor = m.end();
                continue;
            }
        }
        special_spans.push((m.start(), m.end()));
        cursor = m.end();
    }
    for (mut s, mut e) in special_spans {
        s = trim_range_sign_prefix(step8_text, &step8_shadow, s, e);
        if e < step8_text.len()
            && step8_text[e..]
                .chars()
                .next()
                .is_some_and(|nc| nc.is_alphanumeric() || nc == '_')
        {
            mask_utf8_span(&mut step8_shadow, s, e);
            continue;
        }
        let raw = &step8_shadow[s..e];
        let trimmed_slice = raw.trim();
        let lead = raw.len() - raw.trim_start().len();
        s += lead;
        e = s + trimmed_slice.len();
        let first_is_digit = trimmed_slice
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false);
        if s > 0
            && first_is_digit
            && let Some(pc) = step8_text[..s].chars().next_back()
            && matches!(pc, '.' | ',')
            && step8_text[..s - pc.len_utf8()]
                .chars()
                .next_back()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
        {
            continue;
        }
        let last_is_digit = trimmed_slice
            .chars()
            .next_back()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false);
        if e < step8_text.len()
            && last_is_digit
            && let Some(nc) = step8_text[e..].chars().next()
            && matches!(nc, '.' | ',')
            && step8_text[e + nc.len_utf8()..]
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
        {
            continue;
        }
        if !is_valid_non_exact_span(step8_text, s, e, trimmed_slice) {
            continue;
        }
        push_unique(&mut set.numbers, &mut seen_num, trimmed_slice.to_string());
        mask_utf8_span(&mut step8_shadow, s, e);
    }

    // 10. Dates (skip ISO dates immediately adjacent to -[0-9] or invalid non-exact boundaries so codes like 2024-11-21-01 fall through to numeric_range_re,
    // and skip unmasked "$-"/"$–" prefixes so malformed "$-2026-11-21" falls through to numeric_range_re without weakening "$-" boundary guards)
    let date_spans: Vec<(usize, usize)> = date_re()
        .find_iter(&step8_shadow)
        .filter(|m| {
            let prev_is_unmasked_dollar_dash = step8_shadow[..m.start()]
                .strip_suffix(['-', '\u{2013}'])
                .map(|p| p.ends_with('$'))
                .unwrap_or(false);
            !prev_is_unmasked_dollar_dash
                && is_valid_non_exact_span(step8_text, m.start(), m.end(), m.as_str().trim())
        })
        .map(|m| (m.start(), m.end()))
        .collect();
    for (mut s, mut e) in date_spans {
        let raw = &step8_shadow[s..e];
        let trimmed = raw.trim();
        let lead = raw.len() - raw.trim_start().len();
        s += lead;
        e = s + trimmed.len();
        if !is_valid_non_exact_span(step8_text, s, e, trimmed) {
            continue;
        }
        push_unique(&mut set.dates, &mut seen_date, trimmed.to_string());
        mask_utf8_span(&mut step8_shadow, s, e);
    }

    // 11. Technical hyphenated/dotted identifiers (e.g., UTF-8, COVID-19, IEEE-1547-2018)
    let tech_spans: Vec<(usize, usize)> = tech_ident_re()
        .find_iter(&step8_shadow)
        .map(|m| (m.start(), m.end()))
        .collect();
    for (s, e) in tech_spans {
        mask_utf8_span(&mut step8_shadow, s, e);
    }

    // 11.5 Numeric ranges (e.g., 2024-25, 10%-20%, 555-123-4567, -15%-5%, 0-0) kept intact so strict boundary checks pass
    let range_spans: Vec<(usize, usize)> = numeric_range_re()
        .find_iter(&step8_shadow)
        .filter(|m| {
            let prev_is_dash_digit = step8_text[..m.start()]
                .strip_suffix(['-', '\u{2013}'])
                .and_then(|p| p.chars().next_back())
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false);
            let next_is_dash_digit = step8_text[m.end()..]
                .strip_prefix(['-', '\u{2013}'])
                .and_then(|p| p.chars().next())
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false);
            let first_is_digit = step8_shadow[m.start()..m.end()]
                .trim()
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false);
            let prev_is_sep_digit = first_is_digit
                && step8_text[..m.start()]
                    .chars()
                    .next_back()
                    .filter(|&pc| matches!(pc, '.' | ','))
                    .and_then(|pc| step8_text[..m.start() - pc.len_utf8()].chars().next_back())
                    .map(|c| c.is_ascii_digit())
                    .unwrap_or(false);
            let last_is_digit = step8_shadow[m.start()..m.end()]
                .trim()
                .chars()
                .next_back()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false);
            let next_is_sep_digit = last_is_digit
                && step8_text[m.end()..]
                    .chars()
                    .next()
                    .filter(|&nc| matches!(nc, '.' | ','))
                    .and_then(|nc| step8_text[m.end() + nc.len_utf8()..].chars().next())
                    .map(|c| c.is_ascii_digit())
                    .unwrap_or(false);
            !prev_is_dash_digit
                && !next_is_dash_digit
                && !prev_is_sep_digit
                && !next_is_sep_digit
        })
        .map(|m| (m.start(), m.end()))
        .collect();
    for (mut s, mut e) in range_spans {
        s = trim_range_sign_prefix(step8_text, &step8_shadow, s, e);
        if e < step8_text.len()
            && step8_text[e..]
                .chars()
                .next()
                .is_some_and(|nc| nc.is_alphanumeric() || nc == '_')
        {
            mask_utf8_span(&mut step8_shadow, s, e);
            continue;
        }
        let raw = &step8_shadow[s..e];
        let trimmed = raw.trim();
        let lead = raw.len() - raw.trim_start().len();
        s += lead;
        e = s + trimmed.len();
        if !is_valid_non_exact_span(step8_text, s, e, trimmed) {
            continue;
        }
        push_unique(&mut set.numbers, &mut seen_num, trimmed.to_string());
        mask_utf8_span(&mut step8_shadow, s, e);
    }

    // 12. General numbers & percentages (handling dash ranges vs genuine negatives)
    let num_spans: Vec<(usize, usize)> = number_re()
        .find_iter(&step8_shadow)
        .map(|m| (m.start(), m.end()))
        .collect();
    for (mut s, mut e) in num_spans {
        s = trim_range_sign_prefix(step8_text, &step8_shadow, s, e);
        let raw = &step8_shadow[s..e];
        let trimmed = raw.trim();
        let lead = raw.len() - raw.trim_start().len();
        s += lead;
        e = s + trimmed.len();
        // Skip numbers glued onto an alphanumeric word in original text (e.g. "x1"),
        // range bounds immediately after "%" (e.g. "foo-1,000%-2,000%"),
        // masked dash prefixes failing left-boundary (e.g. "https://example.com/a-\n100"),
        // or orphaned decimal/comma fragments adjacent to digits in original text (e.g. "foo-1,000")
        if s > 0 {
            let prev_orig = step8_text[..s].chars().next_back();
            if let Some(pc) = prev_orig {
                if pc.is_alphanumeric() || pc == '_' {
                    continue;
                }
                if pc == '%' && step8_shadow[s..e].starts_with(['-', '\u{2013}']) {
                    continue;
                }
                if matches!(pc, '-' | '\u{2013}')
                    && !is_left_boundary_ok(step8_text, s, trimmed)
                {
                    continue;
                }
                if matches!(pc, '.' | ',')
                    && step8_text[..s - pc.len_utf8()]
                        .chars()
                        .next_back()
                        .map(|c| c.is_ascii_digit())
                        .unwrap_or(false)
                {
                    continue;
                }
            }
        }
        if e < step8_text.len() {
            let next_orig = step8_text[e..].chars().next();
            if let Some(nc) = next_orig {
                if nc.is_alphanumeric() || nc == '_' {
                    continue;
                }
                if matches!(nc, '.' | ',')
                    && step8_text[e + nc.len_utf8()..]
                        .chars()
                        .next()
                        .map(|c| c.is_ascii_digit())
                        .unwrap_or(false)
                {
                    continue;
                }
            }
        }
        if !is_valid_non_exact_span(step8_text, s, e, trimmed) {
            continue;
        }
        push_unique(&mut set.numbers, &mut seen_num, trimmed.to_string());
    }

    // 14. Advisory named entities
    for m in entity_re().find_iter(&step8_shadow) {
        let s = m.as_str().trim();
        if s.len() >= 4 {
            push_unique(&mut set.entities, &mut seen_ent, s.to_string());
        }
    }

    set
}

fn normalize_typography(s: &str) -> String {
    s.replace('\u{2013}', "-")
        .replace(['\u{201c}', '\u{201d}'], "\"")
        .replace(['\u{2018}', '\u{2019}'], "'")
}

fn collapse_whitespace_with_mode(
    s: &str,
    collapse_horizontal: bool,
    collapse_wrapped_hyphen: bool,
) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while i < s.len() {
        let rest = &s[i..];
        let Some(ch) = rest.chars().next() else {
            break;
        };
        if !ch.is_whitespace() {
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        let ws_start = i;
        let mut newlines = 0usize;
        while i < s.len() {
            let Some(wch) = s[i..].chars().next() else {
                break;
            };
            if !wch.is_whitespace() {
                break;
            }
            if wch == '\r' {
                newlines += 1;
                i += 1;
                if s[i..].starts_with('\n') {
                    i += 1;
                }
            } else if wch == '\n' {
                newlines += 1;
                i += 1;
            } else {
                i += wch.len_utf8();
            }
        }
        if newlines >= 2 {
            out.push_str("\n\n");
        } else if newlines == 1 {
            let wrapped_hyphen = collapse_wrapped_hyphen
                && out.ends_with('-')
                && out[..out.len() - 1]
                    .chars()
                    .next_back()
                    .map(|prev| !prev.is_whitespace() && prev != '-')
                    .unwrap_or(false);
            if !wrapped_hyphen {
                out.push(' ');
            }
        } else if collapse_horizontal {
            out.push(' ');
        } else {
            out.push_str(&s[ws_start..i]);
        }
    }
    out
}

fn normalize_line_wraps(s: &str) -> String {
    collapse_whitespace_with_mode(s, false, false)
}

fn collapse_inline_whitespace(s: &str) -> String {
    collapse_whitespace_with_mode(s, true, true)
}

fn starts_with_numeric_or_currency(needle: &str) -> bool {
    let Some(first) = needle.chars().next() else {
        return false;
    };
    if first.is_ascii_digit() || matches!(first, '$' | '€' | '£' | '¥' | '₹') {
        return true;
    }
    const SYM_PREFIXES: &[&str] = &["US$", "CA$", "AU$", "NZ$", "HK$", "SG$"];
    if SYM_PREFIXES.iter().any(|p| {
        needle
            .get(..p.len())
            .map(|head| head.eq_ignore_ascii_case(p))
            .unwrap_or(false)
    }) {
        return true;
    }
    const ISO_CODES: &[&str] = &[
        "USD", "EUR", "GBP", "JPY", "CHF", "CAD", "AUD", "INR", "CNY",
    ];
    ISO_CODES.iter().any(|code| {
        needle
            .get(..code.len())
            .map(|head| head.eq_ignore_ascii_case(code))
            .unwrap_or(false)
            && needle
                .get(code.len()..)
                .and_then(|rest| rest.chars().next())
                .map(|c| c.is_whitespace())
                .unwrap_or(false)
    })
}

fn is_emoji_prefix_char(c: char) -> bool {
    matches!(
        c as u32,
        0x2300..=0x23FF
            | 0x2600..=0x27BF
            | 0x2B00..=0x2BFF
            | 0xFE00..=0xFE0F
            | 0x200D
            | 0x20E3
            | 0x1F000..=0x1FAFF
    )
}

fn is_left_boundary_ok(haystack: &str, start: usize, needle: &str) -> bool {
    if start == 0 {
        return true;
    }
    let Some(prev) = haystack[..start].chars().next_back() else {
        return true;
    };
    let first = needle.chars().next();
    if prev.is_alphanumeric() || prev == '_' {
        return false;
    }
    if matches!(prev, '-' | '\u{2013}') && starts_with_numeric_or_currency(needle) {
        let before_str = &haystack[..start - prev.len_utf8()];
        let before_char = before_str.chars().next_back();
        let needle_allows_alpha_hyphen_prefix = !needle
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
            || needle
                .chars()
                .any(|c| matches!(c, '.' | '-' | '\u{2013}') || c.is_ascii_alphabetic());
        let preceded_by_alpha_ident =
            needle_allows_alpha_hyphen_prefix && trailing_alpha_ident_re().is_match(before_str);
        let preceded_by_latex_close = needle_allows_alpha_hyphen_prefix
            && before_char == Some('$')
            && scan_inline_latex_intervals(before_str)
                .last()
                .map(|&(_, e)| e == before_str.len())
                .unwrap_or(false);
        let preceded_by_emoji = needle_allows_alpha_hyphen_prefix
            && before_char.map(is_emoji_prefix_char).unwrap_or(false);
        let preceded_by_percent =
            needle_allows_alpha_hyphen_prefix && before_char == Some('%');
        let preceded_by_closing_quote = needle_allows_alpha_hyphen_prefix
            && matches!(before_char, Some('"' | '\'' | '\u{201d}' | '\u{2019}'))
            && before_char
                .and_then(|q| {
                    before_str[..before_str.len() - q.len_utf8()]
                        .chars()
                        .next_back()
                })
                .is_some_and(|c| {
                    c.is_alphanumeric() || matches!(c, ')' | ']' | '}' | '.' | '!' | '?')
                });
        let preceded_by_closing_delim = matches!(before_char, Some(')' | ']' | '}' | '`'))
            || preceded_by_latex_close
            || preceded_by_emoji
            || preceded_by_percent
            || preceded_by_closing_quote;
        if !preceded_by_alpha_ident && !preceded_by_closing_delim {
            return false;
        }
    }
    if first.map(|c| c.is_ascii_digit()).unwrap_or(false) && matches!(prev, '.' | ',') {
        let before_sep = haystack[..start - 1].chars().next_back();
        if before_sep.map(|c| c.is_ascii_digit()).unwrap_or(false) {
            return false;
        }
    }
    if first
        .map(|c| matches!(c, '$' | '€' | '£' | '¥' | '₹'))
        .unwrap_or(false)
        && prev.is_ascii_digit()
    {
        return false;
    }
    true
}

fn is_right_boundary_ok(haystack: &str, end: usize, needle: &str) -> bool {
    if end >= haystack.len() {
        return true;
    }
    let Some(next) = haystack[end..].chars().next() else {
        return true;
    };
    if next.is_alphanumeric() || next == '_' {
        return false;
    }
    let last = needle.chars().next_back();
    if last.map(|c| c.is_ascii_digit()).unwrap_or(false) {
        if matches!(next, '.' | ',') {
            let after_sep = haystack[end + 1..].chars().next();
            if after_sep.map(|c| c.is_ascii_digit()).unwrap_or(false) {
                return false;
            }
        }
        if matches!(next, '-' | '\u{2013}') {
            let after_dash = haystack[end + next.len_utf8()..].chars().next();
            if after_dash.map(|c| c.is_ascii_digit()).unwrap_or(false) {
                return false;
            }
        }
        if next == '%' {
            return false;
        }
    }
    if last == Some('%') && next.is_ascii_digit() {
        return false;
    }
    true
}

fn contains_with_boundary_single(needle: &str, haystack: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    for (idx, _) in haystack.match_indices(needle) {
        let end = idx + needle.len();
        if is_left_boundary_ok(haystack, idx, needle)
            && is_right_boundary_ok(haystack, end, needle)
        {
            return true;
        }
    }
    false
}

/// O(n) boundary-aware invariant checker (no regex lookarounds).
pub fn contains_with_boundary(needle: &str, haystack: &str, exact_mode: bool) -> bool {
    if needle.is_empty() {
        return true;
    }
    if exact_mode {
        if haystack.contains(needle) {
            return true;
        }
        let n_norm = normalize_typography(needle);
        let h_norm = normalize_typography(haystack);
        if h_norm.contains(&n_norm) {
            return true;
        }
        let n_wrap = normalize_line_wraps(&n_norm);
        let h_wrap = normalize_line_wraps(&h_norm);
        if h_wrap.contains(&n_wrap) {
            return true;
        }
        // Case-insensitive DOI / URL host tolerance
        if (needle.starts_with("10.") || needle.starts_with("http"))
            && h_wrap.to_ascii_lowercase().contains(&n_wrap.to_ascii_lowercase())
        {
            return true;
        }
        return false;
    }

    let n_norm = collapse_inline_whitespace(&normalize_typography(needle));
    let h_norm = collapse_inline_whitespace(&normalize_typography(haystack));
    if contains_with_boundary_single(&n_norm, &h_norm) {
        return true;
    }

    // Comma-stripped equivalence for numbers/currencies (e.g., 1,420 <-> 1420)
    if n_norm.contains(',') {
        let no_commas = n_norm.replace(',', "");
        if contains_with_boundary_single(&no_commas, &h_norm) {
            return true;
        }
    }
    if h_norm.contains(',') {
        let h_no_commas = h_norm.replace(',', "");
        let n_no_commas = n_norm.replace(',', "");
        if contains_with_boundary_single(&n_no_commas, &h_no_commas) {
            return true;
        }
    }

    // Case-insensitive fallback for dates / ISO currencies
    let n_lower = n_norm.to_ascii_lowercase();
    let h_lower = h_norm.to_ascii_lowercase();
    contains_with_boundary_single(&n_lower, &h_lower)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::VoiceOption;

    fn opts(preserve_citations: bool, preserve_quotes: bool) -> RewriteOptions {
        RewriteOptions {
            preserve_length: true,
            preserve_formatting: true,
            preserve_citations,
            preserve_quotes,
            language: "en".to_string(),
            voice: VoiceOption::Auto,
        }
    }

    #[test]
    fn extracts_all_invariant_categories_without_cross_leakage() {
        let text = concat!(
            "Visit https://en.wikipedia.org/wiki/Paraphrase_(computational_linguistics)?q=a&b=1#frag ",
            "or email ops@example.org regarding doi:10.1145/3580305.3599123. ",
            "Dr. Elena Vance (Smith et al., 2024, pp. 12–18) and [1, 2] found $1,245,000 ",
            "and EUR 380,500 on 2026-11-21T00:57:05Z (p < 0.001, v2.4.1, 1.5e-4, 192.168.1.1). ",
            "Run `cargo test` with $E=mc^2$ and avoid (-15) across 2024-25 or 10%-20% or $10-20. ",
            "She stated, \"Zero patient records left the local machine\" and \"visit https://example.com\"."
        );

        let inv = extract_invariants(text, &opts(true, true));
        assert!(inv.urls.iter().any(|u| u.contains("Paraphrase_(computational_linguistics)?q=a&b=1#frag")));
        assert!(inv.emails.contains(&"ops@example.org".to_string()));
        assert!(inv.dois.contains(&"10.1145/3580305.3599123".to_string()));
        assert!(inv.citations.iter().any(|c| c.contains("Smith et al., 2024")));
        assert!(inv.citations.iter().any(|c| c == "[1, 2]"));
        assert!(inv.currencies.contains(&"$1,245,000".to_string()));
        assert!(inv.currencies.contains(&"EUR 380,500".to_string()));
        assert!(inv.dates.contains(&"2026-11-21T00:57:05Z".to_string()));
        assert!(inv.code_blocks.contains(&"`cargo test`".to_string()));
        assert!(inv.latex_spans.contains(&"$E=mc^2$".to_string()));
        assert!(inv.numbers.contains(&"-15".to_string()));
        // Range bounds must NOT become negative -25 or -20
        assert!(!inv.numbers.contains(&"-25".to_string()));
        assert!(inv.numbers.contains(&"2024-25".to_string()));
        assert!(inv.numbers.contains(&"10%-20%".to_string()));
        assert!(inv.currencies.contains(&"$10-20".to_string()));
        assert!(!inv.numbers.contains(&"-20".to_string()));
        assert!(!inv.numbers.contains(&"-11".to_string()));
        assert!(!inv.numbers.contains(&"-21".to_string()));
        assert!(inv.quoted_strings.contains(&"\"Zero patient records left the local machine\"".to_string()));
    }

    #[test]
    fn boundary_checker_avoids_partial_number_matches() {
        assert!(!contains_with_boundary("180", "Value is 1.180 and 1800.", false));
        assert!(contains_with_boundary("180", "Value is 180 units.", false));
        assert!(contains_with_boundary("1,420", "Total reached 1420 units.", false));
        assert!(!contains_with_boundary("-15", "Value was 15.", false));
        assert!(contains_with_boundary("-15", "Temp dropped to (-15) C.", false));
    }

    #[test]
    fn quote_and_citation_flags_toggle_enforcement() {
        let text = "Smith (2024) wrote \"Exact phrase here.\" with 42 samples.";
        let inv = extract_invariants(text, &opts(false, false));
        let enforced = inv.enforced_items(&opts(false, false));
        assert!(!enforced.iter().any(|(k, _, _)| *k == "citation" || *k == "quote"));
        assert!(enforced.iter().any(|(k, v, _)| *k == "number" && *v == "42"));
    }

    #[test]
    fn bare_semver_extracted_intact_without_splitting() {
        let text = "Applications built with Tauri 2.12.0 and Python 3.12.4 run locally across 5 nodes.";
        let inv = extract_invariants(text, &opts(true, true));
        assert!(inv.numbers.contains(&"2.12.0".to_string()));
        assert!(inv.numbers.contains(&"3.12.4".to_string()));
        assert!(inv.numbers.contains(&"5".to_string()));
        assert!(!inv.numbers.contains(&"2.12".to_string()));
        assert!(!inv.numbers.contains(&"0".to_string()));
        assert!(!inv.numbers.contains(&"3.12".to_string()));
        assert!(!inv.numbers.contains(&"4".to_string()));
    }

    #[test]
    fn parenthetical_prose_not_captured_as_apa_citation_and_preserves_numbers() {
        let text = "Performance improved (The system launched in 2024 and processed 95% of requests) across all clusters, matching (Vance et al., 2024).";
        for preserve_cites in [true, false] {
            let inv = extract_invariants(text, &opts(preserve_cites, true));
            assert!(
                !inv.citations.iter().any(|c| c.contains("launched in 2024")),
                "prose parenthetical wrongly matched as APA citation: {:?}",
                inv.citations
            );
            assert!(inv.citations.contains(&"(Vance et al., 2024)".to_string()));
            assert!(inv.numbers.contains(&"2024".to_string()));
            assert!(inv.numbers.contains(&"95%".to_string()));
        }
    }

    #[test]
    fn numeric_boundary_rejects_sign_flip_iso_date_prefix_and_percent_suffix() {
        assert!(!contains_with_boundary("15", "Temperature is -15 C.", false));
        assert!(!contains_with_boundary("2026", "Updated on 2026-11-21.", false));
        assert!(!contains_with_boundary("10", "Growth was 10%.", false));
        assert!(contains_with_boundary("10%", "Growth was 10%.", false));
        assert!(contains_with_boundary("2026", "In 2026—specifically Q4—load grew.", false));
        assert!(contains_with_boundary("2.12.0", "Built with Tauri 2.12.0 locally.", false));
    }

    #[test]
    fn currency_addition_not_paired_as_latex_and_curly_quoted_urls_trimmed() {
        let text = "Total cost is $10+$20 plus $E=mc^2$, documented at “https://example.org/spec?v=1&q=(test)” and “10.1145/3580305.3599123”.";
        let inv = extract_invariants(text, &opts(true, true));
        assert_eq!(inv.latex_spans, vec!["$E=mc^2$".to_string()]);
        assert!(inv.currencies.contains(&"$10".to_string()));
        assert!(inv.currencies.contains(&"$20".to_string()));
        assert_eq!(
            inv.urls,
            vec!["https://example.org/spec?v=1&q=(test)".to_string()]
        );
        assert_eq!(
            inv.dois,
            vec!["10.1145/3580305.3599123".to_string()]
        );
    }

    #[test]
    fn negative_currency_and_currency_ranges_preserve_verbatim_and_catch_sign_flips() {
        let text = concat!(
            "Net loss was -$10, -€50, -US$10, -USD 100, -10 USD, and \"-$10\" and \"-15\" ",
            "across $10-$20, $10–$20, $10M-$20M, -$10–$20, -$10-$20, USD 100-200, and 100 USD-200 USD ranges, ",
            "plus cost-$10 and sub-$100."
        );
        let inv = extract_invariants(text, &opts(true, true));
        for expected in [
            "-$10",
            "-€50",
            "-US$10",
            "-USD 100",
            "-10 USD",
            "$10-$20",
            "$10–$20",
            "$10M-$20M",
            "-$10–$20",
            "-$10-$20",
            "USD 100-200",
            "100 USD-200 USD",
            "$10",
            "$100",
        ] {
            assert!(
                inv.currencies.contains(&expected.to_string()),
                "missing expected currency invariant {expected:?} in {:?}",
                inv.currencies
            );
        }
        assert!(inv.numbers.contains(&"-15".to_string()));
        for (cat, val, exact) in inv.enforced_items(&opts(true, true)) {
            assert!(
                contains_with_boundary(val, text, exact),
                "verbatim check failed for {cat}:{val}"
            );
        }

        // Sign-flip checks (including ISO & prefixed currencies S1/S2)
        assert!(contains_with_boundary("-$10", "Net loss was -$10 in Q4.", false));
        assert!(!contains_with_boundary("-$10", "Net gain was $10 in Q4.", false));
        assert!(!contains_with_boundary("$10", "Net loss was -$10 in Q4.", false));
        assert!(!contains_with_boundary("USD 100", "Balance was -USD 100.", false));
        assert!(!contains_with_boundary("US$10", "Balance was -US$10.", false));
        assert!(contains_with_boundary("$10-$20", "Range is $10–$20.", false));

        // Ensure $10+$20+$x$+$E=mc^2$ still preserves LaTeX & currency arithmetic
        let math_text = "Total is $10+$20=$30 and $10+$x$ plus $E=mc^2$.";
        let math_inv = extract_invariants(math_text, &opts(true, true));
        assert!(math_inv.latex_spans.contains(&"$E=mc^2$".to_string()));
        assert!(math_inv.currencies.contains(&"$10".to_string()));
        assert!(math_inv.currencies.contains(&"$20".to_string()));
        assert!(math_inv.currencies.contains(&"$30".to_string()));
    }

    #[test]
    fn hyphen_prefixed_identifiers_and_delimiters_pass_without_weakening_sign_checks() {
        let text = concat!(
            "Upgrade package tauri-2.12.0, ec2-2026-11-21, backup-2026-11-21, ",
            "metric foo-1.5e-4, and compare `foo`-5 and (2024)-5 today."
        );
        let inv = extract_invariants(text, &opts(true, true));
        assert!(inv.numbers.contains(&"2.12.0".to_string()));
        assert!(inv.dates.contains(&"2026-11-21".to_string()));
        assert!(inv.numbers.contains(&"1.5e-4".to_string()));
        assert!(inv.numbers.contains(&"5".to_string()));
        for (cat, val, exact) in inv.enforced_items(&opts(true, true)) {
            assert!(
                contains_with_boundary(val, text, exact),
                "verbatim check failed for {cat}:{val}"
            );
        }

        assert!(!contains_with_boundary("15", "Temperature is -15 C.", false));
        assert!(!contains_with_boundary("15", "Temp is (-15) C.", false));
        assert!(!contains_with_boundary("-15", "Temperature is 15 C.", false));
        assert!(!contains_with_boundary("11-21", "Updated on 2026-11-21.", false));
        assert!(!contains_with_boundary("2026", "Updated on 2026-11-21.", false));
        assert!(!contains_with_boundary("19", "Variant COVID-19 spread.", false));
        assert!(!contains_with_boundary("1.5e-4", "Value was -1.5e-4.", false));
    }

    #[test]
    fn multi_segment_ranges_calver_oids_sci_and_iso_timestamps_extract_intact() {
        let text = concat!(
            "Call 555-123-4567, ref 1-2-3, ISBN 978-3-16-148410-0 and 0-306-40615-2. ",
            "Change was -15%-5% and rates 10%-20%-30%, error -1.5e-4. ",
            "Release 2024.10.1.2, OID 1.3.6.1.4, and range 192.168.1.1-192.168.1.10 shipped. ",
            "Timestamp 2024-01-15T10:30:00.123Z, window 2024-11-21-2024-11-22, and code 2024-11-21-01 logged."
        );
        let inv = extract_invariants(text, &opts(true, true));
        for expected_num in [
            "555-123-4567",
            "1-2-3",
            "978-3-16-148410-0",
            "0-306-40615-2",
            "-15%-5%",
            "10%-20%-30%",
            "-1.5e-4",
            "2024.10.1.2",
            "1.3.6.1.4",
            "192.168.1.1-192.168.1.10",
            "2024-11-21-01",
        ] {
            assert!(
                inv.numbers.contains(&expected_num.to_string()),
                "missing expected number {expected_num:?} in {:?}",
                inv.numbers
            );
        }
        assert!(!inv.numbers.contains(&"555".to_string()));
        assert!(!inv.numbers.contains(&"2024.10.1".to_string()));
        assert!(!inv.numbers.contains(&"1.3.6.1".to_string()));
        assert!(inv.dates.contains(&"2024-01-15T10:30:00.123Z".to_string()));
        assert!(inv.dates.contains(&"2024-11-21-2024-11-22".to_string()));
        assert!(!inv.dates.contains(&"2024-11-21".to_string()));

        for (cat, val, exact) in inv.enforced_items(&opts(true, true)) {
            assert!(
                contains_with_boundary(val, text, exact),
                "verbatim check failed for {cat}:{val}"
            );
        }

        assert!(!contains_with_boundary("1-2-3", "Ref 1-2-3-4.", false));
        assert!(!contains_with_boundary("1-2-3", "Ref 0-1-2-3.", false));
    }

    #[test]
    fn apa_citations_with_nobiliary_particles_are_extracted() {
        let text = "Results held (de la Cruz, 2024, p. 12) and (van der Waals, 1973) and (Vance, 2024).";
        let inv = extract_invariants(text, &opts(true, true));
        assert!(inv.citations.contains(&"(de la Cruz, 2024, p. 12)".to_string()));
        assert!(inv.citations.contains(&"(van der Waals, 1973)".to_string()));
        assert!(inv.citations.contains(&"(Vance, 2024)".to_string()));
    }

    #[test]
    fn r4_unicode_tech_idents_multipart_currencies_sci_ranges_and_emdash_normalization() {
        // 1. Non-ASCII letters/CJK before -<digits> pass verbatim while semver/dates still extract and standalone digits stay rejected
        let unicode_cases = [
            "Submit résumé-15 by Friday.",
            "Section 字-15 applies.",
            "Upgrade über-2.12.0 and naïve-2024-11-21 today.",
        ];
        for text in unicode_cases {
            let inv = extract_invariants(text, &opts(true, true));
            for (cat, val, exact) in inv.enforced_items(&opts(true, true)) {
                assert!(
                    contains_with_boundary(val, text, exact),
                    "verbatim check failed for {cat}:{val} in {text:?}"
                );
            }
        }
        let inv_uber = extract_invariants("Upgrade über-2.12.0 and naïve-2024-11-21 today.", &opts(true, true));
        assert!(inv_uber.numbers.contains(&"2.12.0".to_string()));
        assert!(inv_uber.dates.contains(&"2024-11-21".to_string()));

        assert!(!contains_with_boundary("15", "Submit résumé-15 by Friday.", false));
        assert!(!contains_with_boundary("15", "Section 字-15 applies.", false));
        assert!(!contains_with_boundary("19", "Variant COVID-19 spread.", false));

        // 2. 3+ part symbol currency ranges
        let tier_text = "Tiers $10-$20-$30 apply.";
        let inv_tier = extract_invariants(tier_text, &opts(true, true));
        assert!(inv_tier.currencies.contains(&"$10-$20-$30".to_string()));
        for (cat, val, exact) in inv_tier.enforced_items(&opts(true, true)) {
            assert!(contains_with_boundary(val, tier_text, exact), "failed {cat}:{val}");
        }

        // 3. Scientific-notation ranges, p < 0.05-1, and trim_range_sign_prefix adjacent to masked spans
        let sci_text = "Range 1.5e-4-2.5e-4 holds.";
        let inv_sci = extract_invariants(sci_text, &opts(true, true));
        assert!(inv_sci.numbers.contains(&"1.5e-4-2.5e-4".to_string()));
        for (cat, val, exact) in inv_sci.enforced_items(&opts(true, true)) {
            assert!(contains_with_boundary(val, sci_text, exact), "failed {cat}:{val}");
        }

        for text in [
            "Test p < 0.05-1 holds.",
            "Formula $x$-5 applies.",
            "Contact a@b.com-5 now.",
            "Price 10 USD-5 applies.",
            "Compare (-15), $10+$20, (2024)-5, and `foo`-5.",
        ] {
            let inv = extract_invariants(text, &opts(true, true));
            for (cat, val, exact) in inv.enforced_items(&opts(true, true)) {
                assert!(
                    contains_with_boundary(val, text, exact),
                    "verbatim check failed for {cat}:{val} in {text:?}"
                );
            }
        }
        let inv_latex_minus = extract_invariants("Formula $x$-5 applies.", &opts(true, true));
        assert!(inv_latex_minus.numbers.contains(&"-5".to_string()));

        // 4. Em-dash normalization preserves em-dash boundary while keeping en-dash range equivalence
        assert!(contains_with_boundary("1,420", "Cost was —1420.", false));
        assert!(contains_with_boundary("2024-11-21T10:30:00Z", "—2024-11-21t10:30:00z.", false));
        assert!(contains_with_boundary("$10-$20", "Range is $10–$20.", false));

        // 5. All R1/R2/R3 negative checks remain intact
        assert!(!contains_with_boundary("15", "Temp is -15 C.", false));
        assert!(!contains_with_boundary("15", "Temp is (-15) C.", false));
        assert!(!contains_with_boundary("2026", "Date 2026-11-21.", false));
        assert!(!contains_with_boundary("10", "Rate 10%.", false));
        assert!(!contains_with_boundary("USD 100", "Balance was -USD 100.", false));
        assert!(!contains_with_boundary("US$10", "Balance was -US$10.", false));
        assert!(!contains_with_boundary("$10", "Net loss was -$10.", false));
    }

    #[test]
    fn r5_suffix_iso_currency_ranges_nfd_marks_mixed_sci_ranges_and_latex_dates() {
        // 1. Suffix-only ISO currency ranges
        let usd_range = "Budget 100-200 USD applies.";
        let inv_usd = extract_invariants(usd_range, &opts(true, true));
        assert_eq!(inv_usd.currencies, vec!["100-200 USD".to_string()]);
        assert!(inv_usd.numbers.is_empty());
        assert!(contains_with_boundary("100-200 USD", usd_range, false));

        let eur_range = "Budget 1,000-2,000 EUR applies.";
        let inv_eur = extract_invariants(eur_range, &opts(true, true));
        assert_eq!(inv_eur.currencies, vec!["1,000-2,000 EUR".to_string()]);
        assert!(inv_eur.numbers.is_empty());
        assert!(contains_with_boundary("1,000-2,000 EUR", eur_range, false));

        // Guard: "2024-11-21 USD" must extract date cleanly without swallowing as currency range
        let date_usd = "2024-11-21 USD";
        let inv_date_usd = extract_invariants(date_usd, &opts(true, true));
        assert_eq!(inv_date_usd.dates, vec!["2024-11-21".to_string()]);
        assert!(inv_date_usd.currencies.is_empty());
        assert!(inv_date_usd.numbers.is_empty());
        assert!(contains_with_boundary("2024-11-21", date_usd, false));

        let date_usd_amount = "Date 2024-11-21 USD 100 applies.";
        let inv_date_usd_amt = extract_invariants(date_usd_amount, &opts(true, true));
        assert_eq!(inv_date_usd_amt.dates, vec!["2024-11-21".to_string()]);
        assert_eq!(inv_date_usd_amt.currencies, vec!["USD 100".to_string()]);

        let date_code_usd = "2024-11-21-01 USD";
        let inv_date_code_usd = extract_invariants(date_code_usd, &opts(true, true));
        assert!(inv_date_code_usd.currencies.is_empty());
        assert_eq!(inv_date_code_usd.numbers, vec!["2024-11-21-01".to_string()]);

        // "Price 10 USD-5 applies." must still extract only "10 USD"
        let price_dash = "Price 10 USD-5 applies.";
        let inv_price_dash = extract_invariants(price_dash, &opts(true, true));
        assert_eq!(inv_price_dash.currencies, vec!["10 USD".to_string()]);
        assert!(inv_price_dash.numbers.is_empty());

        // 2. NFD combining marks on final letter before -{semver|date}
        let nfd_semver = "Upgrade re\u{0301}sume\u{0301}-2.12.0 today.";
        let inv_nfd_sv = extract_invariants(nfd_semver, &opts(true, true));
        assert_eq!(inv_nfd_sv.numbers, vec!["2.12.0".to_string()]);
        assert!(contains_with_boundary("2.12.0", nfd_semver, false));

        let nfd_date = "Backup re\u{0301}sume\u{0301}-2024-11-21 today.";
        let inv_nfd_dt = extract_invariants(nfd_date, &opts(true, true));
        assert_eq!(inv_nfd_dt.dates, vec!["2024-11-21".to_string()]);
        assert!(contains_with_boundary("2024-11-21", nfd_date, false));

        let nfd_sci = "Ref re\u{0301}sume\u{0301}-1.5e-4 holds.";
        let inv_nfd_sci = extract_invariants(nfd_sci, &opts(true, true));
        assert_eq!(inv_nfd_sci.numbers, vec!["1.5e-4".to_string()]);
        assert!(contains_with_boundary("1.5e-4", nfd_sci, false));

        // 3. Mixed scientific-notation + plain numeric range
        let sci_float = "Range 1.5e-4-2.5 holds.";
        let inv_sci_float = extract_invariants(sci_float, &opts(true, true));
        assert_eq!(inv_sci_float.numbers, vec!["1.5e-4-2.5".to_string()]);
        assert!(contains_with_boundary("1.5e-4-2.5", sci_float, false));

        let sci_int = "Range 1.5e-4-10 holds.";
        let inv_sci_int = extract_invariants(sci_int, &opts(true, true));
        assert_eq!(inv_sci_int.numbers, vec!["1.5e-4-10".to_string()]);
        assert!(contains_with_boundary("1.5e-4-10", sci_int, false));

        // 4. Inline LaTeX closing '$' before -{date}
        let latex_date = "Formula $x$-2026-11-21 applies.";
        let inv_latex_dt = extract_invariants(latex_date, &opts(true, true));
        assert_eq!(inv_latex_dt.latex_spans, vec!["$x$".to_string()]);
        assert_eq!(inv_latex_dt.dates, vec!["2026-11-21".to_string()]);
        assert!(contains_with_boundary("2026-11-21", latex_date, false));

        // Negative boundary checks remain strict
        assert!(!contains_with_boundary("15", "$-15", false));
        assert!(!contains_with_boundary("15", "Cost $-15", false));
        assert!(!contains_with_boundary("2026-11-21", "$-2026-11-21", false));
        assert!(!contains_with_boundary("1.5e-4", "Value was -1.5e-4.", false));
        assert!(!contains_with_boundary("15", "Submit re\u{0301}sume\u{0301}-15.", false));
        assert!(!contains_with_boundary("2026", "2026-11-21 USD", false));
        assert!(!contains_with_boundary("19", "COVID-19", false));
    }

    #[test]
    fn r6_date_currency_ranges_sci_pct_comma_endash_newline_latex_and_emoji_dates() {
        // 1. Date-digit prefix guard on currency_range_re() + currency_re()
        for text in [
            "Code 2024-11-21-01-02 USD applies.",
            "2024-11-21-100-200 USD applies.",
            "Code 2024-11-21–01–02 USD applies.",
        ] {
            let inv = extract_invariants(text, &opts(true, true));
            assert!(
                inv.currencies.is_empty(),
                "unexpected currency in {text:?}: {:?}",
                inv.currencies
            );
            assert!(
                inv.dates.is_empty(),
                "unexpected partial date in {text:?}: {:?}",
                inv.dates
            );
            assert_eq!(
                inv.numbers.len(),
                1,
                "expected single code in {text:?}: {:?}",
                inv.numbers
            );
            for (cat, val, exact) in inv.enforced_items(&opts(true, true)) {
                assert!(
                    contains_with_boundary(val, text, exact),
                    "verbatim check failed for {cat}:{val} in {text:?}"
                );
            }
        }

        // Positive currency & date companions remain intact
        for (text, exp_curr, exp_date) in [
            ("Budget 100-200 USD applies.", Some("100-200 USD"), None),
            ("Budget 1,000-2,000 EUR applies.", Some("1,000-2,000 EUR"), None),
            ("2024-11-21 USD", None, Some("2024-11-21")),
            ("Date 2024-11-21 USD 100 applies.", Some("USD 100"), Some("2024-11-21")),
            ("2024-11-21-2024-11-22 USD", None, Some("2024-11-21-2024-11-22")),
        ] {
            let inv = extract_invariants(text, &opts(true, true));
            if let Some(c) = exp_curr {
                assert!(
                    inv.currencies.contains(&c.to_string()),
                    "{text:?} -> {:?}",
                    inv.currencies
                );
            } else {
                assert!(inv.currencies.is_empty(), "{text:?} -> {:?}", inv.currencies);
            }
            if let Some(d) = exp_date {
                assert!(inv.dates.contains(&d.to_string()), "{text:?} -> {:?}", inv.dates);
            }
        }

        // 2. Scientific-notation ranges with trailing % and plain-first-to-sci ranges
        for (text, expected) in [
            ("Range 1.5e-4-2.5% holds.", "1.5e-4-2.5%"),
            ("Range 1.5e-4-2.5e-4% holds.", "1.5e-4-2.5e-4%"),
            ("Range 2.5-1.5e-4 holds.", "2.5-1.5e-4"),
            ("Range 1-1.5e4 holds.", "1-1.5e4"),
        ] {
            let inv = extract_invariants(text, &opts(true, true));
            assert_eq!(inv.numbers, vec![expected.to_string()], "{text:?}");
            assert!(contains_with_boundary(expected, text, false), "{text:?}");
        }
        assert!(!contains_with_boundary("1.5e-4-2.5", "Range 1.5e-4-2.5% holds.", false));
        assert!(!contains_with_boundary("10", "Growth was 10%.", false));
        let date_then_sci = "On 2024-11-21 - 1.5e-4 was observed.";
        let inv_dts = extract_invariants(date_then_sci, &opts(true, true));
        assert_eq!(inv_dts.dates, vec!["2024-11-21".to_string()]);
        assert_eq!(inv_dts.numbers, vec!["1.5e-4".to_string()]);

        // 3. Comma-inflation numeric boundary checks
        assert!(!contains_with_boundary("100", "The cost is 100,000.", false));
        assert!(!contains_with_boundary("000", "The cost is 100,000.", false));
        assert!(!contains_with_boundary("USD 100", "The cost is USD 100,000.", false));
        assert!(!contains_with_boundary("1,000", "The cost is 1,000,000.", false));
        assert!(contains_with_boundary("100", "Values are 100, 200, 300.", false));
        assert!(contains_with_boundary("200", "Values are 100, 200, 300.", false));
        assert!(contains_with_boundary("1,000", "The cost is 1,000.", false));
        assert!(contains_with_boundary("1,420", "Total reached 1420 units.", false));
        assert!(contains_with_boundary("1,000-2,000 EUR", "Budget 1,000-2,000 EUR applies.", false));

        // 4. En-dash boundary checks on raw haystack pass
        assert!(!contains_with_boundary("15", "Temp is –15 C.", false));
        assert!(!contains_with_boundary("15", "Submit résumé–15.", false));
        assert!(!contains_with_boundary("10", "Range 10–20.", false));
        assert!(!contains_with_boundary("20", "Range 10–20.", false));
        assert!(contains_with_boundary("10-20", "Range 10–20.", false));
        assert!(contains_with_boundary("100-200 USD", "Budget 100–200 USD applies.", false));

        for text in ["Submit résumé–15.", "Temp is –15 C.", "Range 10–20."] {
            let inv = extract_invariants(text, &opts(true, true));
            for (cat, val, exact) in inv.enforced_items(&opts(true, true)) {
                assert!(
                    contains_with_boundary(val, text, exact),
                    "verbatim check failed for {cat}:{val} in {text:?}"
                );
            }
        }
        assert!(extract_invariants("Submit résumé–15.", &opts(true, true)).numbers.is_empty());
        assert_eq!(
            extract_invariants("Range 10–20.", &opts(true, true)).numbers,
            vec!["10–20".to_string()]
        );

        // 5. Currency whitespace does not capture newlines
        let wrapped_curr = "Budget 100-200\nUSD applies.";
        let inv_wrapped = extract_invariants(wrapped_curr, &opts(true, true));
        assert!(inv_wrapped.currencies.is_empty(), "{:?}", inv_wrapped.currencies);
        assert_eq!(inv_wrapped.numbers, vec!["100-200".to_string()]);
        assert!(contains_with_boundary("100-200", "Budget 100-200 USD applies.", false));

        // 6. Inline LaTeX without operators ($f(x)$, $a-b$) & emoji-prefixed dates
        let fn_latex = "Formula $f(x)$-2026-11-21 applies.";
        let inv_fn = extract_invariants(fn_latex, &opts(true, true));
        assert_eq!(inv_fn.latex_spans, vec!["$f(x)$".to_string()]);
        assert_eq!(inv_fn.dates, vec!["2026-11-21".to_string()]);
        assert!(contains_with_boundary("2026-11-21", fn_latex, false));

        let minus_latex = "Formula $a-b$-2026-11-21 applies.";
        let inv_minus = extract_invariants(minus_latex, &opts(true, true));
        assert_eq!(inv_minus.latex_spans, vec!["$a-b$".to_string()]);
        assert_eq!(inv_minus.dates, vec!["2026-11-21".to_string()]);
        assert!(contains_with_boundary("2026-11-21", minus_latex, false));

        // Currency pairs must NOT be misclassified as inline LaTeX
        let pair_curr = extract_invariants("Cost $10 and $20.", &opts(true, true));
        assert!(pair_curr.latex_spans.is_empty());
        assert_eq!(pair_curr.currencies, vec!["$10".to_string(), "$20".to_string()]);

        let tier_curr = extract_invariants("Tiers $10-$20-$30 apply.", &opts(true, true));
        assert!(tier_curr.latex_spans.is_empty());
        assert_eq!(tier_curr.currencies, vec!["$10-$20-$30".to_string()]);

        let add_curr = extract_invariants("Total is $10+$20.", &opts(true, true));
        assert!(add_curr.latex_spans.is_empty());
        assert_eq!(add_curr.currencies, vec!["$10".to_string(), "$20".to_string()]);

        // Emoji-prefixed date vs negative number
        let emoji_dt = "Backup 🚀-2024-11-21 today.";
        let inv_emoji = extract_invariants(emoji_dt, &opts(true, true));
        assert_eq!(inv_emoji.dates, vec!["2024-11-21".to_string()]);
        assert!(contains_with_boundary("2024-11-21", emoji_dt, false));

        let emoji_num = "Status 🚀-15 applies.";
        let inv_emoji_num = extract_invariants(emoji_num, &opts(true, true));
        assert_eq!(inv_emoji_num.numbers, vec!["-15".to_string()]);
        assert!(!contains_with_boundary("15", emoji_num, false));
        assert!(!contains_with_boundary("2026-11-21", "$-2026-11-21", false));
        assert!(!contains_with_boundary("15", "$-15", false));
    }

    #[test]
    fn r7_endash_comma_sci_pval_years_linewrap_latex_and_dollar_dash_guards() {
        // 1. En-dash negative currencies & currency ranges
        for (text, exp_curr) in [
            ("Cost was –$10.", "–$10"),
            ("Balance was –USD 100.", "–USD 100"),
            ("Balance was USD –100.", "USD –100"),
            ("Cost was –10 USD.", "–10 USD"),
            ("Balance was –US$10.", "–US$10"),
            ("Budget –100–200 USD applies.", "–100–200 USD"),
            ("Budget –1,000–2,000 EUR applies.", "–1,000–2,000 EUR"),
            ("Budget –USD 100–200 applies.", "–USD 100–200"),
            ("Budget USD –100–200 applies.", "USD –100–200"),
            ("Budget –$10–$20 applies.", "–$10–$20"),
        ] {
            let inv = extract_invariants(text, &opts(true, true));
            assert_eq!(inv.currencies, vec![exp_curr.to_string()], "{text:?}");
            assert!(inv.numbers.is_empty(), "{text:?} -> {:?}", inv.numbers);
            for (cat, val, exact) in inv.enforced_items(&opts(true, true)) {
                assert!(contains_with_boundary(val, text, exact), "failed {cat}:{val} in {text:?}");
            }
        }
        // Bidirectional hyphen <-> en-dash currency equivalence
        assert!(contains_with_boundary("–$10", "Cost was -$10.", false));
        assert!(contains_with_boundary("-$10", "Cost was –$10.", false));
        assert!(contains_with_boundary("–100–200 USD", "Budget -100-200 USD applies.", false));
        assert!(contains_with_boundary("-100-200 USD", "Budget –100–200 USD applies.", false));
        assert!(contains_with_boundary("USD –100", "Balance was USD -100.", false));

        // En-dash date-continuation & identifier protections stay intact
        for (text, exp_num) in [
            ("Code 2024-11-21–01–02 USD applies.", "2024-11-21–01–02"),
            ("2024-11-21–100–200 USD applies.", "2024-11-21–100–200"),
            ("2024-11-21–01 USD", "2024-11-21–01"),
            ("Temp is –15 C.", "–15"),
        ] {
            let inv = extract_invariants(text, &opts(true, true));
            assert!(inv.currencies.is_empty(), "{text:?} -> {:?}", inv.currencies);
            assert!(inv.dates.is_empty(), "{text:?} -> {:?}", inv.dates);
            assert_eq!(inv.numbers, vec![exp_num.to_string()], "{text:?}");
            assert!(contains_with_boundary(exp_num, text, false), "{text:?}");
        }

        // 2 & 3. En-dash negative numeric/sci/IP/semver/CalVer ranges & p < 0.05 prefix
        for (text, exp_num) in [
            ("Temp –10%–20% holds.", "–10%–20%"),
            ("Temp –10–20 C.", "–10–20"),
            ("Range –1,000–2,000 holds.", "–1,000–2,000"),
            ("Value was –1.5e-4.", "–1.5e-4"),
            ("Range –1.5e-4–2.5e-4 holds.", "–1.5e-4–2.5e-4"),
            ("Range –1–1.5e4 holds.", "–1–1.5e4"),
            ("Range 192.168.1.1–192.168.1.10 shipped.", "192.168.1.1–192.168.1.10"),
            ("Versions v2.1.0–2.2.0 shipped.", "v2.1.0–2.2.0"),
            ("Releases 2024.10.1.2–2024.10.1.3 shipped.", "2024.10.1.2–2024.10.1.3"),
            ("Test p < 0.05-1.5e-4 holds.", "0.05-1.5e-4"),
            ("Test p < 0.05–1.5e-4 holds.", "0.05–1.5e-4"),
            ("Test p < 0.05-2.12.0 holds.", "0.05-2.12.0"),
            ("Test p < 0.05-1 holds.", "0.05-1"),
            ("Test p < 0.05–1 holds.", "0.05–1"),
            ("Test p < 0.05 holds.", "p < 0.05"),
        ] {
            let inv = extract_invariants(text, &opts(true, true));
            assert_eq!(inv.numbers, vec![exp_num.to_string()], "{text:?}");
            assert!(contains_with_boundary(exp_num, text, false), "{text:?}");
        }

        // 3b & 5. Comma-grouped scientific notation & tech-ident orphaned ",000" guard
        for (text, exp_num) in [
            ("Value 1,000e5 holds.", "1,000e5"),
            ("Range 1,000-1.5e4 holds.", "1,000-1.5e4"),
            ("Range 1.5e-4-1,000 holds.", "1.5e-4-1,000"),
            ("Range 1,000%-1.5e4% holds.", "1,000%-1.5e4%"),
            ("Range 1,000,000-2,000,000e-3% holds.", "1,000,000-2,000,000e-3%"),
            ("Value –1,000e5 holds.", "–1,000e5"),
            ("Range 1.5e-4-2.5% holds.", "1.5e-4-2.5%"),
            ("Range 2.5-1.5e-4 holds.", "2.5-1.5e-4"),
        ] {
            let inv = extract_invariants(text, &opts(true, true));
            assert_eq!(inv.numbers, vec![exp_num.to_string()], "{text:?}");
            assert!(contains_with_boundary(exp_num, text, false), "{text:?}");
        }
        assert!(contains_with_boundary("1,000e5", "Value 1000e5 holds.", false));
        assert!(contains_with_boundary("1,000-1.5e4", "Range 1000-1.5e4 holds.", false));
        assert!(!contains_with_boundary("100", "The cost is 100,000.", false));
        assert!(!contains_with_boundary("000", "The cost is 100,000.", false));
        assert!(!contains_with_boundary("1,000", "The cost is 100,000.", false));
        assert!(contains_with_boundary("100", "Values are 100, 200, 300.", false));
        assert!(!contains_with_boundary("1,000,000-2,000,000e-3", "Range 1,000,000-2,000,000e-3% holds.", false));
        let dts = extract_invariants("On 2024-11-21 - 1.5e-4 was observed.", &opts(true, true));
        assert_eq!(dts.dates, vec!["2024-11-21".to_string()]);
        assert_eq!(dts.numbers, vec!["1.5e-4".to_string()]);

        for text in ["Submit foo-1,000.", "Submit foo–1,000."] {
            let inv = extract_invariants(text, &opts(true, true));
            assert!(inv.numbers.is_empty(), "{text:?} -> {:?}", inv.numbers);
        }

        // 4. 4-digit years 1900..=2099 handled cleanly without year_re() hijacking
        for (text, exp_nums) in [
            ("Value is 2024.5.", vec!["2024.5"]),
            ("Growth was 2024%.", vec!["2024%"]),
            ("Revenue was 2024,000.", vec!["2024,000"]),
            ("Balance was -2024.", vec!["-2024"]),
            ("Balance was –2024.", vec!["–2024"]),
            ("Formula $x$-2024.", vec!["-2024"]),
            ("Tag `foo`-2024.", vec!["2024"]),
            ("Ref (A)-2024.", vec!["2024"]),
            ("Status 🚀-2024.", vec!["-2024"]),
            ("Growth was 1999%.", vec!["1999%"]),
            ("Compare: (2024)-5.", vec!["2024", "5"]),
        ] {
            let inv = extract_invariants(text, &opts(true, true));
            let expected: Vec<String> = exp_nums.into_iter().map(str::to_string).collect();
            assert_eq!(inv.numbers, expected, "{text:?}");
            for (cat, val, exact) in inv.enforced_items(&opts(true, true)) {
                assert!(contains_with_boundary(val, text, exact), "failed {cat}:{val} in {text:?}");
            }
        }

        // 6. Bidirectional line-wrap whitespace equivalence & strict exact-mode spacing
        assert!(contains_with_boundary("100-200 USD", "Budget 100-200\nUSD applies.", false));
        assert!(contains_with_boundary("100-200 USD", "Budget 100-200\r\n  USD applies.", false));
        assert!(contains_with_boundary("100-200 USD", "Budget 100-200\tUSD applies.", false));
        assert!(contains_with_boundary("100-200 USD", "Budget 100-200\u{00A0}USD applies.", false));
        assert!(contains_with_boundary("–100–200 USD", "Budget -100-200\nUSD applies.", false));
        assert!(contains_with_boundary("1,000-2,000 EUR", "Budget 1000-2000\nEUR applies.", false));
        assert!(contains_with_boundary("100-200 USD", "Budget 100-200\nusd applies.", false));
        assert!(contains_with_boundary("USD 100", "Cost USD\n100 applies.", false));
        assert!(contains_with_boundary("(de la Cruz, 2024)", "Results (de la Cruz,\n2024).", true));
        assert!(contains_with_boundary("(de la\nCruz, 2024)", "Results (de la Cruz, 2024).", true));
        assert!(contains_with_boundary("January 15, 2024", "On January 15,\n2024 it ran.", false));
        assert!(contains_with_boundary("January 15,\n2024", "On January 15, 2024 it ran.", false));
        assert!(contains_with_boundary("10-\n20", "Range 10-20 holds.", false));
        assert!(contains_with_boundary("10-20", "Range 10-\n20 holds.", false));
        // Strict negative whitespace & sign guards
        assert!(!contains_with_boundary("`a  b`", "Run `a b` now.", true));
        assert!(!contains_with_boundary("100-200 USD", "Budget 100-200\n\nUSD applies.", false));
        assert!(!contains_with_boundary("100-200 USD", "Budget 100-200\nEUR applies.", false));
        assert!(!contains_with_boundary("(de la Cruz, 2024)", "Results (de la Cruz,\n\n2024).", true));
        assert!(!contains_with_boundary("10-20", "Range 10-\n\n20 holds.", false));
        assert!(!contains_with_boundary("15", "Temp is –15 C.", false));
        assert!(!contains_with_boundary("$10", "Cost was –$10.", false));
        assert!(!contains_with_boundary("USD 100", "Balance was –USD 100.", false));
        assert!(!contains_with_boundary("USD 100", "Balance was –usd 100.", false));
        assert!(!contains_with_boundary("USD\t100", "Balance was –USD\t100.", false));
        assert!(!contains_with_boundary("USD 100", "Balance was –USD\n100.", false));
        assert!(!contains_with_boundary("US$10", "Balance was –US$10.", false));
        assert!(!contains_with_boundary("10 USD", "Cost was –10 USD.", false));
        assert!(!contains_with_boundary("100-200 USD", "Budget –100–200 USD applies.", false));
        assert!(!contains_with_boundary("100-200 USD", "Budget –100–200\nUSD applies.", false));
        assert!(!contains_with_boundary("–$10", "Cost was $10.", false));
        assert!(!contains_with_boundary("USD –100", "Balance was USD 100.", false));
        assert!(!contains_with_boundary("USD 100", "Balance was USD –100.", false));

        // 7. Inline LaTeX comparison capture & $-2026-11-21 Option A2
        for (text, exp_first) in [
            ("Compare $10=$US$20 today.", "$10"),
            ("Compare $10<$US$20 today.", "$10"),
            ("Compare $10>$US$20 today.", "$10"),
            ("Compare $10M=$US$20 today.", "$10M"),
        ] {
            let inv = extract_invariants(text, &opts(true, true));
            assert!(inv.latex_spans.is_empty(), "{text:?} -> {:?}", inv.latex_spans);
            assert_eq!(inv.currencies, vec![exp_first.to_string(), "US$20".to_string()], "{text:?}");
            for (cat, val, exact) in inv.enforced_items(&opts(true, true)) {
                assert!(contains_with_boundary(val, text, exact), "failed {cat}:{val} in {text:?}");
            }
        }
        assert_eq!(
            extract_invariants("Formula $f(x)$ and $a-b$ and $E=mc^2$.", &opts(true, true)).latex_spans,
            vec!["$f(x)$", "$a-b$", "$E=mc^2$"]
        );
        assert!(extract_invariants("Cost $10+$20, $10-$20-$30, and $10 and $20.", &opts(true, true)).latex_spans.is_empty());

        let neg_dt = extract_invariants("Cost $10 and $-2026-11-21.", &opts(true, true));
        assert_eq!(neg_dt.currencies, vec!["$10".to_string()]);
        assert!(neg_dt.dates.is_empty());
        assert_eq!(neg_dt.numbers, vec!["-2026-11-21".to_string()]);
        assert!(contains_with_boundary("-2026-11-21", "Cost $10 and $-2026-11-21.", false));
        assert!(!contains_with_boundary("2026-11-21", "$-2026-11-21", false));

        let latex_dt = extract_invariants("Formula $x$-2026-11-21 applies.", &opts(true, true));
        assert_eq!(latex_dt.latex_spans, vec!["$x$".to_string()]);
        assert_eq!(latex_dt.dates, vec!["2026-11-21".to_string()]);
        assert!(contains_with_boundary("2026-11-21", "Formula $x$-2026-11-21 applies.", false));
    }

    #[test]
    fn r8_step115_semver_comma_pct_nbsp_and_wrapped_range_guards() {
        // 1. Step 11.5 & Step 9 orphaned '.' / ',' fragment guard
        for text in [
            "Submit foo-1,000-2,000.",
            "Submit foo–1,000–2,000.",
            "Submit foo-1,000-2.",
            "Submit foo-1-2,000-3,000.",
            "Submit foo-1,000%-2,000%.",
            "Submit foo-1,000.",
            "Submit foo–1,000.",
        ] {
            let inv = extract_invariants(text, &opts(true, true));
            assert!(inv.numbers.is_empty(), "{text:?} -> {:?}", inv.numbers);
            for (cat, val, exact) in inv.enforced_items(&opts(true, true)) {
                assert!(
                    contains_with_boundary(val, text, exact),
                    "verbatim failed {cat}:{val} in {text:?}"
                );
            }
        }
        for (text, exp_num) in [
            ("Submit foo-1,000-2.12.0.", "1,000-2.12.0"),
            ("Submit foo-1,000-1.5e4.", "1,000-1.5e4"),
        ] {
            let inv = extract_invariants(text, &opts(true, true));
            assert_eq!(inv.numbers, vec![exp_num.to_string()], "{text:?}");
            assert!(contains_with_boundary(exp_num, text, false), "{text:?}");
        }

        // 2. Step 9 semver/IP/CalVer suffixes with ',' / '%' and comma-int-to-semver ranges
        for (text, exp_num) in [
            ("Test 2.12.0-1,000.", "2.12.0-1,000"),
            ("Test v2.1.0-1,000.", "v2.1.0-1,000"),
            ("Range 192.168.1.1-1,000.", "192.168.1.1-1,000"),
            ("Test 2.12.0-100%.", "2.12.0-100%"),
            ("Test v2.1.0-10%.", "v2.1.0-10%"),
            ("Range 192.168.1.1-50%.", "192.168.1.1-50%"),
            ("Release 2024.10.1.2-50%.", "2024.10.1.2-50%"),
            ("Test 1,000-2.12.0.", "1,000-2.12.0"),
            ("Test 100-2.12.0.", "100-2.12.0"),
            ("Test 1,000–v2.1.0.", "1,000–v2.1.0"),
            ("Test p < 0.05-1.5e-4 holds.", "0.05-1.5e-4"),
            ("Test p < 0.05-2.12.0 holds.", "0.05-2.12.0"),
        ] {
            let inv = extract_invariants(text, &opts(true, true));
            assert_eq!(inv.numbers, vec![exp_num.to_string()], "{text:?}");
            assert!(contains_with_boundary(exp_num, text, false), "{text:?}");
        }

        // Step 9 negative & prose-comma guards
        assert_eq!(
            extract_invariants("v2.1.0, v2.2.0", &opts(true, true)).numbers,
            vec!["v2.1.0".to_string(), "v2.2.0".to_string()]
        );
        assert_eq!(
            extract_invariants("v2.1.0-rc1, v2.2.0", &opts(true, true)).numbers,
            vec!["v2.1.0-rc1".to_string(), "v2.2.0".to_string()]
        );
        assert_eq!(
            extract_invariants("v2.1.0, next", &opts(true, true)).numbers,
            vec!["v2.1.0".to_string()]
        );
        assert!(!contains_with_boundary("100", "The cost is 100,000.", false));

        // 3. NBSP currency whitespace & raw-check wrapped-range consistency
        let inv_usd_nbsp = extract_invariants("Balance was –USD\u{00A0}100.", &opts(true, true));
        assert_eq!(inv_usd_nbsp.currencies, vec!["–USD\u{00A0}100".to_string()]);
        assert!(inv_usd_nbsp.numbers.is_empty());
        assert!(contains_with_boundary("–USD\u{00A0}100", "Balance was –USD\u{00A0}100.", false));
        assert!(!contains_with_boundary("–USD\u{00A0}100", "Balance was +USD 100.", false));
        assert!(!contains_with_boundary("USD 100", "Balance was –USD\u{00A0}100.", false));
        assert!(!contains_with_boundary("USD\u{00A0}100", "Balance was –USD\u{00A0}100.", false));

        let inv_sym_nbsp = extract_invariants("Cost was –$\u{00A0}10.", &opts(true, true));
        assert_eq!(inv_sym_nbsp.currencies, vec!["–$\u{00A0}10".to_string()]);
        assert!(inv_sym_nbsp.numbers.is_empty());
        assert!(contains_with_boundary("–$\u{00A0}10", "Cost was –$\u{00A0}10.", false));
        assert!(!contains_with_boundary("–$\u{00A0}10", "Cost was +$10.", false));
        assert!(!contains_with_boundary("$10", "Cost was –$\u{00A0}10.", false));

        assert!(!contains_with_boundary("10", "Range 10-\n20 holds.", false));
        assert!(!contains_with_boundary("20", "Range 10-\n20 holds.", false));
        assert!(!contains_with_boundary("10", "Range 10–\n20 holds.", false));
        assert!(contains_with_boundary("10-20", "Range 10-\n20 holds.", false));
        assert!(!contains_with_boundary("100-200 USD", "Budget 100-200\n\nUSD applies.", false));
        assert!(!contains_with_boundary("`a  b`", "Run `a b` now.", true));
        assert!(!contains_with_boundary("-1.5e-4", "On 2024-11-21 -\n1.5e-4 was observed.", false));
        assert!(!contains_with_boundary("10—20", "Range 10—\n20 holds.", false));
        assert!(!contains_with_boundary("-20", "Range -\n20 holds.", false));
    }

    #[test]
    fn r9_wrapped_hyphens_unified_boundaries_sci_semver_and_closing_delims() {
        let assert_verbatim = |text: &str| {
            for (pc, pq) in [(true, true), (false, false)] {
                let o = opts(pc, pq);
                let inv = extract_invariants(text, &o);
                for (cat, val, exact) in inv.enforced_items(&o) {
                    assert!(
                        contains_with_boundary(val, text, exact),
                        "verbatim failed ({pc},{pq}) for {cat}:{val:?} in {text:?} | nums={:?} curr={:?} dates={:?}",
                        inv.numbers,
                        inv.currencies,
                        inv.dates
                    );
                }
            }
        };

        // 1. Challenger Group A ((?:%|\b) + alnum/_) + fall-through traps
        for (text, exp_nums) in [
            ("Test v2.1.0-10%20.", vec!["20"]),
            ("Test 2.12.0-50%off.", vec![]),
            ("Range 192.168.1.1-50%off.", vec![]),
            ("Test 1,000-2.12.0-50%off.", vec![]),
            ("Test 1.5e4%off.", vec![]),
            ("Test 1.5e4%20.", vec!["20"]),
            ("Test 10%-20%off.", vec![]),
            ("Test 10%-20%20.", vec!["20"]),
            ("Test 1,000%-2,000%off.", vec![]),
            ("Test 2.12.0-10-50%off.", vec![]),
            ("Range 192.168.1.1-10-50%off.", vec![]),
            ("Test 10-1.5e4%off.", vec![]),
            ("Test 10-20%off.", vec![]),
            ("Test 1,000-2,000%off.", vec![]),
            ("Test 2024-11-21-50%off.", vec![]),
            ("Test 2024-11-21-01%off.", vec![]),
            ("Test p < 0.05-10%off.", vec![]),
            ("Test v2.1.0-1,000-20-30%off.", vec![]),
            ("Test 10%-20%_tag.", vec![]),
            ("Test 10%-20%é.", vec![]),
            ("Test v2.1.0-10%off-20%.", vec![]),
            ("Test 1.5e4%off-2.5e4.", vec!["2.5e4"]),
            ("Test v2.1.0-10%off-2.12.0.", vec!["2.12.0"]),
            ("Test 1.5E+4%off.", vec![]),
            ("Test v2.1.0-rc1%off.", vec![]),
            ("Test 10-20-30%off.", vec![]),
            ("Test 100%-200%off.", vec![]),
            ("Test v2.1.0-10%off.", vec![]),
            ("Test 50%off.", vec![]),
            ("Test 50% off.", vec!["50%"]),
            ("Test 10%-20%.", vec!["10%-20%"]),
            ("Test 2.12.0-50%.", vec!["2.12.0-50%"]),
            ("Test 1.5e4%.", vec!["1.5e4%"]),
            ("Test v2.1.0-10%.", vec!["v2.1.0-10%"]),
            ("Test 1,000-2.12.0-50%.", vec!["1,000-2.12.0-50%"]),
            ("Test v2.1.0-rc1%.", vec!["v2.1.0-rc1%"]),
            (
                "Percent 10%20 and 10%off and 10%-20%20 and 10%-20%off.",
                vec!["20"],
            ),
        ] {
            assert_verbatim(text);
            let inv = extract_invariants(text, &opts(true, true));
            let expected: Vec<String> = exp_nums.into_iter().map(str::to_string).collect();
            assert_eq!(inv.numbers, expected, "{text:?}");
        }
        assert!(!contains_with_boundary("10%-20%", "Test 10%-20%off.", false));
        assert!(!contains_with_boundary("2.12.0-50%", "Test 2.12.0-50%off.", false));
        assert!(!contains_with_boundary("1.5e4%", "Test 1.5e4%off.", false));
        assert!(!contains_with_boundary("v2.1.0-10%", "Test v2.1.0-10%20.", false));
        assert!(!contains_with_boundary("50%", "Test 50%off.", false));

        // 2. Challenger Group B (wrapped hyphens -\n / –\n) + masked delimiter & URL cases
        for text in [
            "Range $10-\n$20 holds.",
            "Range $10–\r\n  $20 holds.",
            "Range 100-\n200 USD holds.",
            "Range 100–\r\n  200 USD holds.",
            "Range $100-\n200.",
            "Range USD 100-\n200.",
            "Date 2024-11-21-\n1.5e-4 holds.",
            "Submit foo-\n100.",
            "Submit foo–\n100.",
            "Submit foo-\r\n  100.",
            "Submit foo-\n1,000.",
            "Submit résumé-\n100.",
            "Variant COVID-\n19 spread.",
            "Code 2024-11-21-\n01 holds.",
            "Test p < 0.05-\n100 holds.",
            "Range 10-\n20 holds.",
            "Range 10–\n20 holds.",
            "Range 10-\r\n  20 holds.",
            "Range 10–\r\n  20 holds.",
            "Formula $x$-\n5 applies.",
            "Formula $x$-\n100.",
            "Compare `foo`-\n5 today.",
            "Visit https://example.com/a-\n100 now.",
            "Range $10-\n\n$20 holds.",
            "Range 100-\n\n200 USD holds.",
            "On 2024-11-21 -\n1.5e-4 was observed.",
            "Range 10—\n20 holds.",
            "Budget 100-200\nUSD applies.",
            "Budget 100-200\n\nUSD applies.",
            "On 2024-11-21-\n2025.",
            "On January 15, 2024-\n2025.",
            "On 15 Jan 2024-\n2025.",
            "Dates May 2024% and May 2024-2025 and May 2024-\n2025.",
        ] {
            assert_verbatim(text);
        }
        assert_eq!(
            extract_invariants("Range $10-\n$20 holds.", &opts(true, true)).currencies,
            vec!["$10-$20".to_string()]
        );
        assert_eq!(
            extract_invariants("Range 100-\n200 USD holds.", &opts(true, true)).currencies,
            vec!["100-200 USD".to_string()]
        );
        assert_eq!(
            extract_invariants("Date 2024-11-21-\n1.5e-4 holds.", &opts(true, true)).numbers,
            vec!["2024-11-21-1.5e-4".to_string()]
        );
        assert!(extract_invariants("Submit foo-\n100.", &opts(true, true)).numbers.is_empty());
        assert!(extract_invariants("Submit foo–\n100.", &opts(true, true)).numbers.is_empty());
        assert!(extract_invariants("Variant COVID-\n19 spread.", &opts(true, true)).numbers.is_empty());
        assert_eq!(
            extract_invariants("Code 2024-11-21-\n01 holds.", &opts(true, true)).numbers,
            vec!["2024-11-21-01".to_string()]
        );
        assert_eq!(
            extract_invariants("Test p < 0.05-\n100 holds.", &opts(true, true)).numbers,
            vec!["0.05-100".to_string()]
        );
        let inv_latex_wrap = extract_invariants("Formula $x$-\n5 applies.", &opts(true, true));
        assert_eq!(inv_latex_wrap.latex_spans, vec!["$x$".to_string()]);
        assert_eq!(inv_latex_wrap.numbers, vec!["-5".to_string()]);
        let inv_code_wrap = extract_invariants("Compare `foo`-\n5 today.", &opts(true, true));
        assert_eq!(inv_code_wrap.code_blocks, vec!["`foo`".to_string()]);
        assert_eq!(inv_code_wrap.numbers, vec!["5".to_string()]);
        let inv_url_wrap = extract_invariants("Visit https://example.com/a-\n100 now.", &opts(true, true));
        assert_eq!(inv_url_wrap.urls, vec!["https://example.com/a-".to_string()]);
        assert!(inv_url_wrap.numbers.is_empty());
        assert!(!contains_with_boundary("10", "Range 10–\r\n  20 holds.", false));
        assert!(contains_with_boundary("10-20", "Range 10–\r\n  20 holds.", false));
        assert!(contains_with_boundary("100-200 USD", "Budget 100-200\nUSD applies.", false));
        assert!(!contains_with_boundary("100-200 USD", "Budget 100-200\n\nUSD applies.", false));

        // 3. Sci-to-semver Alternation 3
        for (text, exp_num) in [
            ("Test 1.5e-4-2.12.0.", "1.5e-4-2.12.0"),
            ("Test 1.5e4-2.12.0.", "1.5e4-2.12.0"),
            ("Test 1.5e-4–v2.1.0.", "1.5e-4–v2.1.0"),
            ("Test 1.5e-4-192.168.1.1.", "1.5e-4-192.168.1.1"),
            ("Test 1.5e-4-2024.10.1.2.", "1.5e-4-2024.10.1.2"),
            ("Test 1,000e-4-2.12.0.", "1,000e-4-2.12.0"),
            ("Test 1.5e-4-2.12.0%.", "1.5e-4-2.12.0%"),
        ] {
            assert_verbatim(text);
            let inv = extract_invariants(text, &opts(true, true));
            assert_eq!(inv.numbers, vec![exp_num.to_string()], "{text:?}");
        }

        // 4. Unified non-exact boundary gate (currency, date, p-value) & closing delimiters
        for text in [
            "Pay $10 - EUR 20 and $10-USD 20.",
            "Pay -$10-$20 and $-10 and $-10-$20.",
            "Cost is $10%.",
            "Range is $10-$20%.",
            "Range is USD 100-200%.",
            "Cost is USD 100%.",
            "Compare $10%=$US$20%.",
            "Cost is $10Moff.",
            "Cost is $10off.",
            "Cost is USD 100off.",
            "Cost is $10_000.",
            "Cost is $10M_x.",
            "Price $10-2.12.0.",
            "Price $10-1.5e-4.",
            "Price $10-2024-11-21.",
            "Price $10,2024.",
            "Price BUS$10.",
            "Price x$10.",
            "Price 10$20.",
            "Price _-$10.",
            "Compare $x$10.",
            "On 2024-11-21%.",
            "On January 15, 2024%.",
            "On 15 Jan 2024%.",
            "On 01/15/2024%.",
            "On 2024-11-21.5.",
            "On 1.2024-11-21.",
            "On January 15, 2024.5.",
            "On 15 Jan 2024.5.",
            "On 1.15 Jan 2024.",
            "On 01/15/2024.5.",
            "On 1.01/15/2024.",
            "On 2024-11-21T10:30:00Z-5.",
            "On 10%-2024-11-21.",
            "On \"foo\"-2024-11-21.",
            "On 100-January 15, 2024.",
            "Test p < 0.05%.",
            "Test p < .05%.",
            "Compare \"foo\"-2.12.0.",
            "Compare {foo}-2.12.0.",
            "She said “foo”-2.12.0 and 'baz'-2.12.0.",
        ] {
            assert_verbatim(text);
        }
        assert_eq!(
            extract_invariants("On 2024-11-21T10:30:00Z-5.", &opts(true, true)).dates,
            vec!["2024-11-21T10:30:00Z".to_string()]
        );
        assert!(extract_invariants("On 2024-11-21T10:30:00Z-5.", &opts(true, true)).numbers.is_empty());
        assert_eq!(
            extract_invariants("On 10%-2024-11-21.", &opts(true, true)).dates,
            vec!["2024-11-21".to_string()]
        );
        assert_eq!(
            extract_invariants("On \"foo\"-2024-11-21.", &opts(true, true)).dates,
            vec!["2024-11-21".to_string()]
        );
        assert_eq!(
            extract_invariants("Compare \"foo\"-2.12.0.", &opts(true, true)).numbers,
            vec!["2.12.0".to_string()]
        );
        assert_eq!(
            extract_invariants("Compare {foo}-2.12.0.", &opts(true, true)).numbers,
            vec!["2.12.0".to_string()]
        );
        assert_eq!(
            extract_invariants("She said “foo”-2.12.0 and 'baz'-2.12.0.", &opts(true, true)).numbers,
            vec!["2.12.0".to_string()]
        );
        assert_eq!(
            extract_invariants("Test p < 0.05%.", &opts(true, true)).numbers,
            vec!["0.05%".to_string()]
        );

        // Negative opening-quote & boundary guards remain strict
        assert!(!contains_with_boundary("15", "She said \"-15\" today.", false));
        assert!(!contains_with_boundary("$10", "She said \"-$10\" today.", false));
        assert!(!contains_with_boundary("2.12.0", "She said \"-2.12.0\" today.", false));
        assert!(!contains_with_boundary("2026-11-21", "$-2026-11-21", false));
        assert!(!contains_with_boundary("100", "The cost is 100,000.", false));
    }
}
