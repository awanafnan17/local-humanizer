use crate::engine::chunker::ChunkJoin;

#[derive(Debug, Default)]
pub struct Reconstructor {
    output: String,
    warnings: Vec<String>,
}

impl Reconstructor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_piece(&mut self, text: &str, join: ChunkJoin) {
        let piece = text.trim_matches('\n');
        if piece.trim().is_empty() {
            return;
        }
        if !self.output.is_empty() {
            match join {
                ChunkJoin::First => {}
                ChunkJoin::ParagraphBreak => {
                    self.output.push_str("\n\n");
                    self.output.push_str(piece);
                    return;
                }
                ChunkJoin::Space => {
                    if !self.output.ends_with(char::is_whitespace) {
                        self.output.push(' ');
                    }
                    self.output.push_str(piece.trim_start());
                    return;
                }
                ChunkJoin::Exact => {}
            }
        }
        self.output.push_str(piece);
    }

    pub fn push_warnings(&mut self, new_warnings: Vec<String>) {
        self.warnings.extend(new_warnings);
    }

    pub fn partial_output(&self) -> String {
        self.output.clone()
    }

    pub fn finish(self) -> (String, Vec<String>) {
        (self.output, self.warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconstructor_joins_paragraphs_spaces_and_exact_splits_cleanly() {
        let mut rec = Reconstructor::new();
        rec.push_piece("First half of para one.", ChunkJoin::First);
        rec.push_piece("Second half of para one.", ChunkJoin::Space);
        rec.push_piece("```rust\nfn x() {}\n```", ChunkJoin::ParagraphBreak);
        rec.push_piece("PartA", ChunkJoin::ParagraphBreak);
        rec.push_piece("PartB", ChunkJoin::Exact);
        let (out, warnings) = rec.finish();
        assert_eq!(
            out,
            "First half of para one. Second half of para one.\n\n```rust\nfn x() {}\n```\n\nPartAPartB"
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn reconstructor_preserves_order_and_paragraph_count_across_3000_word_doc() {
        let mut rec = Reconstructor::new();
        for idx in 0..40 {
            let join = if idx == 0 {
                ChunkJoin::First
            } else if idx % 10 == 5 {
                ChunkJoin::Space
            } else {
                ChunkJoin::ParagraphBreak
            };
            let words = (0..80)
                .map(|w| format!("p{idx}w{w}"))
                .collect::<Vec<_>>()
                .join(" ");
            rec.push_piece(&words, join);
        }
        let (out, _) = rec.finish();
        let paras: Vec<&str> = out.split("\n\n").collect();
        // 40 pieces with 4 Space joins => 36 paragraphs
        assert_eq!(paras.len(), 36);
        assert!(out.starts_with("p0w0 "));
        assert!(out.contains("p39w79"));
    }
}
