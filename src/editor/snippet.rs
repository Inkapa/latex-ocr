//! Snippet palette and text-insertion helpers for the editor.
//!
//! Templates use two markers: `@s` expands to the current selection (or
//! nothing when there is no selection) and `@|` marks the final cursor
//! position. All offsets are character indices, matching egui's cursor.

use std::ops::Range;

pub struct Snippet {
    pub label: &'static str,
    pub template: &'static str,
}

pub const INLINE_MATH: &Snippet = &Snippet {
    label: "Inline math",
    template: "$@s@|$",
};

pub const DISPLAY_MATH: &Snippet = &Snippet {
    label: "Display math",
    template: "$$@s@|$$",
};

pub const TEXT_TOOLS: &[Snippet] = &[
    Snippet {
        label: "Bold",
        template: "\\textbf{@s@|}",
    },
    Snippet {
        label: "Italic",
        template: "\\textit{@s@|}",
    },
    Snippet {
        label: "Roman",
        template: "\\textrm{@s@|}",
    },
    Snippet {
        label: "Sans",
        template: "\\textsf{@s@|}",
    },
    Snippet {
        label: "Code",
        template: "\\texttt{@s@|}",
    },
    Snippet {
        label: "Quote",
        template: "``@s@|''",
    },
];

pub const STRUCTURE: &[Snippet] = &[
    Snippet {
        label: "Equation",
        template: "\\begin{equation}\n  @s@|\n\\end{equation}",
    },
    Snippet {
        label: "Align",
        template: "\\begin{align}\n  @s@|\n\\end{align}",
    },
    Snippet {
        label: "Gather",
        template: "\\begin{gather}\n  @s@|\n\\end{gather}",
    },
    Snippet {
        label: "Cases",
        template: "\\begin{cases}\n  @s@|\n\\end{cases}",
    },
    Snippet {
        label: "Matrix (p)",
        template: "\\begin{pmatrix}\n  @s@|\n\\end{pmatrix}",
    },
    Snippet {
        label: "Matrix (b)",
        template: "\\begin{bmatrix}\n  @s@|\n\\end{bmatrix}",
    },
    Snippet {
        label: "Matrix (v)",
        template: "\\begin{vmatrix}\n  @s@|\n\\end{vmatrix}",
    },
    Snippet {
        label: "Itemize",
        template: "\\begin{itemize}\n  \\item @|@\n\\end{itemize}",
    },
    Snippet {
        label: "Enumerate",
        template: "\\begin{enumerate}\n  \\item @|@\n\\end{enumerate}",
    },
    Snippet {
        label: "Figure",
        template: "\\begin{figure}\n  \\centering\n  \\includegraphics{@|@}\n  \\caption{}\n  \\label{fig:}\n\\end{figure}",
    },
    Snippet {
        label: "Table",
        template: "\\begin{table}\n  \\centering\n  \\begin{tabular}{cc}\n    @s@|\n  \\end{tabular}\n  \\caption{}\n  \\label{tab:}\n\\end{table}",
    },
    Snippet {
        label: "Theorem",
        template: "\\begin{theorem}\n  @s@|\n\\end{theorem}",
    },
    Snippet {
        label: "Proof",
        template: "\\begin{proof}\n  @s@|\n\\end{proof}",
    },
];

pub const MATH: &[Snippet] = &[
    Snippet {
        label: "Fraction",
        template: "\\frac{@s@|}{}",
    },
    Snippet {
        label: "Square root",
        template: "\\sqrt{@s@|}",
    },
    Snippet {
        label: "nth root",
        template: "\\sqrt[@|@]{@s@}",
    },
    Snippet {
        label: "Subscript",
        template: "@s@_{@|@}",
    },
    Snippet {
        label: "Superscript",
        template: "@s@^{@|@}",
    },
    Snippet {
        label: "Sum",
        template: "\\sum_{@|@}^{}",
    },
    Snippet {
        label: "Product",
        template: "\\prod_{@|@}^{}",
    },
    Snippet {
        label: "Integral",
        template: "\\int_{@|@}^{}",
    },
    Snippet {
        label: "Partial",
        template: "\\frac{\\partial @s@}{\\partial @|@}",
    },
    Snippet {
        label: "Limit",
        template: "\\lim_{@|@ \\to \\infty}",
    },
    Snippet {
        label: "Vector",
        template: "\\vec{@s@|}",
    },
    Snippet {
        label: "Hat",
        template: "\\hat{@s@|}",
    },
    Snippet {
        label: "Bar",
        template: "\\bar{@s@|}",
    },
    Snippet {
        label: "Binomial",
        template: "\\binom{@|@}{@s@}",
    },
];

