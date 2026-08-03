//! Local OCR backend that runs the pix2tex model entirely on-device.
//!
//! The backend loads three ONNX graphs exported from the pix2tex checkpoints:
//! the image resizer that predicts the input resolution, the vision
//! transformer encoder, and the transformer decoder. The autoregressive
//! sampling loop and all pre/post-processing run in Rust, so no Python or
//! pix2tex installation is needed at runtime.
//!
//! ONNX Runtime and the model graphs are provisioned lazily on the first
//! recognition: portable builds use the copies shipped next to the executable,
//! while development builds download the runtime library into the app data
//! directory and expect the graphs from `tools/export_onnx.py`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use image::RgbaImage;
use image::imageops::FilterType;
use log::{debug, info};
use ort::session::{Session, builder::GraphOptimizationLevel};
use ort::value::Tensor;

use super::process::{self, post_process};
use super::provision;
use super::tokenizer::Tokenizer;
use super::{OcrBackend, OcrError};

/// Special tokens from the pix2tex vocabulary.
const BOS_TOKEN: i64 = 1;
const EOS_TOKEN: i64 = 2;
const MAX_SEQ_LEN: usize = 512;
const TEMPERATURE: f32 = 0.2;
/// Keeps the top fraction of logits during sampling, as x-transformers' `top_k`.
const TOP_K_THRESHOLD: f32 = 0.9;

fn ort_error(action: impl std::fmt::Display, source: impl std::fmt::Display) -> OcrError {
    OcrError {
        message: format!("local OCR failed to {action}: {source}"),
    }
}

/// Configures the ONNX Runtime environment once per process. Honors
/// `ORT_DYLIB_PATH` when set, otherwise uses the provisioned library.
fn init_runtime(lib_path: &Path) -> Result<(), OcrError> {
    static INIT: OnceLock<Result<(), String>> = OnceLock::new();
    INIT.get_or_init(|| {
        let path = std::env::var_os("ORT_DYLIB_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| lib_path.to_path_buf());
        ort::init_from(&path)
            .map_err(|e| format!("cannot load ONNX Runtime from {}: {e}", path.display()))?
            .commit();
        Ok(())
    })
    .clone()
    .map_err(|e| OcrError {
        message: format!("cannot initialize ONNX Runtime: {e}"),
    })
}

/// Deterministic seedable PRNG for temperature sampling.
struct Random(u64);

impl Random {
    fn new() -> Self {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9);
        Random(seed | 1)
    }

    fn next_f64(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x as f64 / u64::MAX as f64
    }
}

/// Loaded ONNX sessions plus the tokenizer.
struct ReadyBackend {
    encoder: Mutex<Session>,
    decoder: Mutex<Session>,
    resizer: Mutex<Option<Session>>,
    tokenizer: Tokenizer,
    rng: Mutex<Random>,
}

impl ReadyBackend {
    fn load(models_dir: &Path) -> Result<Self, OcrError> {
        let encoder_path = provision::encoder_path(models_dir);
        let decoder_path = provision::decoder_path(models_dir);
        let resizer_path = provision::resizer_path(models_dir);

        let encoder = Session::builder()
            .map_err(|e| ort_error("build encoder session", e))?
            .with_optimization_level(GraphOptimizationLevel::Level1)
            .map_err(|e| ort_error("configure encoder session", e))?
            .commit_from_file(&encoder_path)
            .map_err(|e| ort_error(format!("load {}", encoder_path.display()), e))?;

        let decoder = Session::builder()
            .map_err(|e| ort_error("build decoder session", e))?
            .with_optimization_level(GraphOptimizationLevel::Level1)
            .map_err(|e| ort_error("configure decoder session", e))?
            .commit_from_file(&decoder_path)
            .map_err(|e| ort_error(format!("load {}", decoder_path.display()), e))?;

        let resizer = if resizer_path.is_file() {
            info!("local OCR resizer loaded from {}", resizer_path.display());
            let session = Session::builder()
                .map_err(|e| ort_error("build resizer session", e))?
                .with_optimization_level(GraphOptimizationLevel::Level1)
                .map_err(|e| ort_error("configure resizer session", e))?
                .commit_from_file(&resizer_path)
                .map_err(|e| ort_error(format!("load {}", resizer_path.display()), e))?;
            Some(session)
        } else {
            None
        };

        debug!(
            "local OCR ready (encoder={}, decoder={}, resizer={})",
            encoder_path.display(),
            decoder_path.display(),
            if resizer.is_some() { "yes" } else { "no" }
        );
        Ok(Self {
            encoder: Mutex::new(encoder),
            decoder: Mutex::new(decoder),
            resizer: Mutex::new(resizer),
            tokenizer: Tokenizer::bundled(),
            rng: Mutex::new(Random::new()),
        })
    }

