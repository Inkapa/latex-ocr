//! Turns loose LaTeX snippets into compilable documents.

/// Standard preamble used for standalone snippets. Kept deliberately small so
/// previews stay fast while covering the most common math and layout needs.
///
/// `article` is used instead of `standalone` because the latter does not play
/// well with numbered math environments under tectonic's xdvipdfmx. Pages are
/// trimmed of their white margins after rasterization instead.
pub const PREAMBLE: &str = r#"\documentclass{article}
\usepackage{amsmath}
\usepackage{amssymb}
\usepackage{mathtools}
\pagestyle{empty}
"#;

/// Wraps a snippet that is already a full document, or returns it unchanged.
///
/// Anything that declares its own preamble (via `\documentclass` or
/// `\begin{document}`) is treated as a complete document.
pub fn wrap_source(source: &str) -> String {
    if looks_like_document(source) {
        source.to_string()
    } else {
        format!(
            "{PREAMBLE}\n\\begin{{document}}\n{}\n\\end{{document}}\n",
            source
        )
    }
}

/// True when `source` already contains a preamble of its own.
pub fn looks_like_document(source: &str) -> bool {
    let head = source
        .lines()
        .take_while(|line| {
            let t = line.trim();
            t.is_empty() || t.starts_with('%') || t.starts_with("\\documentclass")
        })
        .collect::<Vec<_>>()
        .join("\n");
    head.contains("\\documentclass")
        || source.contains("\\begin{document}")
        || source.contains("\\begin{frame}")
}

/// True when `source` has at least one line of actual content, ignoring blank
/// lines and `%` comments.
pub fn has_content(source: &str) -> bool {
    source.lines().any(|line| {
        let trimmed = line.trim();
        !trimmed.is_empty() && !trimmed.starts_with('%')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_plain_snippet() {
        let out = wrap_source("hello $x$");
        assert!(out.contains("\\documentclass"));
        assert!(out.contains("hello $x$"));
        assert!(out.contains("\\begin{document}"));
    }

    #[test]
    fn wraps_math_snippet() {
        let out = wrap_source(r"\int_0^1 x^2\,dx");
        assert!(out.contains("\\int_0^1 x^2\\,dx"));
    }

    #[test]
    fn leaves_full_document_alone() {
        let doc = r"\documentclass{article}
\begin{document}
content
\end{document}";
        assert_eq!(wrap_source(doc), doc);
    }

    #[test]
    fn detects_document_with_leading_comments() {
        let doc = "% note\n\\documentclass{article}\n\\begin{document}x\\end{document}";
        assert!(looks_like_document(doc));
        assert_eq!(wrap_source(doc), doc);
    }

    #[test]
    fn detects_begin_document_without_class() {
        let doc = "\\begin{document}raw\\end{document}";
        assert!(looks_like_document(doc));
        assert_eq!(wrap_source(doc), doc);
    }

    #[test]
    fn content_detection_ignores_comments() {
        assert!(!has_content(""));
        assert!(!has_content("   \n  \t \n"));
        assert!(!has_content("% LaTeX OCR\n\n% another comment\n"));
        assert!(has_content("hello"));
        assert!(has_content("  x "));
        assert!(has_content("$"));
        assert!(has_content("% comment\n\\alpha"));
    }
}
