//! Pluggable OCR backends that turn captured images into LaTeX.

pub mod beautify;
pub mod local;
pub mod process;
pub mod provision;
pub mod tokenizer;

use std::io::Cursor;

use image::RgbaImage;

/// Failed OCR request, with a message suitable for the user.
#[derive(Debug, Clone)]
pub struct OcrError {
    pub message: String,
}

impl std::fmt::Display for OcrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for OcrError {}

/// Recognizes math/text in an image and returns LaTeX source.
pub trait OcrBackend: Send + Sync {
    fn recognize(&self, image: &RgbaImage) -> Result<String, OcrError>;
}

/// Talks to a pix2tex (LaTeX-OCR) compatible HTTP server. The server exposes a
/// single POST endpoint that accepts a multipart file upload and answers with
/// JSON like `{"pred": "..."}`.
pub struct HttpOcrBackend {
    client: reqwest::blocking::Client,
    url: String,
}

impl HttpOcrBackend {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
            url: url.into(),
        }
    }

    /// Quick connectivity check used for the settings screen.
    pub fn is_reachable(&self) -> bool {
        reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map(|c| c.get(&self.url).send().is_ok())
            .unwrap_or(false)
    }
}

impl OcrBackend for HttpOcrBackend {
    fn recognize(&self, image: &RgbaImage) -> Result<String, OcrError> {
        let mut png = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|e| OcrError {
                message: format!("cannot encode capture: {e}"),
            })?;

        let part = reqwest::blocking::multipart::Part::bytes(png)
            .file_name("capture.png")
            .mime_str("image/png")
            .map_err(|e| OcrError {
                message: format!("cannot build request: {e}"),
            })?;
        let form = reqwest::blocking::multipart::Form::new().part("file", part);

        let response = self
            .client
            .post(&self.url)
            .multipart(form)
            .send()
            .map_err(|e| OcrError {
                message: format!("OCR server unreachable at {}: {e}", self.url),
            })?;

        if !response.status().is_success() {
            return Err(OcrError {
                message: format!("OCR server returned {}", response.status()),
            });
        }

        let body = response.text().map_err(|e| OcrError {
            message: format!("cannot read OCR response: {e}"),
        })?;
        parse_latex_response(&body)
    }
}

/// Extracts LaTeX from a server response. Accepts `{"pred": "..."}`,
/// `{"latex": "..."}`, `{"text": "..."}` or a plain text body.
pub fn parse_latex_response(body: &str) -> Result<String, OcrError> {
    let trimmed = body.trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        for key in ["pred", "latex", "text", "result", "output"] {
            if let Some(text) = value.get(key).and_then(serde_json::Value::as_str) {
                let text = text.trim();
                if !text.is_empty() {
                    return Ok(text.to_string());
                }
            }
        }
        return Err(OcrError {
            message: "OCR server returned an empty result".to_string(),
        });
    }
    if !trimmed.is_empty() {
        return Ok(trimmed.to_string());
    }
    Err(OcrError {
        message: "OCR server returned an empty result".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pred_field() {
        let latex = parse_latex_response(r#"{"pred": "x^2 + y"}"#).unwrap();
        assert_eq!(latex, "x^2 + y");
    }

    #[test]
    fn parses_alternate_field_names() {
        assert_eq!(parse_latex_response(r#"{"latex":"a+b"}"#).unwrap(), "a+b");
        assert_eq!(parse_latex_response(r#"{"text":"c"}"#).unwrap(), "c");
    }

    #[test]
    fn falls_back_to_plain_text_body() {
        assert_eq!(
            parse_latex_response("\\frac{1}{2}").unwrap(),
            "\\frac{1}{2}"
        );
        assert_eq!(parse_latex_response("  hello  ").unwrap(), "hello");
    }

    #[test]
    fn rejects_empty_result() {
        assert!(parse_latex_response("").is_err());
        assert!(parse_latex_response("{}").is_err());
        assert!(parse_latex_response(r#"{"pred": "  "}"#).is_err());
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(
            parse_latex_response(r#"{"pred": "  x^2  "}"#).unwrap(),
            "x^2"
        );
    }
}
