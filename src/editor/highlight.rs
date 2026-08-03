//! A lightweight LaTeX tokenizer used for syntax highlighting.
//!
//! This is deliberately small and forgiving: source being typed is often
//! unbalanced, and the tokenizer must never panic or wedge on partial input.

use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Default,
    Comment,
    Command,
    Keyword,
    Math,
    Number,
    Bracket,
}

/// Environments whose body is math.
const MATH_ENVS: &[&str] = &[
    "math",
    "displaymath",
    "equation",
    "equation*",
    "align",
    "align*",
    "aligned",
    "alignat",
    "alignat*",
    "gather",
    "gather*",
    "gathered",
    "multline",
    "multline*",
    "eqnarray",
    "eqnarray*",
    "flalign",
    "flalign*",
    "split",
    "cases",
    "dcases",
    "rcases",
    "matrix",
    "pmatrix",
    "bmatrix",
    "Bmatrix",
    "vmatrix",
    "Vmatrix",
    "smallmatrix",
    "array",
    "subarray",
];

const KEYWORD_COMMANDS: &[&str] = &[
    "documentclass",
    "usepackage",
    "begin",
    "end",
    "section",
    "subsection",
    "subsubsection",
    "paragraph",
    "label",
    "ref",
    "eqref",
    "cite",
    "item",
    "textcolor",
    "newcommand",
    "renewcommand",
    "providecommand",
    "newenvironment",
    "renewenvironment",
    "title",
    "author",
    "date",
    "maketitle",
    "hline",
];

