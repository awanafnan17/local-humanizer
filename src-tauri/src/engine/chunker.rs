use crate::commands::RewriteOptions;
use crate::engine::protect::{extract_invariants, protected_split_intervals, InvariantSet};
use crate::engine::segmenter::{count_paragraphs, count_words, Segment, SegmentKind};

pub const MIN_TARGET_WORDS: usize = 90;
pub const MAX_TARGET_WORDS: usize = 150;
pub const HARD_CAP_WORDS: usize = 180;
const MAX_CHUNK_BYTES: usize = 1200;
const MAX_CONTEXT_WORDS: usize = 70;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkJoin {
    First,
    ParagraphBreak,
    Space,
    Exact,
}

#[derive(Debug, Clone)]
pub struct ChunkSpec {
    pub chunk_index: usize,
    pub text: String,
    pub word_count: usize,
    pub paragraph_count: usize,
    pub join_with_prev: ChunkJoin,
    pub invariants: InvariantSet,
}

#[derive(Debug, Clone)]
pub enum WorkUnit {
    Verbatim {
        index: usize,
        text: String,
        join_with_prev: ChunkJoin,
    },
    ParaphraseChunk(ChunkSpec),
}

fn is_protected_pos(pos: usize, intervals: &[(usize, usize)]) -> bool {
    intervals.iter().any(|&(s, e)| pos > s && pos < e)
}

fn ends_with_known_abbreviation(prefix: &str) -> bool {
    let trimmed = prefix.trim_end();
    let lower = trimmed.to_ascii_lowercase();
    const ABBREVS: &[&str] = &[
        "dr.", "mr.", "ms.", "mrs.", "prof.", "sr.", "jr.", "vs.", "fig.", "no.", "pp.", "vol.",
        "inc.", "ltd.", "corp.", "st.", "jan.", "feb.", "mar.", "apr.", "aug.", "sept.", "sep.",
        "oct.", "nov.", "dec.", "et al.", "e.g.", "i.e.", "u.s.", "u.k.", "cf.", "approx.",
        "dept.", "est.", "rev.", "ed.",
    ];
    if ABBREVS.iter().any(|a| lower.ends_with(a)) {
        return true;
    }
    // Single uppercase initial like "J." or "A."
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 2 && bytes[bytes.len() - 1] == b'.' {
        let prev = bytes[bytes.len() - 2];
        if prev.is_ascii_uppercase()
            && (bytes.len() == 2 || !bytes[bytes.len() - 3].is_ascii_alphanumeric())
        {
            return true;
        }
    }
    false
}

fn is_page_abbrev(prefix: &str, suffix: &str) -> bool {
    let trimmed = prefix.trim_end();
    let bytes = trimmed.as_bytes();
    let ends_with_standalone_p = bytes.len() >= 2
        && bytes[bytes.len() - 1] == b'.'
        && matches!(bytes[bytes.len() - 2], b'p' | b'P')
        && (bytes.len() == 2 || !bytes[bytes.len() - 3].is_ascii_alphanumeric());
    if !ends_with_standalone_p {
        return false;
    }
    suffix
        .trim_start()
        .chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false)
}