    fn run_encoder(&self, img: &image::GrayImage) -> Result<(Vec<f32>, usize), OcrError> {
        let (h, w) = (img.height(), img.width());
        let tensor = process::to_tensor(img);
        let input = Tensor::from_array((vec![1usize, 1, h as usize, w as usize], tensor))
            .map_err(|e| ort_error("build encoder input", e))?;
        let mut session = self.encoder.lock().unwrap();
        let outputs = session
            .run(ort::inputs! { "image" => input })
            .map_err(|e| ort_error("run encoder", e))?;
        let array = outputs[0]
            .try_extract_array::<f32>()
            .map_err(|e| ort_error("read encoder output", e))?;
        // The context is `[1, seq, dim]`; the embedding dimension is taken
        // from the actual output shape so it always matches the model.
        let dim = array.shape().last().copied().unwrap_or(0);
        Ok((array.to_owned().into_raw_vec_and_offset().0, dim))
    }

    fn run_resizer(&self, img: &image::GrayImage) -> Result<u32, OcrError> {
        let (h, w) = (img.height(), img.width());
        let tensor = process::to_tensor(img);
        let input = Tensor::from_array((vec![1usize, 1, h as usize, w as usize], tensor))
            .map_err(|e| ort_error("build resizer input", e))?;
        let mut guard = self.resizer.lock().unwrap();
        let session = guard.as_mut().ok_or_else(|| OcrError {
            message: "image resizer model is not loaded".to_string(),
        })?;
        let outputs = session
            .run(ort::inputs! { "image" => input })
            .map_err(|e| ort_error("run resizer", e))?;
        let logits = outputs[0]
            .try_extract_array::<f32>()
            .map(|a| a.to_owned().into_raw_vec_and_offset().0)
            .map_err(|e| ort_error("read resizer output", e))?;
        let best = logits
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i as u32)
            .unwrap_or(0);
        Ok((best + 1) * 32)
    }

    fn run_decoder(
        &self,
        tokens: &[i64],
        context: &[f32],
        ctx_len: usize,
        dim: usize,
    ) -> Result<Vec<f32>, OcrError> {
        let t = tokens.len();
        let tokens_tensor = Tensor::from_array((vec![1usize, t], tokens.to_vec()))
            .map_err(|e| ort_error("build decoder token input", e))?;
        let mask = vec![true; t];
        let mask_tensor = Tensor::from_array((vec![1usize, t], mask))
            .map_err(|e| ort_error("build decoder mask input", e))?;
        let context_tensor = Tensor::from_array((vec![1usize, ctx_len, dim], context.to_vec()))
            .map_err(|e| ort_error("build decoder context input", e))?;
        let mut session = self.decoder.lock().unwrap();
        let outputs = session
            .run(ort::inputs! {
                "tokens" => tokens_tensor,
                "mask" => mask_tensor,
                "context" => context_tensor,
            })
            .map_err(|e| ort_error("run decoder", e))?;
        outputs[0]
            .try_extract_array::<f32>()
            .map(|a| a.to_owned().into_raw_vec_and_offset().0)
            .map_err(|e| ort_error("read decoder output", e))
    }

    /// Greedy, temperature-weighted autoregressive decoding.
    fn sample(&self, context: &[f32], ctx_len: usize, dim: usize) -> Result<Vec<i64>, OcrError> {
        if ctx_len == 0 {
            return Err(OcrError {
                message: "encoder produced no context".to_string(),
            });
        }
        let mut tokens = vec![BOS_TOKEN];
        for _ in 0..MAX_SEQ_LEN {
            let logits = self.run_decoder(&tokens, context, ctx_len, dim)?;
            let vocab = logits.len() / tokens.len();
            let last = &logits[(tokens.len() - 1) * vocab..tokens.len() * vocab];

            let filtered = top_k_filter(last, TOP_K_THRESHOLD);
            let probs = softmax_scaled(&filtered, TEMPERATURE);
            let mut rng = self.rng.lock().unwrap();
            let next = sample_index(&probs, &mut rng) as i64;

            tokens.push(next);
            if next == EOS_TOKEN {
                break;
            }
        }
        // Drop the start token, mirroring `out[:, t:]` in pix2tex's generate.
        Ok(tokens[1..].to_vec())
    }

    /// Runs the whole pipeline: pad, optional resolution prediction, encode
    /// and decode.
    fn recognize_raw(&self, image: &RgbaImage) -> Result<Vec<i64>, OcrError> {
        let mut img = process::minmax_size(&process::pad(image));
        let has_resizer = self.resizer.lock().unwrap().is_some();

        if has_resizer {
            let input_image = img.clone();
            let mut r = 1.0f32;
            let mut w = input_image.width();
            let mut h = input_image.height();
            for _ in 0..10 {
                h = (h as f32 * r) as u32;
                let filter = if r > 1.0 {
                    FilterType::Triangle
                } else {
                    FilterType::Lanczos3
                };
                let resized = process::resize(&input_image, w, h, filter);
                img = process::pad_gray(&process::minmax_size(&resized));
                let predicted = self.run_resizer(&img)?;
                w = predicted;
                if w == img.width() {
                    break;
                }
                r = w as f32 / img.width() as f32;
            }
        } else {
            img = process::pad_gray(&img);
        }

        let (context, dim) = self.run_encoder(&img)?;
        if dim == 0 {
            return Err(OcrError {
                message: "encoder produced an empty context".to_string(),
            });
        }
        let ctx_len = context.len() / dim;
        self.sample(&context, ctx_len, dim)
    }
}