/// Splits `text` into styled spans.
pub fn tokenize(text: &str) -> Vec<(Range<usize>, TokenKind)> {
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut tokens = Vec::new();
    let mut i = 0;

    let mut in_dollar_math = false;
    let mut env_depth = 0usize;

    let math_active = |in_dollar_math: bool, env_depth: usize| in_dollar_math || env_depth > 0;

    while i < n {
        let c = bytes[i];

        if c == b'%' {
            let start = i;
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
            if i < n {
                i += 1; // include the newline in the comment
            }
            tokens.push((start..i, TokenKind::Comment));
            continue;
        }

        if c == b'\\' && i + 1 < n {
            let next = bytes[i + 1];
            if next == b'%' {
                // Escaped percent: not a comment.
                tokens.push((i..i + 2, TokenKind::Default));
                i += 2;
                continue;
            }

            if next.is_ascii_alphabetic() {
                let start = i;
                i += 2;
                while i < n && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                let word = &text[start + 1..i];

                let kind = if KEYWORD_COMMANDS.contains(&word) {
                    TokenKind::Keyword
                } else {
                    TokenKind::Command
                };
                tokens.push((start..i, kind));

                if word == "begin" || word == "end" {
                    // Look ahead for the {env} argument to toggle math mode.
                    let mut j = i;
                    while j < n && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j < n && bytes[j] == b'{' {
                        let env_start = j + 1;
                        let mut k = env_start;
                        while k < n && bytes[k] != b'}' {
                            k += 1;
                        }
                        if k < n {
                            let env = &text[env_start..k];
                            tokens.push((j..k + 1, TokenKind::Bracket));
                            if MATH_ENVS.contains(&env) {
                                if word == "begin" {
                                    env_depth += 1;
                                } else {
                                    env_depth = env_depth.saturating_sub(1);
                                }
                            }
                            i = k + 1;
                        }
                    }
                }
                continue;
            }

            // Symbols like \(, \[, \\, \{, \&, ...
            if next == b'(' || next == b'[' {
                in_dollar_math = !in_dollar_math;
            } else if (next == b')' || next == b']') && in_dollar_math {
                in_dollar_math = false;
            }
            tokens.push((i..i + 2, TokenKind::Command));
            i += 2;
            continue;
        }

        if c == b'$' {
            let start = i;
            if i + 1 < n && bytes[i + 1] == b'$' {
                in_dollar_math = !in_dollar_math;
                i += 2;
            } else {
                in_dollar_math = !in_dollar_math;
                i += 1;
            }
            tokens.push((start..i, TokenKind::Math));
            continue;
        }

        if !math_active(in_dollar_math, env_depth)
            && (c.is_ascii_digit() || (c == b'.' && i + 1 < n && bytes[i + 1].is_ascii_digit()))
        {
            let start = i;
            while i < n && (bytes[i].is_ascii_digit() || bytes[i] == b'.' || bytes[i] == b',') {
                i += 1;
            }
            tokens.push((start..i, TokenKind::Number));
            continue;
        }

        if c == b'{' || c == b'}' || c == b'[' || c == b']' {
            tokens.push((i..i + 1, TokenKind::Bracket));
            i += 1;
            continue;
        }

        // Everything else, including identifiers, spacing, and unicode.
        let start = i;
        let kind = if math_active(in_dollar_math, env_depth) {
            TokenKind::Math
        } else {
            TokenKind::Default
        };
        while i < n {
            let d = bytes[i];
            if d == b'$'
                || d == b'%'
                || d == b'\\'
                || d == b'{'
                || d == b'}'
                || d == b'['
                || d == b']'
            {
                break;
            }
            i += 1;
        }
        if i == start {
            i += 1; // safety: never stall
        }
        tokens.push((start..i, kind));
    }

    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<TokenKind> {
        tokenize(text).into_iter().map(|(_, k)| k).collect()
    }

    fn spans(text: &str) -> Vec<&str> {
        tokenize(text).into_iter().map(|(r, _)| &text[r]).collect()
    }

    #[test]
    fn highlights_commands() {
        assert!(spans(r"\frac{a}{b}").contains(&"\\frac"));
    }

    #[test]
    fn highlights_keywords() {
        let toks = kinds("\\begin{equation}");
        assert!(toks.contains(&TokenKind::Keyword));
        assert!(toks.contains(&TokenKind::Bracket));
    }

    #[test]
    fn comment_runs_to_end_of_line() {
        let text = "% a comment\nx";
        let t = tokenize(text);
        assert_eq!(t[0].1, TokenKind::Comment);
        assert_eq!(&text[t[0].0.clone()], "% a comment\n");
        assert_eq!(&text[t[1].0.clone()], "x");
    }

    #[test]
    fn escaped_percent_is_not_comment() {
        let t = tokenize(r"100\%");
        assert_eq!(t[1].1, TokenKind::Default);
        assert!(!t.iter().any(|(_, k)| *k == TokenKind::Comment));
    }

    #[test]
    fn dollar_math_marks_content() {
        let text = r"$x^2$ text";
        let t = tokenize(text);
        let math: Vec<_> = t
            .iter()
            .filter(|(_, k)| *k == TokenKind::Math)
            .map(|(r, _)| &text[r.clone()])
            .collect();
        assert!(math.contains(&"$"));
        assert!(math.iter().any(|s| s.contains("x^2")));
    }

    #[test]
    fn display_math_pair() {
        let text = r"$$E = mc^2$$";
        let t = tokenize(text);
        assert_eq!(t.first().map(|(_, k)| k), Some(&TokenKind::Math));
        assert!(t.iter().filter(|(_, k)| *k == TokenKind::Math).count() >= 3);
    }

    #[test]
    fn math_env_toggles_math_until_end() {
        let text = r"\begin{align} a+b \end{align} text";
        let toks = tokenize(text);
        // The "a+b" body should be Math, trailing "text" Default.
        let mut saw_math_body = false;
        let mut saw_text_after = false;
        for (r, k) in &toks {
            let s = &text[r.clone()];
            if s.contains("a+b") {
                saw_math_body = *k == TokenKind::Math;
            }
            if s.trim() == "text" {
                saw_text_after = *k == TokenKind::Default;
            }
        }
        assert!(saw_math_body);
        assert!(saw_text_after);
    }

    #[test]
    fn unterminated_math_does_not_panic() {
        let text = "$unterminated";
        let _ = tokenize(text);
    }

    #[test]
    fn empty_and_tiny_inputs_do_not_panic() {
        for text in ["", "\\", "\\frac", "$", "{", "%", "\\begin{align}"] {
            let _ = tokenize(text);
        }
    }

    #[test]
    fn tokens_cover_input_in_order() {
        let text = "\\frac{1}{2} and $x$";
        let t = tokenize(text);
        let mut pos = 0;
        for (r, _) in t {
            assert_eq!(r.start, pos, "gap or overlap in tokens");
            pos = r.end;
        }
        assert_eq!(pos, text.len());
    }
}