pub const GREEK: &[Snippet] = &[
    Snippet {
        label: "alpha",
        template: "\\alpha",
    },
    Snippet {
        label: "beta",
        template: "\\beta",
    },
    Snippet {
        label: "gamma",
        template: "\\gamma",
    },
    Snippet {
        label: "Gamma",
        template: "\\Gamma",
    },
    Snippet {
        label: "delta",
        template: "\\delta",
    },
    Snippet {
        label: "Delta",
        template: "\\Delta",
    },
    Snippet {
        label: "epsilon",
        template: "\\varepsilon",
    },
    Snippet {
        label: "zeta",
        template: "\\zeta",
    },
    Snippet {
        label: "eta",
        template: "\\eta",
    },
    Snippet {
        label: "theta",
        template: "\\theta",
    },
    Snippet {
        label: "Theta",
        template: "\\Theta",
    },
    Snippet {
        label: "lambda",
        template: "\\lambda",
    },
    Snippet {
        label: "Lambda",
        template: "\\Lambda",
    },
    Snippet {
        label: "mu",
        template: "\\mu",
    },
    Snippet {
        label: "nu",
        template: "\\nu",
    },
    Snippet {
        label: "xi",
        template: "\\xi",
    },
    Snippet {
        label: "Xi",
        template: "\\Xi",
    },
    Snippet {
        label: "pi",
        template: "\\pi",
    },
    Snippet {
        label: "Pi",
        template: "\\Pi",
    },
    Snippet {
        label: "rho",
        template: "\\rho",
    },
    Snippet {
        label: "sigma",
        template: "\\sigma",
    },
    Snippet {
        label: "Sigma",
        template: "\\Sigma",
    },
    Snippet {
        label: "tau",
        template: "\\tau",
    },
    Snippet {
        label: "phi",
        template: "\\phi",
    },
    Snippet {
        label: "varphi",
        template: "\\varphi",
    },
    Snippet {
        label: "Phi",
        template: "\\Phi",
    },
    Snippet {
        label: "chi",
        template: "\\chi",
    },
    Snippet {
        label: "psi",
        template: "\\psi",
    },
    Snippet {
        label: "Psi",
        template: "\\Psi",
    },
    Snippet {
        label: "omega",
        template: "\\omega",
    },
    Snippet {
        label: "Omega",
        template: "\\Omega",
    },
];

/// True when the text before the cursor is already inside math mode, so math
/// snippets can be inserted raw instead of being wrapped again.
pub fn in_math_context(before: &str) -> bool {
    let at = before.chars().count();
    crate::editor::math::find_block(before, at).is_some()
}

/// Applies a snippet template to `text`, replacing `selection` (if any).
/// Selection is given in character indices, as used by egui. Returns the new
/// cursor position and the new selection (usually `None`).
pub fn apply(
    text: &mut String,
    selection: Option<Range<usize>>,
    template: &str,
) -> (usize, Option<Range<usize>>) {
    let selected: String = selection
        .as_ref()
        .map(|r| text.chars().skip(r.start).take(r.end - r.start).collect())
        .unwrap_or_default();

    let mut expanded = String::new();
    let mut cursor = None;
    let mut rest = template;

    while let Some(idx) = rest.find('@') {
        expanded.push_str(&rest[..idx]);
        rest = &rest[idx..];
        match rest.chars().nth(1) {
            Some('s') => {
                expanded.push_str(&selected);
                rest = &rest[2..];
            }
            Some('|') => {
                cursor = Some(expanded.chars().count());
                rest = &rest[2..];
            }
            _ => {
                expanded.push('@');
                rest = &rest[1..];
            }
        }
    }
    expanded.push_str(rest);

    let start = selection
        .as_ref()
        .map(|r| r.start)
        .unwrap_or_else(|| text.chars().count());
    let end = selection.map(|r| r.end).unwrap_or(start);
    let start_byte = char_to_byte(text, start);
    let end_byte = char_to_byte(text, end);
    text.replace_range(start_byte..end_byte, &expanded);

    let cursor = cursor
        .map(|c| start + c)
        .unwrap_or(start + expanded.chars().count());
    (cursor, None)
}

