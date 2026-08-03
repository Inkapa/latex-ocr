//! Detection and conversion of math blocks for the inline/display toggle.
//!
//! A math block is a region delimited by `$...$` or `\(...\)` (inline) or
//! `$$...$$`, `\[...\]` or a math environment (display). All offsets are
//! character indices, matching egui's cursor model.

use std::ops::Range;

use super::char_to_byte;

/// The two math styles the editor converts between.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MathMode {
    Inline,
    Display,
}

/// A math region of the document: its content and its full span including
/// delimiters.
#[derive(Debug)]
pub struct MathBlock {
    pub mode: MathMode,
    pub content: Range<usize>,
    pub span: Range<usize>,
    /// The environment name for `\begin{...}` blocks.
    env: Option<String>,
}

impl MathBlock {
    /// True when the cursor at `at` sits inside the content of this block.
    fn contains(&self, at: usize) -> bool {
        self.content.start <= at && at <= self.content.end
    }
}

/// The innermost math block containing the cursor position `at`.
pub fn find_block(text: &str, at: usize) -> Option<MathBlock> {
    scan_blocks(text)
        .into_iter()
        .filter(|b| b.contains(at))
        .min_by_key(|b| b.span.end - b.span.start)
}

/// The result of a toggle: the new cursor position, or a no-op when the
/// target style already applies.
pub enum ToggleOutcome {
    Converted(usize),
    NoChange,
}

/// Switches the math block at `at` to `want`. Returns `None` when there is
/// no math block to convert. When the innermost block already matches the
/// target style but is nested in the opposite style, the enclosing block is
/// converted instead so the result stays coherent.
pub fn toggle(text: &mut String, at: usize, want: MathMode) -> Option<ToggleOutcome> {
    let blocks = scan_blocks(text);
    let inner = blocks
        .iter()
        .filter(|b| b.contains(at))
        .min_by_key(|b| b.span.end - b.span.start)?;
    let target = if inner.mode != want {
        Some(inner)
    } else {
        blocks
            .iter()
            .filter(|b| {
                b.mode != want && b.span.start <= inner.span.start && inner.span.end <= b.span.end
            })
            .min_by_key(|b| b.span.end - b.span.start)
    };
    let Some(block) = target else {
        return Some(ToggleOutcome::NoChange);
    };
    let (replacement, cursor) = convert(text, block, at, want);
    let start_byte = char_to_byte(text, block.span.start);
    let end_byte = char_to_byte(text, block.span.end);
    text.replace_range(start_byte..end_byte, &replacement);
    Some(ToggleOutcome::Converted(cursor))
}

/// Builds the replacement text and the new cursor position, preserving the
/// cursor's offset relative to the math content.
fn convert(text: &str, block: &MathBlock, at: usize, want: MathMode) -> (String, usize) {
    let content = char_slice(text, block.content.clone());
    let rel = at.min(block.content.end) - block.content.start;
    let (replacement, prefix, suffix) = match want {
        MathMode::Inline => {
            let mut body = inline_body(&content, block.env.as_deref());
            if body.is_empty() {
                // An empty `$ $` keeps the block parseable as inline math.
                body = " ".to_string();
            }
            (format!("${body}$"), 1, 1)
        }
        MathMode::Display => (format!("\\[\n{content}\n\\]"), 2, 2),
    };
    let body_len = replacement.chars().count() - prefix - suffix;
    let cursor = block.span.start + prefix + rel.min(body_len);
    (replacement, cursor)
}

/// Prepares the math body for inline style: whitespace collapsed to single
/// spaces, display environments translated to their inline equivalents.
fn inline_body(content: &str, env: Option<&str>) -> String {
    let collapsed: String = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let Some(name) = env else {
        return collapsed;
    };
    let target = match inline_env(name) {
        None => return collapsed,
        Some(target) => target,
    };
    match target {
        None => collapsed,
        Some(new_name) => format!("\\begin{{{new_name}}}{collapsed}\\end{{{new_name}}}"),
    }
}

/// The inline equivalent of a display environment: `None` strips the
/// environment, `Some(name)` renames it, and no entry keeps it as-is.
fn inline_env(name: &str) -> Option<Option<&'static str>> {
    match name {
        "equation" | "equation*" | "displaymath" | "eqnarray" | "eqnarray*" => Some(None),
        "align" | "align*" | "split" => Some(Some("aligned")),
        "alignat" | "alignat*" => Some(Some("alignedat")),
        "gather" | "gather*" | "multline" | "multline*" => Some(Some("gathered")),
        "matrix" => Some(Some("smallmatrix")),
        _ => None,
    }
}

/// Environments whose body is math mode.
const MATH_ENVS: &[&str] = &[
    "align",
    "align*",
    "aligned",
    "alignat",
    "alignat*",
    "alignedat",
    "array",
    "cases",
    "displaymath",
    "eqnarray",
    "eqnarray*",
    "equation",
    "equation*",
    "gather",
    "gather*",
    "gathered",
    "matrix",
    "smallmatrix",
    "pmatrix",
    "bmatrix",
    "vmatrix",
    "Vmatrix",
    "multline",
    "multline*",
    "split",
];

