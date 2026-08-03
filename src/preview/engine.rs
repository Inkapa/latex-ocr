use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A failed LaTeX compilation, with an optional transcript tail for the user.
#[derive(Debug)]
pub struct EngineError {
    pub message: String,
    pub log: Option<String>,
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for EngineError {}

/// Compiles LaTeX source into PDF files.
pub trait TexEngine: Send + Sync {
    /// Compile `source`, writing any outputs into `out_dir`. Returns the paths
    /// of the produced PDF files (normally exactly one).
    fn compile(&self, source: &str, out_dir: &Path) -> Result<Vec<PathBuf>, EngineError>;
}

/// A `TexEngine` backed by the Tectonic command-line tool.
pub struct TectonicCli {
    pub binary: PathBuf,
}

const MAX_LOG_LINES: usize = 40;

impl TexEngine for TectonicCli {
    fn compile(&self, source: &str, out_dir: &Path) -> Result<Vec<PathBuf>, EngineError> {
        if !self.binary.exists() {
            return Err(EngineError {
                message: format!("tectonic not found at {}", self.binary.display()),
                log: None,
            });
        }

        let tex_file = out_dir.join("doc.tex");
        fs::write(&tex_file, source).map_err(|e| EngineError {
            message: format!("cannot write source file: {e}"),
            log: None,
        })?;

        let output = Command::new(&self.binary)
            .arg("--outdir")
            .arg(out_dir)
            .arg("--keep-logs")
            .arg(&tex_file)
            .current_dir(out_dir)
            .output()
            .map_err(|e| EngineError {
                message: format!("failed to run tectonic: {e}"),
                log: None,
            })?;

        let combined = combined_output(&output);
        if !output.status.success() {
            return Err(EngineError {
                message: "LaTeX compilation failed".to_string(),
                log: Some(extract_log(&combined, &out_dir.join("doc.log"))),
            });
        }

        let pdf = out_dir.join("doc.pdf");
        if pdf.exists() {
            Ok(vec![pdf])
        } else {
            Err(EngineError {
                message: "tectonic exited successfully but produced no PDF".to_string(),
                log: Some(extract_log(&combined, &out_dir.join("doc.log"))),
            })
        }
    }
}

fn combined_output(output: &std::process::Output) -> String {
    let mut parts = Vec::new();
    if !output.stdout.is_empty() {
        parts.push(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    if !output.stderr.is_empty() {
        parts.push(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    parts.join("\n")
}

/// Picks the most useful log text: the on-disk transcript when present,
/// otherwise the captured process output.
fn extract_log(combined: &str, log_path: &Path) -> String {
    let transcript = fs::read_to_string(log_path).unwrap_or_default();
    let source = if transcript.trim().is_empty() {
        combined.to_string()
    } else {
        transcript
    };
    summarize_log(&source)
}

/// Trims a TeX transcript down to the error summary the user actually needs.
fn summarize_log(log: &str) -> String {
    let mut out = String::new();
    let mut err_count = 0;
    let mut lines = log.lines().peekable();

    while let Some(line) = lines.next() {
        if line.starts_with('!') && err_count < 5 {
            err_count += 1;
            out.push_str(line);
            out.push('\n');
            // Multi-line error messages continue until a blank line.
            for next in lines.by_ref() {
                if next.trim().is_empty() {
                    break;
                }
                out.push_str(next);
                out.push('\n');
            }
        } else if line.contains("l.") && err_count > 0 {
            // Location marker, e.g. "l.12 \badmacro".
            out.push_str(line.trim());
            out.push('\n');
        }
    }

    if out.trim().is_empty() {
        let tail: Vec<&str> = log.lines().rev().take(MAX_LOG_LINES).collect();
        tail.into_iter().rev().collect()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_errors_from_transcript() {
        let log = r#"
This is TeX, Version 3.14159265 (preloaded format=latex 2021.1.1)
! Undefined control sequence.
l.12 \thisdoesnotexist

? 
! Missing $ inserted.
<inserted text> 
                $
l.20 x + y

"#;
        let summary = summarize_log(log);
        assert!(summary.contains("Undefined control sequence"));
        assert!(summary.contains("l.12"));
        assert!(summary.contains("Missing $ inserted"));
        assert!(!summary.contains("This is TeX"));
    }

    #[test]
    fn falls_back_to_tail_when_no_errors_found() {
        let log = "line1\nline2\nline3";
        let summary = summarize_log(log);
        assert!(summary.contains("line1"));
    }

    #[test]
    fn caps_number_of_reported_errors() {
        let mut log = String::new();
        for i in 0..10 {
            log.push_str(&format!("! Error number {i}\nl.{}\n\n", i * 10));
        }
        let summary = summarize_log(&log);
        assert_eq!(summary.matches("Error number").count(), 5);
    }
}