/// Local pix2tex-style OCR running against ONNX models. The ONNX Runtime
/// library and the model graphs are set up lazily on the first recognition, so
/// construction is cheap and any provisioning errors surface at OCR time.
pub struct LocalOcrBackend {
    models_dir: PathBuf,
    inner: Mutex<Option<Result<Arc<ReadyBackend>, OcrError>>>,
}

impl LocalOcrBackend {
    pub fn new(models_dir: PathBuf) -> Self {
        Self {
            models_dir,
            inner: Mutex::new(None),
        }
    }

    fn ensure(&self) -> Result<Arc<ReadyBackend>, OcrError> {
        let mut slot = self.inner.lock().unwrap();
        if slot.is_none() {
            *slot = Some(build_backend(&self.models_dir));
        }
        slot.as_ref().unwrap().clone()
    }
}

fn build_backend(models_dir: &Path) -> Result<Arc<ReadyBackend>, OcrError> {
    // Fail fast on missing model graphs so nothing is downloaded.
    if !provision::encoder_path(models_dir).is_file()
        || !provision::decoder_path(models_dir).is_file()
    {
        return Err(OcrError {
            message: format!(
                "ONNX models not found in {}. Run `python tools/export_onnx.py` once to create them.",
                models_dir.display()
            ),
        });
    }
    let lib_path = provision::resolve_onnxruntime().map_err(|e| OcrError {
        message: format!("cannot set up ONNX Runtime: {e}"),
    })?;
    init_runtime(&lib_path)?;
    Ok(Arc::new(ReadyBackend::load(models_dir)?))
}

impl OcrBackend for LocalOcrBackend {
    fn recognize(&self, image: &RgbaImage) -> Result<String, OcrError> {
        let backend = self.ensure()?;
        let ids = backend.recognize_raw(image)?;
        let latex = backend.tokenizer.decode_latex(&ids);
        Ok(post_process(&latex))
    }
}

/// Keeps only the top `(1 - thres)` fraction of logits, setting the rest to
/// negative infinity, exactly like x-transformers' `top_k`.
fn top_k_filter(logits: &[f32], thres: f32) -> Vec<f32> {
    let keep = (((1.0 - thres) * logits.len() as f32).ceil() as usize).min(logits.len());
    let mut order: Vec<usize> = (0..logits.len()).collect();
    order.sort_by(|&a, &b| {
        logits[b]
            .partial_cmp(&logits[a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut out = vec![f32::NEG_INFINITY; logits.len()];
    for &i in order.iter().take(keep) {
        out[i] = logits[i];
    }
    out
}

fn softmax_scaled(logits: &[f32], temperature: f32) -> Vec<f32> {
    let max = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max) / temperature;
    let exps: Vec<f32> = logits
        .iter()
        .map(|&x| ((x / temperature) - max).exp())
        .collect();
    let sum: f32 = exps.iter().sum();
    exps.into_iter().map(|e| e / sum).collect()
}

fn sample_index(probs: &[f32], rng: &mut Random) -> usize {
    let draw = rng.next_f64();
    let mut cumulative = 0.0f32;
    for (i, &p) in probs.iter().enumerate() {
        cumulative += p;
        if f64::from(cumulative) >= draw {
            return i;
        }
    }
    probs.len() - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_k_keeps_only_largest_values() {
        let logits = vec![0.1, 5.0, 2.0, -1.0, 3.0];
        let filtered = top_k_filter(&logits, 0.9);
        // keep = ceil(0.1 * 5) = 1, so only the max survives.
        assert_eq!(filtered.iter().filter(|&&x| x.is_finite()).count(), 1);
        assert!(filtered.contains(&5.0));
    }

    #[test]
    fn softmax_is_normalized() {
        let probs = softmax_scaled(&[1.0, 2.0, 3.0], 0.2);
        let sum: f32 = probs.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
        assert!(probs[2] > probs[1] && probs[1] > probs[0]);
    }

    #[test]
    fn sampling_respects_probabilities() {
        let probs = vec![1.0, 0.0, 0.0];
        let mut rng = Random(0xDEAD_BEEF);
        for _ in 0..10 {
            assert_eq!(sample_index(&probs, &mut rng), 0);
        }
    }

    #[test]
    fn sample_index_returns_last_on_numerical_drift() {
        let probs = vec![0.0, 0.0];
        let mut rng = Random(0x1234_5678);
        assert_eq!(sample_index(&probs, &mut rng), 1);
    }

    #[test]
    fn missing_models_report_helpful_error() {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalOcrBackend::new(dir.path().to_path_buf());
        let img = RgbaImage::from_pixel(10, 10, image::Rgba([255, 255, 255, 255]));
        let err = backend.recognize(&img).unwrap_err();
        assert!(err.message.contains("export_onnx.py"), "{}", err.message);
    }
}