fn is_math_env(name: &str) -> bool {
    MATH_ENVS.contains(&name)
}

/// The environment name of a `\begin{...}` starting at `at`, if any.
fn open_env_name(text: &str, at: usize) -> Option<String> {
    let rest = text.get(at..)?.strip_prefix("\\begin{")?;
    let name = rest.split('}').next()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// The environment name of a `\end{...}` starting at `at`, if any.
fn close_env_name(text: &str, at: usize) -> Option<String> {
    let rest = text.get(at..)?.strip_prefix("\\end{")?;
    let name = rest.split('}').next()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// A math region that has been opened but not yet closed.
enum Frame {
    Dollar {
        open: usize,
        open_len: usize,
    },
    Paren {
        kind: char,
        open: usize,
    },
    Env {
        name: String,
        open: usize,
        open_len: usize,
    },
}

/// Scans the document and returns every closed math block, plus any block
/// left open at the end of the text.
fn scan_blocks(text: &str) -> Vec<MathBlock> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let n = chars.len();
    let mut stack: Vec<Frame> = Vec::new();
    let mut blocks: Vec<MathBlock> = Vec::new();
    let mut i = 0;

    while i < n {
        let c = chars[i].1;
        match stack.last() {
            None => match c {
                '%' => {
                    while i < n && chars[i].1 != '\n' {
                        i += 1;
                    }
                }
                '$' if i + 1 < n && chars[i + 1].1 == '$' => {
                    stack.push(Frame::Dollar {
                        open: i,
                        open_len: 2,
                    });
                    i += 2;
                }
                '$' => {
                    stack.push(Frame::Dollar {
                        open: i,
                        open_len: 1,
                    });
                    i += 1;
                }
                '\\' => {
                    if i + 1 >= n {
                        i += 1;
                    } else {
                        match chars[i + 1].1 {
                            '(' => {
                                stack.push(Frame::Paren { kind: '(', open: i });
                                i += 2;
                            }
                            '[' => {
                                stack.push(Frame::Paren { kind: '[', open: i });
                                i += 2;
                            }
                            'b' | 'e' => {
                                let at = chars[i].0;
                                if let Some(name) = open_env_name(text, at) {
                                    let open_len = 8 + name.len();
                                    let skip = 7 + name.len() + 1;
                                    if is_math_env(&name) {
                                        stack.push(Frame::Env {
                                            name,
                                            open: i,
                                            open_len,
                                        });
                                    }
                                    i += skip;
                                } else if let Some(name) = close_env_name(text, at) {
                                    i += 5 + name.len() + 1;
                                } else {
                                    i += 2;
                                }
                            }
                            _ => i += 2,
                        }
                    }
                }
                _ => i += 1,
            },
            Some(Frame::Dollar { open, open_len }) => {
                if c == '\\' {
                    i = (i + 2).min(n);
                } else if c == '$' {
                    blocks.push(MathBlock {
                        mode: if *open_len == 2 {
                            MathMode::Display
                        } else {
                            MathMode::Inline
                        },
                        content: open + *open_len..i,
                        span: *open..i + 1,
                        env: None,
                    });
                    stack.pop();
                    i += 1;
                } else {
                    i += 1;
                }
            }
            Some(Frame::Paren { kind, open }) => {
                if c == '\\' {
                    let closing = i + 1 < n
                        && ((*kind == '(' && chars[i + 1].1 == ')')
                            || (*kind == '[' && chars[i + 1].1 == ']'));
                    if closing {
                        blocks.push(MathBlock {
                            mode: if *kind == '(' {
                                MathMode::Inline
                            } else {
                                MathMode::Display
                            },
                            content: open + 2..i,
                            span: *open..i + 2,
                            env: None,
                        });
                        stack.pop();
                    }
                    i = (i + 2).min(n);
                } else {
                    i += 1;
                }
            }
            Some(Frame::Env {
                name,
                open,
                open_len,
            }) => {
                if c == '\\' {
                    if let Some(close_name) = close_env_name(text, chars[i].0) {
                        if close_name == *name {
                            blocks.push(MathBlock {
                                mode: MathMode::Display,
                                content: open + *open_len..i,
                                span: *open..i + 5 + close_name.len() + 1,
                                env: Some(name.clone()),
                            });
                            stack.pop();
                        }
                        i += 5 + close_name.len() + 1;
                    } else if let Some(open_name) = open_env_name(text, chars[i].0) {
                        let open_len = 8 + open_name.len();
                        let skip = 7 + open_name.len() + 1;
                        if is_math_env(&open_name) {
                            stack.push(Frame::Env {
                                name: open_name,
                                open: i,
                                open_len,
                            });
                        }
                        i += skip;
                    } else {
                        i = (i + 2).min(n);
                    }
                } else {
                    i += 1;
                }
            }
        }
    }

    for frame in stack {
        blocks.push(match frame {
            Frame::Dollar { open, open_len } => MathBlock {
                mode: if open_len == 2 {
                    MathMode::Display
                } else {
                    MathMode::Inline
                },
                content: open + open_len..n,
                span: open..n,
                env: None,
            },
            Frame::Paren { kind, open } => MathBlock {
                mode: if kind == '(' {
                    MathMode::Inline
                } else {
                    MathMode::Display
                },
                content: open + 2..n,
                span: open..n,
                env: None,
            },
            Frame::Env {
                name,
                open,
                open_len,
            } => MathBlock {
                mode: MathMode::Display,
                content: open + open_len..n,
                span: open..n,
                env: Some(name),
            },
        });
    }
    blocks
}

fn char_slice(text: &str, range: Range<usize>) -> String {
    text.chars()
        .skip(range.start)
        .take(range.end.saturating_sub(range.start))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toggled(text: &str, at: usize, want: MathMode) -> (String, ToggleOutcome) {
        let mut t = text.to_string();
        let outcome = toggle(&mut t, at, want).unwrap_or(ToggleOutcome::NoChange);
        (t, outcome)
    }

    #[test]
    fn finds_inline_and_display_dollar_blocks() {
        let block = find_block("$x$", 1).unwrap();
        assert_eq!(block.mode, MathMode::Inline);
        assert_eq!(block.content, 1..2);
        let block = find_block("$$x$$", 2).unwrap();
        assert_eq!(block.mode, MathMode::Display);
        assert_eq!(block.content, 2..3);
    }

    #[test]
    fn finds_paren_and_bracket_blocks() {
        assert_eq!(find_block(r"\(x\)", 2).unwrap().mode, MathMode::Inline);
        assert_eq!(find_block(r"\[x\]", 2).unwrap().mode, MathMode::Display);
    }

    #[test]
    fn ignores_escaped_dollars_and_line_breaks() {
        assert!(find_block(r"a \$ b $x$", 8).is_some());
        assert!(find_block(r"a \$ b", 4).is_none());
        assert!(find_block(r"\\[x]", 3).is_none());
    }

    #[test]
    fn finds_environment_blocks() {
        let block = find_block(r"\begin{align}x\end{align}", 14).unwrap();
        assert_eq!(block.mode, MathMode::Display);
        assert_eq!(block.content, 13..14);
    }

    #[test]
    fn skips_comments() {
        assert!(find_block("% $x$", 4).is_none());
        assert!(find_block("% $x$\n$y$", 8).is_some());
    }

    #[test]
    fn no_block_between_two_inline_math_regions() {
        assert!(find_block("$a$ b $c$", 4).is_none());
        assert!(toggle(&mut "$a$ b $c$".to_string(), 4, MathMode::Display).is_none());
    }

    #[test]
    fn unclosed_blocks_are_usable() {
        let block = find_block("$x", 1).unwrap();
        assert_eq!(block.mode, MathMode::Inline);
        assert_eq!(block.content, 1..2);
        assert!(find_block(r"\begin{align}x", 14).is_some());
    }

    #[test]
    fn converts_inline_to_display() {
        let (t, outcome) = toggled("$x$", 1, MathMode::Display);
        assert!(matches!(outcome, ToggleOutcome::Converted(_)));
        assert_eq!(t, "\\[\nx\n\\]");
    }

    #[test]
    fn converts_display_to_inline_and_collapses_whitespace() {
        let (t, outcome) = toggled("\\[\n a \n b \n\\]", 3, MathMode::Inline);
        assert!(matches!(outcome, ToggleOutcome::Converted(_)));
        assert_eq!(t, "$a b$");
    }

    #[test]
    fn round_trip_preserves_content() {
        let (t, _) = toggled("$x$", 1, MathMode::Display);
        let (t, _) = toggled(&t, 3, MathMode::Inline);
        assert_eq!(t, "$x$");
    }

    #[test]
    fn converts_align_environment_to_aligned() {
        let (t, _) = toggled(r"\begin{align}a\\b\end{align}", 13, MathMode::Inline);
        assert_eq!(t, r"$\begin{aligned}a\\b\end{aligned}$");
    }

    #[test]
    fn strips_equation_environment() {
        let (t, _) = toggled(r"\begin{equation}x\end{equation}", 16, MathMode::Inline);
        assert_eq!(t, "$x$");
    }

    #[test]
    fn converts_outer_block_when_already_inline() {
        let (t, outcome) = toggled(r"$a\begin{aligned}b\end{aligned}c$", 16, MathMode::Display);
        assert!(matches!(outcome, ToggleOutcome::Converted(_)));
        assert_eq!(t, "\\[\na\\begin{aligned}b\\end{aligned}c\n\\]");
    }

    #[test]
    fn no_change_when_mode_matches() {
        let (t, outcome) = toggled("$x$", 1, MathMode::Inline);
        assert!(matches!(outcome, ToggleOutcome::NoChange));
        assert_eq!(t, "$x$");
    }

    #[test]
    fn cursor_lands_inside_new_delimiters() {
        let (_, outcome) = toggled("$xy$", 2, MathMode::Display);
        match outcome {
            ToggleOutcome::Converted(cursor) => assert_eq!(cursor, 3),
            _ => panic!("expected a conversion"),
        }
    }
}
