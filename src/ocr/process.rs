//! Image preprocessing and LaTeX cleanup for the local OCR backend.
//!
//! The image pipeline mirrors pix2tex: normalize and crop the capture onto a
//! white canvas sized to a multiple of 32, optionally shrink it into the
//! model's input box, then convert to the same normalized tensor the PyTorch
//! code feeds the network. `post_process` replicates pix2tex's regex cleanup
//! of the decoded token stream.

use fancy_regex::Regex;
use image::imageops::FilterType;
use image::{GrayImage, Luma, RgbaImage};

/// Maximum image size the encoder accepts (width, height).
pub const MAX_WIDTH: u32 = 672;
pub const MAX_HEIGHT: u32 = 192;
/// Minimum image size the encoder accepts.
pub const MIN_WIDTH: u32 = 32;
pub const MIN_HEIGHT: u32 = 32;
/// Normalization constants from pix2tex's `test_transform`.
pub const NORMALIZE_MEAN: f32 = 0.7931;
pub const NORMALIZE_STD: f32 = 0.1738;

/// Images are padded so both dimensions are a multiple of this.
const PAD_DIV: u32 = 32;
const THRESHOLD: f32 = 128.0;

/// PIL's luma coefficients for the "L" channel of an RGBA image.
fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000) as u8
}

fn next_multiple(n: u32, div: u32) -> u32 {
    let (q, r) = (n / div, n % div);
    div * (q + u32::from(r > 0))
}

fn clamp_round(v: f32) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

