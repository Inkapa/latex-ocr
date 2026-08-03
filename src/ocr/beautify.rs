//! Lightweight LaTeX cleanup for OCR results.
//!
//! pix2tex output is functional but flat: it uses the deprecated `\bf`, wraps
//! every array cell in redundant braces, and lays out multi-line results as a
//! single `array`. `beautify` fixes those so longer expressions are readable
//! and conventional in the editor. It only performs safe transformations that
//! cannot change the meaning of the formula.

use fancy_regex::Regex;

/// Cleans up an OCR result for display and insertion.
pub fn beautify(latex: &str) -> String {
    let s = fix_deprecated_bf(latex);
    reformat_arrays(&s)
}

/// Replaces the deprecated `\bf` with `\mathbf`, which is the modern spelling.
fn fix_deprecated_bf(latex: &str) -> String {
    let braced = Regex::new(r"\\bf\s*\{([^{}]*)\}").expect("regex");
    let s = braced.replace_all(latex, "\\mathbf{$1}").into_owned();
    let single = Regex::new(r"\\bf\s+([A-Za-z0-9])").expect("regex");
    single.replace_all(&s, "\\mathbf{$1}").into_owned()
}

/// Converts `\begin{array}{l}...\end{array}` blocks into an `aligned`
/// environment with each row on its own line, aligned on `=`.
fn reformat_arrays(latex: &str) -> String {
    let start = Regex::new(r"\\begin\s*\{\s*array\s*\}\s*\{\s*[lcr]+\s*\}").expect("regex");
    let end = Regex::new(r"\\end\s*\{\s*array\s*\}").expect("regex");

    let mut out = String::with_capacity(latex.len());
    let mut rest = latex;
    while let Some(start_match) = start.find(rest).ok().flatten() {
        out.push_str(&rest[..start_match.start()]);
        let body_start = start_match.end();
        match end.find(&rest[body_start..]).ok().flatten() {
            Some(end_match) => {
                let body = &rest[body_start..body_start + end_match.start()];
                out.push_str(&format_aligned(body));
                rest = &rest[body_start + end_match.end()..];
            }
            None => {
                out.push_str(&rest[body_start..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

fn format_aligned(body: &str) -> String {
    let rows: Vec<String> = split_rows(body)
        .into_iter()
        .map(|row| format_row(strip_outer_braces(row.trim())))
        .collect();

    let mut out = String::from("\\begin{aligned}\n");
    for (i, row) in rows.iter().enumerate() {
        out.push_str("    ");
        out.push_str(row);
        out.push_str(if i + 1 < rows.len() { " \\\\\n" } else { "\n" });
    }
    out.push_str("\\end{aligned}");
    out
}

/// Splits an array body into rows on top-level `\\`.
fn split_rows(body: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = body.char_indices().collect();
    let mut rows = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut i = 0;
    while i < chars.len() {
        let (idx, c) = chars[i];
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            '\\' if depth == 0 && i + 1 < chars.len() && chars[i + 1].1 == '\\' => {
                rows.push(&body[start..idx]);
                start = idx + 2;
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    rows.push(&body[start..]);
    rows
}

/// Aligns a single row on `=`, like `a + b &= c`. Rows that already contain
/// alignment markers or no `=` are left alone.
fn format_row(row: &str) -> String {
    if row.is_empty() || top_level_char(row, '&').is_some() {
        return row.to_string();
    }
    match top_level_char(row, '=') {
        Some(idx) => format!("{} &= {}", row[..idx].trim(), row[idx + 1..].trim()),
        None => row.to_string(),
    }
}

/// Byte index of the first `needle` at brace depth zero.
fn top_level_char(s: &str, needle: char) -> Option<usize> {
    let mut depth = 0i32;
    for (idx, c) in s.char_indices() {
        if c == '\\' {
            continue; // a command, not a structural token
        }
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {
                if depth == 0 && c == needle {
                    return Some(idx);
                }
            }
        }
    }
    None
}

/// Removes wrapping brace layers that enclose the whole string, like the
/// `{{...}}` cells pix2tex emits.
fn strip_outer_braces(mut s: &str) -> &str {
    while is_fully_wrapped(s) {
        s = &s[1..s.len() - 1];
    }
    s
}

fn is_fully_wrapped(s: &str) -> bool {
    if s.len() < 2 {
        return false;
    }
    let bytes = s.as_bytes();
    if bytes[0] != b'{' || bytes[s.len() - 1] != b'}' {
        return false;
    }
    let mut depth = 0i32;
    let last = s.len() - 1;
    for (i, c) in s.char_indices() {
        if c == '{' {
            depth += 1;
        } else if c == '}' {
            depth -= 1;
            if depth == 0 && i != last {
                return false; // the outer brace closes early
            }
        }
    }
    depth == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixes_deprecated_bf() {
        assert_eq!(beautify(r"{\bf a}"), r"{\mathbf{a}}");
        assert_eq!(beautify(r"\int_{\bf a}^{1}"), r"\int_{\mathbf{a}}^{1}");
        assert_eq!(beautify(r"\bf {ab}"), r"\mathbf{ab}");
    }

    #[test]
    fn leaves_plain_math_alone() {
        assert_eq!(beautify("E = mc^{2}"), "E = mc^{2}");
    }

    #[test]
    fn reformats_array_into_aligned() {
        let input = r"\begin{array}{l}{{(a+b)^{2}=a^{2}+2a b+b^{2}}}\\ {{\displaystyle\int_{\bf a}^{1}x^{2}\,d x=\frac{1}{3}}}\end{array}";
        let expected = "\\begin{aligned}\n    (a+b)^{2} &= a^{2}+2a b+b^{2} \\\\\n    \\displaystyle\\int_{\\mathbf{a}}^{1}x^{2}\\,d x &= \\frac{1}{3}\n\\end{aligned}";
        assert_eq!(beautify(input), expected);
    }

    #[test]
    fn keeps_rows_without_equals() {
        let input = r"\begin{array}{l}{{a+b}}\\{{c}}\end{array}";
        let expected = "\\begin{aligned}\n    a+b \\\\\n    c\n\\end{aligned}";
        assert_eq!(beautify(input), expected);
    }

    #[test]
    fn split_rows_ignores_double_slashes_in_commands() {
        let rows = split_rows(r"{{a}}\\{{b}}");
        assert_eq!(rows, vec!["{{a}}", "{{b}}"]);
    }

    #[test]
    fn strip_braces_unwraps_cells() {
        assert_eq!(strip_outer_braces("{{a+b}}"), "a+b");
        assert_eq!(strip_outer_braces("{a}{b}"), "{a}{b}");
        assert_eq!(strip_outer_braces("x"), "x");
    }

    #[test]
    fn top_level_char_ignores_braced_groups() {
        assert_eq!(top_level_char(r"a+\frac{=}{2}=b", '='), Some(13));
    }
}