/// Abbreviation-safe and URL/DOI/citation/quote-safe UTF-8 sentence splitter.
pub fn split_sentences_safe(text: &str) -> Vec<String> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let protected = protected_split_intervals(text);
    let mut sentences = Vec::new();
    let mut start = 0usize;
    let indices: Vec<(usize, char)> = text.char_indices().collect();

    for (pos_idx, &(byte_idx, ch)) in indices.iter().enumerate() {
        if !matches!(ch, '.' | '!' | '?' | '。' | '！' | '？') {
            continue;
        }
        let end_punct = byte_idx + ch.len_utf8();
        if is_protected_pos(byte_idx, &protected) || is_protected_pos(end_punct, &protected) {
            continue;
        }
        // Skip decimal numbers like 3.14
        if ch == '.' {
            let prev_digit = pos_idx > 0 && indices[pos_idx - 1].1.is_ascii_digit();
            let next_digit = indices
                .get(pos_idx + 1)
                .map(|&(_, c)| c.is_ascii_digit())
                .unwrap_or(false);
            if prev_digit && next_digit {
                continue;
            }
        }
        // Include any trailing closing quote/paren attached to sentence end
        let mut split_at = end_punct;
        let mut look_idx = pos_idx + 1;
        while let Some(&(b2, c2)) = indices.get(look_idx) {
            if matches!(c2, '"' | '\'' | '”' | '’' | ')' | ']') {
                split_at = b2 + c2.len_utf8();
                look_idx += 1;
            } else {
                break;
            }
        }
        // Must be followed by whitespace or EOF (for ASCII .!?)
        let next_is_boundary = indices
            .get(look_idx)
            .map(|&(_, c)| c.is_whitespace() || matches!(ch, '。' | '！' | '？'))
            .unwrap_or(true);
        if !next_is_boundary {
            continue;
        }
        if ch == '.'
            && (ends_with_known_abbreviation(&text[start..end_punct])
                || is_page_abbrev(&text[start..end_punct], &text[end_punct..]))
        {
            continue;
        }

        let piece = text[start..split_at].trim();
        if !piece.is_empty() {
            sentences.push(piece.to_string());
        }
        start = split_at;
        while start < text.len() && text[start..].starts_with(char::is_whitespace) {
            let c = text[start..].chars().next().unwrap();
            start += c.len_utf8();
        }
    }

    if start < text.len() {
        let tail = text[start..].trim();
        if !tail.is_empty() {
            sentences.push(tail.to_string());
        }
    }
    sentences
}

fn split_oversized_sentence(sentence: &str) -> Vec<(String, ChunkJoin)> {
    let words: Vec<&str> = sentence.split_whitespace().collect();
    let total_words = count_words(sentence);
    if words.len() <= 1 && (sentence.len() > MAX_CHUNK_BYTES || total_words > HARD_CAP_WORDS) {
        // Giant unspaced token (e.g. 20,000-char token or unspaced CJK): split safely on UTF-8 char
        // boundaries while keeping both byte size <= MAX_CHUNK_BYTES and word count <= MAX_TARGET_WORDS.
        let mut out = Vec::new();
        let mut start = 0usize;
        let mut piece_bytes = 0usize;
        let mut piece_cjk_words = 0usize;
        for (idx, ch) in sentence.char_indices() {
            let ch_len = ch.len_utf8();
            let ch_words = usize::from(crate::engine::segmenter::is_cjk_or_kana(ch));
            if idx > start
                && (piece_bytes + ch_len > MAX_CHUNK_BYTES
                    || piece_cjk_words + ch_words > MAX_TARGET_WORDS)
            {
                let join = if out.is_empty() {
                    ChunkJoin::Space
                } else {
                    ChunkJoin::Exact
                };
                out.push((sentence[start..idx].to_string(), join));
                start = idx;
                piece_bytes = 0;
                piece_cjk_words = 0;
            }
            piece_bytes += ch_len;
            piece_cjk_words += ch_words;
        }
        if start < sentence.len() {
            let join = if out.is_empty() {
                ChunkJoin::Space
            } else {
                ChunkJoin::Exact
            };
            out.push((sentence[start..].to_string(), join));
        }
        return out;
    }

    if total_words <= HARD_CAP_WORDS && sentence.len() <= MAX_CHUNK_BYTES * 3 {
        return vec![(sentence.to_string(), ChunkJoin::Space)];
    }

    let mut out = Vec::new();
    let mut cur = Vec::new();
    let mut cur_bytes = 0usize;
    let mut cur_words = 0usize;
    for w in words {
        let w_words = count_words(w);
        if w.len() > MAX_CHUNK_BYTES || w_words > HARD_CAP_WORDS {
            if !cur.is_empty() {
                out.push((cur.join(" "), ChunkJoin::Space));
                cur.clear();
                cur_bytes = 0;
                cur_words = 0;
            }
            for piece in split_oversized_sentence(w) {
                out.push(piece);
            }
            continue;
        }
        if !cur.is_empty()
            && (cur_words + w_words > MAX_TARGET_WORDS || cur_bytes + w.len() + 1 > MAX_CHUNK_BYTES * 2)
        {
            out.push((cur.join(" "), ChunkJoin::Space));
            cur.clear();
            cur_bytes = 0;
            cur_words = 0;
        }
        cur_bytes += w.len() + 1;
        cur_words += w_words;
        cur.push(w);
    }
    if !cur.is_empty() {
        out.push((cur.join(" "), ChunkJoin::Space));
    }
    out
}