/// Bounding box of non-zero pixels, PIL's `getbbox` for grayscale images.
fn bbox_nonzero(img: &GrayImage) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = (img.width(), img.height());
    let mut min_x = w;
    let mut min_y = h;
    let mut max_x = 0;
    let mut max_y = 0;
    for (x, y, p) in img.enumerate_pixels() {
        if p[0] != 0 {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    if max_x < min_x {
        None
    } else {
        Some((min_x, min_y, max_x - min_x + 1, max_y - min_y + 1))
    }
}

/// Normalizes, crops and pads a capture onto a white canvas whose dimensions
/// are multiples of 32, as pix2tex's `pad` does.
pub fn pad(img: &RgbaImage) -> GrayImage {
    let (w, h) = (img.width(), img.height());
    let mut data: Vec<f32> = Vec::with_capacity((w * h) as usize);
    let mut alpha_min = 255u8;
    let mut alpha_max = 0u8;
    let mut alphas = Vec::with_capacity((w * h) as usize);
    for p in img.pixels() {
        data.push(f32::from(luma(p[0], p[1], p[2])));
        alphas.push(p[3]);
        alpha_min = alpha_min.min(p[3]);
        alpha_max = alpha_max.max(p[3]);
    }
    // Opaque captures use the luminance channel; transparent images use the
    // inverted alpha as the glyph mask.
    if alpha_min != alpha_max {
        for (d, &a) in data.iter_mut().zip(&alphas) {
            *d = 255.0 - f32::from(a);
        }
    }
    pad_luma(&data, w, h)
}

/// `pad` for images that are already grayscale.
pub fn pad_gray(img: &GrayImage) -> GrayImage {
    let (w, h) = (img.width(), img.height());
    let data: Vec<f32> = img.pixels().map(|p| f32::from(p[0])).collect();
    pad_luma(&data, w, h)
}

fn pad_luma(data_in: &[f32], w: u32, h: u32) -> GrayImage {
    let mut data = data_in.to_vec();

    let (mut dmin, mut dmax) = (f32::INFINITY, f32::NEG_INFINITY);
    for &x in &data {
        dmin = dmin.min(x);
        dmax = dmax.max(x);
    }
    let span = dmax - dmin;
    if span > 0.0 {
        for x in &mut data {
            *x = (*x - dmin) / span * 255.0;
        }
    }

    let mean = data.iter().sum::<f32>() / data.len() as f32;
    // Dark text on a light background is the norm; invert when the image is
    // mostly dark so the glyphs end up black on white.
    let invert = mean <= THRESHOLD;
    let mut mask = vec![false; data.len()];
    for i in 0..data.len() {
        let text = if invert {
            data[i] > THRESHOLD
        } else {
            data[i] < THRESHOLD
        };
        if invert {
            data[i] = 255.0 - data[i];
        }
        mask[i] = text;
    }

    let mut min_x = w;
    let mut min_y = h;
    let mut max_x = 0;
    let mut max_y = 0;
    for y in 0..h {
        for x in 0..w {
            if mask[(y * w + x) as usize] {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    let (bw, bh) = if max_x < min_x {
        (w, h)
    } else {
        (max_x - min_x + 1, max_y - min_y + 1)
    };

    let (pw, ph) = (next_multiple(bw, PAD_DIV), next_multiple(bh, PAD_DIV));
    let mut out = GrayImage::from_pixel(pw, ph, Luma([255]));
    for y in 0..bh {
        for x in 0..bw {
            let (sx, sy) = if max_x < min_x {
                (x, y)
            } else {
                (min_x + x, min_y + y)
            };
            out.put_pixel(x, y, Luma([clamp_round(data[(sy * w + sx) as usize])]));
        }
    }
    out
}

/// Shrinks or pads an image so it fits within the model's input dimensions.
pub fn minmax_size(img: &GrayImage) -> GrayImage {
    let (w, h) = (img.width(), img.height());
    let max_ratio = (w as f32 / MAX_WIDTH as f32).max(h as f32 / MAX_HEIGHT as f32);
    let mut out = if max_ratio > 1.0 {
        let nw = ((w as f32) / max_ratio).floor().max(1.0) as u32;
        let nh = ((h as f32) / max_ratio).floor().max(1.0) as u32;
        image::imageops::resize(img, nw, nh, FilterType::Triangle)
    } else {
        img.clone()
    };

    if out.width() < MIN_WIDTH || out.height() < MIN_HEIGHT {
        let pw = out.width().max(MIN_WIDTH);
        let ph = out.height().max(MIN_HEIGHT);
        let mut canvas = GrayImage::from_pixel(pw, ph, Luma([255]));
        let (ox, oy) = bbox_nonzero(&out)
            .map(|(x, y, _, _)| (x, y))
            .unwrap_or((0, 0));
        for y in 0..out.height() {
            for x in 0..out.width() {
                canvas.put_pixel(ox + x, oy + y, *out.get_pixel(x, y));
            }
        }
        out = canvas;
    }
    out
}

/// Resizes a grayscale image with the given PIL-equivalent filter.
pub fn resize(img: &GrayImage, width: u32, height: u32, filter: FilterType) -> GrayImage {
    image::imageops::resize(img, width.max(1), height.max(1), filter)
}

/// Converts a grayscale image into the normalized `(1, 1, H, W)` float input
/// the models expect, matching pix2tex's `test_transform`.
pub fn to_tensor(img: &GrayImage) -> Vec<f32> {
    img.pixels()
        .map(|p| (f32::from(p[0]) / 255.0 - NORMALIZE_MEAN) / NORMALIZE_STD)
        .collect()
}

/// Removes whitespace that LaTeX does not need, as pix2tex's `post_process`.
pub fn post_process(input: &str) -> String {
    let text_reg = Regex::new(r"\\(operatorname|mathrm|text|mathbf)\s?\*? {.*?}").expect("regex");
    let letter = "[a-zA-Z]";
    let noletter = "[^a-zA-Z]";

    let mut names: Vec<String> = text_reg
        .find_iter(input)
        .filter_map(|m| m.ok())
        .map(|m| m.as_str().replace(' ', ""))
        .collect();
    let mut s = String::with_capacity(input.len());
    let mut last = 0;
    for m in text_reg.find_iter(input).filter_map(|m| m.ok()) {
        s.push_str(&input[last..m.start()]);
        s.push_str(&names.remove(0));
        last = m.end();
    }
    s.push_str(&input[last..]);

    let collapse = |noletter: &str, letter: &str| {
        Regex::new(&format!(r"(?!\\ )({noletter})\s+?({letter})")).expect("regex")
    };
    let a = collapse(noletter, noletter);
    let b = collapse(noletter, letter);
    let c = collapse(letter, noletter);

    loop {
        let next = a.replace_all(&s, "$1$2").into_owned();
        let next = b.replace_all(&next, "$1$2").into_owned();
        let next = c.replace_all(&next, "$1$2").into_owned();
        if next == s {
            return s;
        }
        s = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Luma};

    fn gray(w: u32, h: u32, f: impl Fn(u32, u32) -> u8) -> GrayImage {
        ImageBuffer::from_fn(w, h, |x, y| Luma([f(x, y)]))
    }

    fn rgba_from_gray(g: &GrayImage) -> RgbaImage {
        ImageBuffer::from_fn(g.width(), g.height(), |x, y| {
            let v = g.get_pixel(x, y).0[0];
            image::Rgba([v, v, v, 255])
        })
    }

    #[test]
    fn pad_keeps_small_text_on_white_canvas() {
        // A small dark glyph on white.
        let img = rgba_from_gray(&gray(10, 8, |x, y| if x < 4 && y < 3 { 0 } else { 255 }));
        let out = pad(&img);
        assert_eq!(out.width(), 32);
        assert_eq!(out.height(), 32);
        assert_eq!(out.get_pixel(0, 0).0[0], 0); // glyph at origin
        assert_eq!(out.get_pixel(31, 31).0[0], 255); // white margin
    }

    #[test]
    fn pad_inverts_dark_background() {
        // Bright text on a dark background.
        let img = rgba_from_gray(&gray(8, 8, |x, y| if x < 3 && y < 3 { 255 } else { 0 }));
        let out = pad(&img);
        // After inversion the glyph should be black on white again.
        assert_eq!(out.get_pixel(0, 0).0[0], 0);
        assert_eq!(out.get_pixel(7, 7).0[0], 255);
    }

    #[test]
    fn pad_rounds_dimensions_up_to_multiple_of_32() {
        let img = rgba_from_gray(&gray(33, 40, |_, _| 0));
        let out = pad(&img);
        assert_eq!(out.width(), 64);
        assert_eq!(out.height(), 64);
    }

    #[test]
    fn minmax_shrinks_wide_images() {
        let img = gray(1024, 64, |_, _| 0);
        let out = minmax_size(&img);
        // Proportional downscale keeps width within the model input box.
        assert_eq!(out.width(), MAX_WIDTH);
        assert_eq!(out.height(), 42);
    }

    #[test]
    fn to_tensor_normalizes_white_and_black() {
        let img = gray(1, 2, |_, y| if y == 0 { 0 } else { 255 });
        let t = to_tensor(&img);
        let expected_black = (0.0 - NORMALIZE_MEAN) / NORMALIZE_STD;
        let expected_white = (1.0 - NORMALIZE_MEAN) / NORMALIZE_STD;
        assert!((t[0] - expected_black).abs() < 1e-6);
        assert!((t[1] - expected_white).abs() < 1e-6);
    }

    #[test]
    fn post_process_removes_spaces_around_operators() {
        assert_eq!(post_process("x + y"), "x+y");
        assert_eq!(post_process("a = 5"), "a=5");
        assert_eq!(post_process("x^{2}+y"), "x^{2}+y");
    }

    #[test]
    fn post_process_keeps_single_spaces_between_letters() {
        assert_eq!(post_process("x   y  z"), "x y z");
    }

    #[test]
    fn post_process_collapses_command_braces() {
        assert_eq!(post_process(r"\frac { 1 } { 2 }"), r"\frac{1}{2}");
        assert_eq!(post_process(r"\sqrt {x} + y"), r"\sqrt{x}+y");
        assert_eq!(post_process(r"\mathrm {ab} + c"), r"\mathrm{ab}+c");
        assert_eq!(
            post_process(r"\operatorname {lim} x"),
            r"\operatorname{lim}x"
        );
    }

    #[test]
    fn post_process_preserves_escaped_space() {
        assert_eq!(post_process(r"a \ b"), r"a\ b");
    }

    #[test]
    fn post_process_handles_empty_input() {
        assert_eq!(post_process(""), "");
    }
}
