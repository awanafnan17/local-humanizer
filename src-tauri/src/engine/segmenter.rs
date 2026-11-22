#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegmentKind {
    Prose { is_heading: bool, is_list_item: bool },
    VerbatimBlock { fence: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub index: usize,
    pub kind: SegmentKind,
    pub text: String,
    pub word_count: usize,
}

pub(crate) fn is_cjk_or_kana(c: char) -> bool {
    matches!(
        c as u32,
        0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0x3040..=0x309F | 0x30A0..=0x30FF
    )
}

fn is_outer_word_punct(c: char) -> bool {
    matches!(
        c,
        '`' | '*'
            | '_'
            | '~'
            | '"'
            | '\''
            | '“'
            | '”'
            | '‘'
            | '’'
            | '('
            | ')'
            | '['
            | ']'
            | '{'
            | '}'
            | '<'
            | '>'
            | '.'
            | ','
            | ';'
            | ':'
            | '!'
            | '?'
            | '•'
            | '·'
    )
}

pub fn count_words(text: &str) -> usize {
    if text.trim().is_empty() {
        return 0;
    }
    let mut total = 0usize;
    let normalized = text.replace(['\u{2013}', '\u{2014}'], " ");
    for raw_tok in normalized.split_whitespace() {
        let tok = raw_tok.trim_matches(is_outer_word_punct);
        if tok.is_empty() || !tok.chars().any(char::is_alphanumeric) {
            continue;
        }
        let mut cjk_count = 0usize;
        let mut has_non_cjk_alnum = false;
        for c in tok.chars() {
            if is_cjk_or_kana(c) {
                cjk_count += 1;
            } else if c.is_alphanumeric() {
                has_non_cjk_alnum = true;
            }
        }
        if cjk_count > 0 {
            total += cjk_count;
            if has_non_cjk_alnum {
                total += 1;
            }
        } else {
            total += 1;
        }
    }
    total
}

fn is_bidi_override(c: char) -> bool {
    matches!(c as u32, 0x202A..=0x202E | 0x2066..=0x2069)
}

fn detect_fence_open(trimmed_start: &str) -> Option<(char, usize)> {
    let first = trimmed_start.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let count = trimmed_start.chars().take_while(|&c| c == first).count();
    if count >= 3 {
        // Inline backtick code on a single line like ```foo``` is NOT a multi-line fence opener
        let rest = &trimmed_start[count..];
        if first == '`' && rest.contains('`') {
            return None;
        }
        Some((first, count))
    } else {
        None
    }
}

fn is_fence_close(trimmed: &str, open_ch: char, open_len: usize) -> bool {
    let count = trimmed.chars().take_while(|&c| c == open_ch).count();
    count >= open_len && trimmed[count..].trim().is_empty()
}

/// O(n) single-pass input sanitizer:
/// - Strips NUL, Bidi overrides, ANSI CSI escape sequences, and disallowed C0 controls
/// - Normalizes `\r\n` and `\r` -> `\n`
/// - Collapses >4 consecutive horizontal ASCII spaces outside fenced blocks
/// - Caps consecutive blank lines outside fenced blocks to `\n\n`
pub fn sanitize_input(raw: &str) -> String {
    let mut cleaned = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    while let Some(nc) = chars.next() {
                        if ('@'..='~').contains(&nc) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(nc) = chars.next() {
                        if nc == '\x07' {
                            break;
                        }
                        if nc == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        if c == '\0' || is_bidi_override(c) {
            continue;
        }
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            cleaned.push('\n');
            continue;
        }
        if (c as u32) < 0x20 && c != '\n' && c != '\t' || c == '\x7f' {
            continue;
        }
        cleaned.push(c);
    }

    let mut out = String::with_capacity(cleaned.len());
    let mut in_fence: Option<(char, usize)> = None;
    let mut in_math = false;
    let mut consecutive_blanks = 0usize;

    for line in cleaned.split('\n') {
        let trimmed_start = line.trim_start();
        let trimmed = line.trim();

        if let Some((fc, flen)) = in_fence {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(line);
            if is_fence_close(trimmed, fc, flen) {
                in_fence = None;
            }
            consecutive_blanks = 0;
            continue;
        }

        if in_math {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(line);
            if trimmed == "$$" {
                in_math = false;
            }
            consecutive_blanks = 0;
            continue;
        }

        if let Some((fc, flen)) = detect_fence_open(trimmed_start) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(line);
            in_fence = Some((fc, flen));
            consecutive_blanks = 0;
            continue;
        }

        if trimmed == "$$" {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(line);
            in_math = true;
            consecutive_blanks = 0;
            continue;
        }

        if trimmed.is_empty() {
            consecutive_blanks += 1;
            if consecutive_blanks <= 1 && !out.is_empty() {
                out.push('\n');
            }
            continue;
        }

        consecutive_blanks = 0;
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&collapse_excessive_spaces(line));
    }

    out.trim().to_string()
}

