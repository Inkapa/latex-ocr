//! Tokenizer used by the local OCR backend.
//!
//! The upstream pix2tex tokenizer is a byte-level BPE whose `tokenizer.json`
//! declares no decoder, so decoding is a plain concatenation of the vocabulary
//! entries for each token id. `decode_latex` then applies the same cleanups as
//! pix2tex's `token2str`: literal spaces are dropped, the byte-level space
//! glyph (U+0120) becomes a real space, and the special tokens are removed.

use std::collections::HashMap;

use serde::Deserialize;

/// Decoder for the pix2tex vocabulary, built from the bundled tokenizer file.
pub struct Tokenizer {
    by_id: HashMap<usize, String>,
}

#[derive(Deserialize)]
struct TokenizerFile {
    model: Model,
}

#[derive(Deserialize)]
struct Model {
    vocab: HashMap<String, usize>,
}

const TOKENIZER_JSON: &[u8] = include_bytes!("../../assets/tokenizer.json");

impl Tokenizer {
    /// Loads the tokenizer that ships with the binary.
    pub fn bundled() -> Self {
        let file: TokenizerFile =
            serde_json::from_slice(TOKENIZER_JSON).expect("bundled tokenizer is valid JSON");
        let by_id = file
            .model
            .vocab
            .into_iter()
            .map(|(token, id)| (id, token))
            .collect();
        Self { by_id }
    }

    /// Converts a generated token stream into LaTeX source.
    pub fn decode_latex(&self, ids: &[i64]) -> String {
        let mut raw = String::new();
        for &id in ids {
            if id >= 0
                && let Some(token) = self.by_id.get(&(id as usize))
            {
                raw.push_str(token);
            }
        }
        let mut out = raw.replace(' ', "").replace('\u{0120}', " ");
        for special in ["[EOS]", "[BOS]", "[PAD]"] {
            out = out.replace(special, "");
        }
        out.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_simple_expression() {
        let tok = Tokenizer::bundled();
        // [BOS] x ^ { 2 } Ġ+ Ġy [EOS]
        let ids = [1, 89, 63, 92, 19, 94, 118, 197, 2];
        assert_eq!(tok.decode_latex(&ids), "x^{2} + y");
    }

    #[test]
    fn decodes_spaced_tokens() {
        let tok = Tokenizer::bundled();
        // [BOS] Ġa Ġ+ Ġb [EOS]
        let ids = [1, 141, 118, 180, 2];
        assert_eq!(tok.decode_latex(&ids), "a + b");
    }

    #[test]
    fn decodes_commands() {
        let tok = Tokenizer::bundled();
        // [BOS] \ frac { 1 } { 2 } [EOS]
        let ids = [1, 61, 117, 92, 18, 94, 92, 19, 94, 2];
        assert_eq!(tok.decode_latex(&ids), "\\frac{1}{2}");
    }

    #[test]
    fn removes_padding_and_special_tokens() {
        let tok = Tokenizer::bundled();
        // [BOS] Ġ [PAD] [PAD] [EOS]
        let ids = [1, 101, 0, 0, 2];
        assert_eq!(tok.decode_latex(&ids), "");
    }

    #[test]
    fn trims_and_strips_leading_space_glyph() {
        let tok = Tokenizer::bundled();
        // [BOS] Ġx Ġ= Ġ5 [EOS]
        let ids = [1, 128, 112, 256, 2];
        assert_eq!(tok.decode_latex(&ids), "x = 5");
    }
}