fn split_giant_paragraph(text: &str, first_join: ChunkJoin) -> Vec<(String, ChunkJoin)> {
    let sentences = split_sentences_safe(text);
    if sentences.is_empty() {
        return vec![(text.to_string(), first_join)];
    }

    let mut pieces: Vec<(String, ChunkJoin)> = Vec::new();
    let mut buf = String::new();
    let mut buf_words = 0usize;

    let flush_buf = |buf: &mut String, buf_words: &mut usize, pieces: &mut Vec<(String, ChunkJoin)>| {
        if !buf.trim().is_empty() {
            let join = if pieces.is_empty() {
                first_join
            } else {
                ChunkJoin::Space
            };
            pieces.push((buf.trim().to_string(), join));
            buf.clear();
            *buf_words = 0;
        }
    };

    for sent in sentences {
        let sw = count_words(&sent);
        if sw > HARD_CAP_WORDS || sent.len() > MAX_CHUNK_BYTES * 2 {
            flush_buf(&mut buf, &mut buf_words, &mut pieces);
            for (sub, sub_join) in split_oversized_sentence(&sent) {
                let join = if pieces.is_empty() {
                    first_join
                } else {
                    sub_join
                };
                pieces.push((sub, join));
            }
            continue;
        }
        if !buf.is_empty()
            && (buf_words + sw > MAX_TARGET_WORDS
                && (buf_words >= MIN_TARGET_WORDS || buf_words + sw > HARD_CAP_WORDS))
        {
            flush_buf(&mut buf, &mut buf_words, &mut pieces);
        }
        if !buf.is_empty() {
            buf.push(' ');
        }
        buf.push_str(&sent);
        buf_words += sw;
    }
    flush_buf(&mut buf, &mut buf_words, &mut pieces);
    pieces
}

pub fn build_work_units(segments: &[Segment], options: &RewriteOptions) -> Vec<WorkUnit> {
    let mut units = Vec::new();
    let mut chunk_counter = 0usize;
    let mut has_emitted_unit = false;

    for seg in segments {
        match &seg.kind {
            SegmentKind::VerbatimBlock { .. } => {
                let join = if has_emitted_unit {
                    ChunkJoin::ParagraphBreak
                } else {
                    ChunkJoin::First
                };
                units.push(WorkUnit::Verbatim {
                    index: units.len(),
                    text: seg.text.clone(),
                    join_with_prev: join,
                });
                has_emitted_unit = true;
            }
            SegmentKind::Prose { .. } => {
                if seg.word_count == 0 {
                    let join = if has_emitted_unit {
                        ChunkJoin::ParagraphBreak
                    } else {
                        ChunkJoin::First
                    };
                    units.push(WorkUnit::Verbatim {
                        index: units.len(),
                        text: seg.text.clone(),
                        join_with_prev: join,
                    });
                    has_emitted_unit = true;
                    continue;
                }

                let first_join = if has_emitted_unit {
                    ChunkJoin::ParagraphBreak
                } else {
                    ChunkJoin::First
                };

                if seg.word_count > HARD_CAP_WORDS || seg.text.len() > MAX_CHUNK_BYTES * 2 {
                    for (sub_text, sub_join) in split_giant_paragraph(&seg.text, first_join) {
                        let wc = count_words(&sub_text);
                        let p_count = count_paragraphs(&sub_text).max(1);
                        let inv = extract_invariants(&sub_text, options);
                        units.push(WorkUnit::ParaphraseChunk(ChunkSpec {
                            chunk_index: chunk_counter,
                            text: sub_text,
                            word_count: wc,
                            paragraph_count: p_count,
                            join_with_prev: sub_join,
                            invariants: inv,
                        }));
                        chunk_counter += 1;
                        has_emitted_unit = true;
                    }
                    continue;
                }

                let inv = extract_invariants(&seg.text, options);
                units.push(WorkUnit::ParaphraseChunk(ChunkSpec {
                    chunk_index: chunk_counter,
                    text: seg.text.clone(),
                    word_count: seg.word_count,
                    paragraph_count: 1,
                    join_with_prev: first_join,
                    invariants: inv,
                }));
                chunk_counter += 1;
                has_emitted_unit = true;
            }
        }
    }

    units
}