fn collapse_excessive_spaces(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut space_run = 0usize;
    for c in line.chars() {
        if c == ' ' {
            space_run += 1;
        } else {
            if space_run > 4 {
                out.push(' ');
            } else {
                for _ in 0..space_run {
                    out.push(' ');
                }
            }
            space_run = 0;
            out.push(c);
        }
    }
    if space_run > 0 && !out.is_empty() {
        // Trim trailing horizontal spaces
    }
    out
}

fn detect_atx_heading_hashes(line: &str) -> Option<usize> {
    let trimmed_start = line.trim_start();
    let hashes = trimmed_start.chars().take_while(|&c| c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let after_hashes = &trimmed_start[hashes..];
    let first_after = after_hashes.chars().next()?;
    if first_after != ' ' && first_after != '\t' {
        return None;
    }
    if after_hashes.trim().is_empty() {
        return None;
    }
    Some(hashes)
}

fn detect_heading_or_list(text: &str) -> (bool, bool) {
    let trimmed = text.trim_start();
    let is_heading = detect_atx_heading_hashes(trimmed).is_some();

    let is_bullet = trimmed.starts_with("- ")
        || trimmed.starts_with("* ")
        || trimmed.starts_with("+ ");
    let is_numbered = trimmed
        .split_once(". ")
        .map(|(prefix, _)| !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or(false);

    (is_heading, is_bullet || is_numbered)
}

pub fn segment_document(sanitized: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    if sanitized.trim().is_empty() {
        return segments;
    }

    let mut prose_lines: Vec<&str> = Vec::new();
    let mut block_lines: Vec<&str> = Vec::new();
    let mut in_fence: Option<(char, usize, String)> = None;
    let mut in_math = false;

    let flush_prose = |lines: &mut Vec<&str>, segments: &mut Vec<Segment>| {
        if lines.is_empty() {
            return;
        }
        let joined = lines.join("\n");
        lines.clear();
        if joined.trim().is_empty() {
            return;
        }
        let (is_heading, is_list_item) = detect_heading_or_list(&joined);
        let word_count = count_words(&joined);
        let index = segments.len();
        segments.push(Segment {
            index,
            kind: SegmentKind::Prose {
                is_heading,
                is_list_item,
            },
            text: joined,
            word_count,
        });
    };

    for line in sanitized.split('\n') {
        let trimmed_start = line.trim_start();
        let trimmed = line.trim();

        if let Some((fc, flen, fence_label)) = in_fence.clone() {
            block_lines.push(line);
            if is_fence_close(trimmed, fc, flen) {
                let text = block_lines.join("\n");
                block_lines.clear();
                let word_count = count_words(&text);
                let index = segments.len();
                segments.push(Segment {
                    index,
                    kind: SegmentKind::VerbatimBlock { fence: fence_label },
                    text,
                    word_count,
                });
                in_fence = None;
            }
            continue;
        }

        if in_math {
            block_lines.push(line);
            if trimmed == "$$" {
                let text = block_lines.join("\n");
                block_lines.clear();
                let word_count = count_words(&text);
                let index = segments.len();
                segments.push(Segment {
                    index,
                    kind: SegmentKind::VerbatimBlock {
                        fence: "$$".to_string(),
                    },
                    text,
                    word_count,
                });
                in_math = false;
            }
            continue;
        }

        if let Some((fc, flen)) = detect_fence_open(trimmed_start) {
            flush_prose(&mut prose_lines, &mut segments);
            let fence_str: String = std::iter::repeat_n(fc, flen).collect();
            block_lines.push(line);
            in_fence = Some((fc, flen, fence_str));
            continue;
        }

        if trimmed == "$$" {
            flush_prose(&mut prose_lines, &mut segments);
            block_lines.push(line);
            in_math = true;
            continue;
        }

        if let Some(hash_count) = detect_atx_heading_hashes(line) {
            flush_prose(&mut prose_lines, &mut segments);
            let text = line.trim_end().to_string();
            let word_count = count_words(&text);
            let index = segments.len();
            segments.push(Segment {
                index,
                kind: SegmentKind::VerbatimBlock {
                    fence: "#".repeat(hash_count),
                },
                text,
                word_count,
            });
            continue;
        }

        if trimmed.is_empty() {
            flush_prose(&mut prose_lines, &mut segments);
        } else {
            prose_lines.push(line);
        }
    }

    // Flush unclosed fence/math or trailing prose gracefully
    if let Some((_, _, fence_label)) = in_fence {
        let text = block_lines.join("\n");
        let word_count = count_words(&text);
        let index = segments.len();
        segments.push(Segment {
            index,
            kind: SegmentKind::VerbatimBlock { fence: fence_label },
            text,
            word_count,
        });
    } else if in_math {
        let text = block_lines.join("\n");
        let word_count = count_words(&text);
        let index = segments.len();
        segments.push(Segment {
            index,
            kind: SegmentKind::VerbatimBlock {
                fence: "$$".to_string(),
            },
            text,
            word_count,
        });
    } else {
        flush_prose(&mut prose_lines, &mut segments);
    }

    segments
}

pub fn count_paragraphs(text: &str) -> usize {
    segment_document(&sanitize_input(text)).len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn word_count_handles_unicode_cjk_emojis_and_dashes() {
        assert_eq!(count_words(""), 0);
        assert_eq!(count_words("... --- !!!"), 0);
        assert_eq!(count_words("Serendipity"), 1);
        assert_eq!(count_words("state-of-the-art"), 1);
        assert_eq!(count_words("word1—word2"), 2);
        assert_eq!(count_words("word1–word2"), 2);
        assert_eq!(count_words("José visited Zürich 🚀 👩🏽‍🔬"), 3);
        assert_eq!(count_words("本地推理模型 ローカル推論 로컬모델"), 13);
    }

    #[test]
    fn fenced_code_with_blank_lines_stays_single_verbatim_block() {
        let doc = "Intro paragraph.\r\n\r\n```rust\nfn main() {\n\n    println!(\"hi\");\n}\n```\r\n\r\nOutro paragraph.";
        let sanitized = sanitize_input(doc);
        let segs = segment_document(&sanitized);
        assert_eq!(segs.len(), 3);
        assert!(matches!(segs[0].kind, SegmentKind::Prose { .. }));
        assert!(matches!(segs[1].kind, SegmentKind::VerbatimBlock { .. }));
        assert!(segs[1].text.contains("\n\n    println!"));
        assert!(matches!(segs[2].kind, SegmentKind::Prose { .. }));
        assert_eq!(count_paragraphs(doc), 3);
    }

    #[test]
    fn display_math_and_unclosed_fence_do_not_panic() {
        let doc = "$$\nE = mc^2\n\na^2 + b^2 = c^2\n$$\n\n~~~python\ndef f():\n    pass";
        let segs = segment_document(&sanitize_input(doc));
        assert_eq!(segs.len(), 2);
        assert!(matches!(segs[0].kind, SegmentKind::VerbatimBlock { .. }));
        assert!(matches!(segs[1].kind, SegmentKind::VerbatimBlock { .. }));
    }

    #[test]
    fn pathological_inputs_sanitize_fast_without_redos() {
        let mut bad = String::new();
        bad.push_str("\x00\u{202e}\x1b[31mHello\x1b[0m");
        bad.push_str(&" ".repeat(10_000));
        bad.push_str(&"\n".repeat(500));
        bad.push_str("world");
        bad.push_str(&"\u{0301}".repeat(64));

        let start = Instant::now();
        let clean = sanitize_input(&bad);
        assert!(start.elapsed().as_millis() < 100);
        assert!(!clean.contains('\0'));
        assert!(!clean.contains('\u{202e}'));
        assert!(!clean.contains('\x1b'));
        assert!(clean.contains("Hello\n\nworld"));
    }

    #[test]
    fn sanitize_input_strips_osc_and_csi_ansi_escapes() {
        let raw = "\x1b]8;;https://evil.com\x07Link\x1b]8;;\x07 and \x1b]0;Window Title\x1b\\End";
        let clean = sanitize_input(raw);
        assert_eq!(clean, "Link and End");
    }

    #[test]
    fn markdown_atx_headings_become_verbatim_blocks_while_fenced_comments_stay_in_code() {
        let doc = "# Main Title\n\n## 1. Introduction and Operational Context\n\nFirst prose paragraph.\n\n```python\n# Python comment inside code\nx = 1\n```\n\n### Subsection Without Blank\nSecond prose paragraph.\n\n####### Not a heading\n\n#hashtag without space";
        let segs = segment_document(&sanitize_input(doc));
        assert_eq!(segs.len(), 8);
        assert!(matches!(segs[0].kind, SegmentKind::VerbatimBlock { ref fence } if fence == "#"));
        assert_eq!(segs[0].text, "# Main Title");
        assert!(matches!(segs[1].kind, SegmentKind::VerbatimBlock { ref fence } if fence == "##"));
        assert_eq!(segs[1].text, "## 1. Introduction and Operational Context");
        assert!(matches!(segs[2].kind, SegmentKind::Prose { .. }));
        assert_eq!(segs[2].text, "First prose paragraph.");
        assert!(matches!(segs[3].kind, SegmentKind::VerbatimBlock { ref fence } if fence == "```"));
        assert!(segs[3].text.contains("# Python comment inside code"));
        assert!(matches!(segs[4].kind, SegmentKind::VerbatimBlock { ref fence } if fence == "###"));
        assert_eq!(segs[4].text, "### Subsection Without Blank");
        assert!(matches!(segs[5].kind, SegmentKind::Prose { .. }));
        assert_eq!(segs[5].text, "Second prose paragraph.");
        assert!(matches!(segs[6].kind, SegmentKind::Prose { .. }));
        assert_eq!(segs[6].text, "####### Not a heading");
        assert!(matches!(segs[7].kind, SegmentKind::Prose { .. }));
        assert_eq!(segs[7].text, "#hashtag without space");
    }
}