fn char_to_byte(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(byte, _)| byte)
        .unwrap_or(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_plain_snippet_at_cursor() {
        let mut text = "ab".to_string();
        let (cur, _) = apply(&mut text, None, "X@|Y");
        assert_eq!(text, "abXY");
        assert_eq!(cur, 3);
    }

    #[test]
    fn replaces_selection() {
        let mut text = "abc".to_string();
        let (cur, sel) = apply(&mut text, Some(1..2), "(@s)@|");
        assert_eq!(text, "a(b)c");
        assert_eq!(cur, 4);
        assert!(sel.is_none());
    }

    #[test]
    fn fraction_snippet_replaces_selection() {
        let mut text = "x".to_string();
        let (cur, _) = apply(&mut text, Some(0..1), "\\frac{@s@|}{}");
        assert_eq!(text, "\\frac{x}{}");
        assert_eq!(cur, "\\frac{x".chars().count());
    }

    #[test]
    fn inline_math_uses_selection_when_present() {
        let mut text = "x^2".to_string();
        let (cur, _) = apply(&mut text, Some(0..3), "$@s@|$");
        assert_eq!(text, "$x^2$");
        assert_eq!(cur, 4);
    }

    #[test]
    fn inline_math_without_selection_places_cursor_inside() {
        let mut text = "abc".to_string();
        let (cur, _) = apply(&mut text, None, "$@s@|$");
        assert_eq!(text, "abc$$");
        assert_eq!(cur, 4);
    }

    #[test]
    fn display_math_wraps_in_double_dollars() {
        let mut text = "e=mc^2".to_string();
        let (cur, _) = apply(&mut text, Some(0..6), "$$@s@|$$");
        assert_eq!(text, "$$e=mc^2$$");
        assert_eq!(cur, 8);
    }

    #[test]
    fn snippet_without_cursor_marker_ends_at_end() {
        let mut text = "a".to_string();
        let (cur, _) = apply(&mut text, None, "\\alpha");
        assert_eq!(text, "a\\alpha");
        assert_eq!(cur, 7);
    }

    #[test]
    fn handles_at_sign_in_template_literally() {
        let mut text = "".to_string();
        let (_, _) = apply(&mut text, None, "@");
        assert_eq!(text, "@");
    }

    #[test]
    fn multibyte_text_offsets_are_character_based() {
        let mut text = "αβ".to_string();
        let (cur, _) = apply(&mut text, Some(1..2), "@s^2@|");
        assert_eq!(text, "αβ^2");
        assert_eq!(text.chars().count(), 4);
        assert_eq!(cur, 4);
    }

    #[test]
    fn structure_snippet_keeps_selection_in_place() {
        let mut text = "a=b".to_string();
        let (cur, _) = apply(
            &mut text,
            Some(0..3),
            "\\begin{align}\n  @s@|\n\\end{align}",
        );
        assert_eq!(text, "\\begin{align}\n  a=b\n\\end{align}");
        assert_eq!(cur, 19);
    }

    #[test]
    fn greek_snippets_have_no_markers() {
        for s in GREEK {
            assert!(!s.template.contains("@"), "{}", s.label);
            assert!(s.template.starts_with('\\'));
        }
    }

    #[test]
    fn all_markers_are_consumed() {
        let mut text = String::new();
        for s in TEXT_TOOLS.iter().chain(MATH.iter()).chain(STRUCTURE.iter()) {
            let _ = apply(&mut text, None, s.template);
        }
    }

    #[test]
    fn detects_open_math_context() {
        assert!(in_math_context("$x"));
        assert!(in_math_context("\\begin{align}\nx &="));
        assert!(!in_math_context(""));
        assert!(!in_math_context("$x$"));
        assert!(!in_math_context("\\begin{align}\nx &= y \\end{align}\n"));
        assert!(!in_math_context(r"the \$ price"));
    }
}