pub fn trailing_sentences(accepted_output: &str, max_sentences: usize) -> String {
    let trimmed = accepted_output.trim();
    if trimmed.is_empty() || max_sentences == 0 {
        return String::new();
    }
    let sents = split_sentences_safe(trimmed);
    if sents.is_empty() {
        return String::new();
    }
    let start = sents.len().saturating_sub(max_sentences);
    let mut joined = sents[start..].join(" ");
    let words: Vec<&str> = joined.split_whitespace().collect();
    if words.len() > MAX_CONTEXT_WORDS {
        joined = words[words.len() - MAX_CONTEXT_WORDS..].join(" ");
    }
    joined
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::VoiceOption;
    use crate::engine::segmenter::{sanitize_input, segment_document};

    fn default_opts() -> RewriteOptions {
        RewriteOptions {
            preserve_length: true,
            preserve_formatting: true,
            preserve_citations: true,
            preserve_quotes: true,
            language: "en".to_string(),
            voice: VoiceOption::Auto,
        }
    }

    #[test]
    fn chunks_3100_word_document_into_single_paragraph_and_sentence_group_windows() {
        let mut paras = Vec::new();
        for p in 0..35 {
            let mut s = format!("Paragraph {p} begins with a clear factual claim about system reliability. ");
            for j in 0..11 {
                s.push_str(&format!(
                    "Observation {j} confirms throughput remained steady at 94.5% across cluster nodes. "
                ));
            }
            paras.push(s.trim().to_string());
        }
        let doc = paras.join("\n\n");
        assert!(count_words(&doc) >= 3000);
        let segs = segment_document(&sanitize_input(&doc));
        let units = build_work_units(&segs, &default_opts());
        let chunks: Vec<&ChunkSpec> = units
            .iter()
            .filter_map(|u| match u {
                WorkUnit::ParaphraseChunk(c) => Some(c),
                _ => None,
            })
            .collect();
        assert_eq!(chunks.len(), 35);
        for (i, ch) in chunks.iter().enumerate() {
            assert_eq!(ch.chunk_index, i);
            assert_eq!(ch.paragraph_count, 1);
            assert!(ch.word_count <= HARD_CAP_WORDS, "chunk {i} exceeded cap: {}", ch.word_count);
            if i > 0 {
                assert_eq!(ch.join_with_prev, ChunkJoin::ParagraphBreak);
            }
        }
    }

    #[test]
    fn single_paragraph_chunking_splits_above_hard_cap_without_packing_multiple_paragraphs() {
        let short_p1 = "First concise paragraph with ten simple words inside it.";
        let short_p2 = "Second concise paragraph also kept in its own unit.";
        let long_p3 = (0..22)
            .map(|i| format!("Sentence {i} describes reliable local processing across distributed nodes."))
            .collect::<Vec<_>>()
            .join(" ");
        let doc = format!("{short_p1}\n\n{short_p2}\n\n{long_p3}");
        let segs = segment_document(&sanitize_input(&doc));
        let units = build_work_units(&segs, &default_opts());
        let chunks: Vec<&ChunkSpec> = units
            .iter()
            .filter_map(|u| match u {
                WorkUnit::ParaphraseChunk(c) => Some(c),
                _ => None,
            })
            .collect();
        assert!(chunks.len() >= 4);
        assert_eq!(chunks[0].text, short_p1);
        assert_eq!(chunks[0].join_with_prev, ChunkJoin::First);
        assert_eq!(chunks[1].text, short_p2);
        assert_eq!(chunks[1].join_with_prev, ChunkJoin::ParagraphBreak);
        assert_eq!(chunks[2].join_with_prev, ChunkJoin::ParagraphBreak);
        assert_eq!(chunks[3].join_with_prev, ChunkJoin::Space);
        for ch in chunks {
            assert_eq!(ch.paragraph_count, 1);
            assert!(ch.word_count <= HARD_CAP_WORDS);
        }
    }

    #[test]
    fn giant_single_paragraph_splits_with_space_join_and_preserves_urls() {
        let mut s = String::new();
        for i in 0..45 {
            s.push_str(&format!(
                "Dr. Vance cited https://example.org/docs/v1.2/spec?id={i} in sentence number {i} with 42.5% accuracy. "
            ));
        }
        let segs = segment_document(&sanitize_input(&s));
        assert_eq!(segs.len(), 1);
        let units = build_work_units(&segs, &default_opts());
        assert!(units.len() >= 2);
        for (i, u) in units.iter().enumerate() {
            if let WorkUnit::ParaphraseChunk(c) = u {
                assert!(c.word_count <= HARD_CAP_WORDS);
                if i > 0 {
                    assert_eq!(c.join_with_prev, ChunkJoin::Space);
                }
                assert!(!c.text.ends_with("https://example.org/docs/v1"));
            }
        }
    }

    #[test]
    fn giant_20k_unspaced_token_splits_safely_on_utf8_boundaries() {
        let giant = "é".repeat(10_000); // 20,000 bytes, multi-byte UTF-8
        let segs = segment_document(&sanitize_input(&giant));
        let units = build_work_units(&segs, &default_opts());
        assert!(units.len() >= 10);
        let mut rebuilt = String::new();
        for u in units {
            if let WorkUnit::ParaphraseChunk(c) = u {
                rebuilt.push_str(&c.text);
            }
        }
        assert_eq!(rebuilt, giant);
    }

    #[test]
    fn trailing_sentences_caps_at_two_sentences_and_70_words() {
        let s = "First sentence here. Second sentence is concise. ".repeat(10);
        let t = trailing_sentences(&s, 2);
        assert!(count_words(&t) <= MAX_CONTEXT_WORDS);
        assert_eq!(split_sentences_safe(&t).len(), 2);
    }

    #[test]
    fn unspaced_cjk_splits_within_hard_cap_words_and_reconstructs_losslessly() {
        let cjk = "字".repeat(2_000);
        let segs = segment_document(&sanitize_input(&cjk));
        let units = build_work_units(&segs, &default_opts());
        assert!(units.len() >= 6);
        let mut rebuilt = String::new();
        for u in units {
            if let WorkUnit::ParaphraseChunk(c) = u {
                assert!(
                    c.word_count <= HARD_CAP_WORDS,
                    "CJK chunk exceeded HARD_CAP_WORDS: {}",
                    c.word_count
                );
                rebuilt.push_str(&c.text);
            }
        }
        assert_eq!(rebuilt, cjk);
    }

    #[test]
    fn sentence_splitter_preserves_nobiliary_citations_and_page_abbrevs_without_breaking_p_words() {
        let cited = split_sentences_safe("See (de la Cruz, 2024, p. 12). Next sentence.");
        assert_eq!(
            cited,
            vec!["See (de la Cruz, 2024, p. 12).", "Next sentence."]
        );

        let page_ref = split_sentences_safe("Refer to p. 12 for details. Next sentence.");
        assert_eq!(
            page_ref,
            vec!["Refer to p. 12 for details.", "Next sentence."]
        );

        let ordinary_p_word = split_sentences_safe("He took a nap. Next sentence.");
        assert_eq!(
            ordinary_p_word,
            vec!["He took a nap.", "Next sentence."]
        );
    }

    #[test]
    fn markdown_headings_emit_verbatim_units_and_keep_chunk_indices_contiguous() {
        let doc = "## 1. Introduction and Operational Context\n\nFirst prose paragraph.\n\n### 1.1 Scope\n\nSecond prose paragraph.";
        let segs = segment_document(&sanitize_input(doc));
        let units = build_work_units(&segs, &default_opts());
        assert_eq!(units.len(), 4);
        assert!(matches!(&units[0], WorkUnit::Verbatim { text, join_with_prev: ChunkJoin::First, .. } if text == "## 1. Introduction and Operational Context"));
        assert!(matches!(&units[1], WorkUnit::ParaphraseChunk(c) if c.chunk_index == 0 && c.text == "First prose paragraph." && c.join_with_prev == ChunkJoin::ParagraphBreak));
        assert!(matches!(&units[2], WorkUnit::Verbatim { text, join_with_prev: ChunkJoin::ParagraphBreak, .. } if text == "### 1.1 Scope"));
        assert!(matches!(&units[3], WorkUnit::ParaphraseChunk(c) if c.chunk_index == 1 && c.text == "Second prose paragraph." && c.join_with_prev == ChunkJoin::ParagraphBreak));
    }
}
