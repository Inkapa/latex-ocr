# LaTeX OCR

A small, efficient cross-platform LaTeX editor with OCR.

![Platform: Windows / macOS / Linux](https://img.shields.io/badge/platform-windows%20%7C%20macos%20%7C%20linux-lightgrey)
![License: MIT](https://img.shields.io/badge/license-MIT-blue)

Type LaTeX on the left, see the rendered result on the right. Snip a formula
off the screen and get its LaTeX back. Rendering and OCR run locally; nothing
is sent anywhere by default.

## Features

- Live preview, re-rendered on a background thread as you type (debounced).
- Editor with line numbers, LaTeX syntax highlighting, and Tab indentation.
- Insert menu and a toolbar for fractions, roots, sums, integrals,
  environments, Greek letters, and text/math styles. Selections are wrapped in
  place.
- Toggle the math at the cursor between inline (`$...$`) and display
  (`\[...\]`) style.
- Screen-snip OCR: select a formula on screen and get the LaTeX for it.
- Two OCR backends: an on-device ONNX model (default) or a pix2tex-compatible
  HTTP server.
- Export the preview as PNG, JPEG, or PDF.
- No TeX install required. Tectonic and a PDF renderer are fetched and cached
  on first use.

## Building

Requires Rust 1.85 or newer (edition 2024). On Windows with the MSVC toolchain
you also need the Visual Studio Build Tools, which Tectonic builds against.

```sh
cargo build --release
cargo run
```

### Portable distribution

Tagging a release (`git tag v0.1.0 && git push --tags`) triggers GitHub Actions
to build a self-contained folder per platform and attach the archives to the
release. Each folder ships the binary, the ONNX Runtime libraries, and the OCR
models, so nothing is downloaded at runtime.

To build the folder locally:

```sh
python tools/export_onnx.py --output models   # one-time, needs Python + PyTorch
cargo build --release
python tools/make_dist.py --exe target/release/latex-ocr --models models --output dist
```

Layout:

```
latex-ocr/
  latex-ocr.exe            # binary name varies by platform
  onnxruntime.dll          # ONNX Runtime + companion libraries
  models/
    encoder.onnx
    decoder.onnx
    resizer.onnx
```

The app looks for `models/` next to the executable first. Development builds
fall back to the per-user data directory (see [OCR](#ocr)).

## Usage

- Type on the left; the preview updates automatically. **Auto** toggles live
  re-rendering, **Render now** forces it.
- Add math from the **Insert** menu, or select text and wrap it with a toolbar
  button. The toolbar also switches math between inline and display style.
- **Snip & OCR**, drag over a formula, then insert or copy the result from the
  OCR window.
- **Save…** exports the preview as PNG, JPEG, or PDF.
- Open and save `.tex` files from the toolbar. Closing with unsaved changes
  prompts first.

## OCR

Two backends are supported. The default runs on-device via ONNX Runtime; a
pix2tex-compatible HTTP server can be used instead. Other model families are
not supported.

### On-device (ONNX)

The default backend runs the pix2tex model (LaTeX-OCR by Lukas Blecher, MIT) as
three ONNX graphs: a resolution predictor, a vision-transformer encoder, and a
transformer decoder. ONNX Runtime is fetched and cached on first use.

Export the graphs once with:

```sh
pip install torch torchvision --index-url https://download.pytorch.org/whl/cpu
pip install pix2tex onnxscript onnx
python tools/export_onnx.py
```

This writes `encoder.onnx`, `decoder.onnx`, and `resizer.onnx` into the app's
OCR model directory. Development builds keep them in the per-user data directory
(`%APPDATA%\latex-ocr` on Windows, `~/.local/share/latex-ocr` on Linux and
macOS); portable builds ship them next to the executable.

Select **Local (ONNX)** in Settings (the default) to run recognition offline.

### HTTP server

Alternatively, point the backend at a pix2tex-compatible server that takes a
multipart file upload and returns LaTeX:

```sh
pip install pix2tex[gui]
python -m pix2tex.api.service
```

The default endpoint is `http://127.0.0.1:8502`, configurable and testable in
Settings. The app accepts `{"pred": "..."}` (pix2tex), `{"latex": "..."}`,
`{"text": "..."}`, or a plain-text body.

## Configuration

Settings live in TOML at `%APPDATA%\latex-ocr\config.toml` (Windows) or
`~/.config/latex-ocr/config.toml` (Linux, macOS):

```toml
ocr_backend = "local"       # "local" for on-device ONNX (default), "http" for a pix2tex server
ocr_beautify = true         # reformat recognized LaTeX (aligned, \mathbf, line breaks)
ocr_url = "http://127.0.0.1:8502"
ocr_model_dir = ""          # override for the ONNX models directory
preview_debounce_ms = 600
preview_zoom = 1.0
tectonic_path = ""          # override for the tectonic executable
```

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

An end-to-end render check (downloads pdfium, needs a tectonic binary) is
available but not run by default:

```sh
TECTONIC_OVERRIDE=/path/to/tectonic cargo test --test render_manual -- --ignored
```

## Platform notes

- **Windows**: screen capture works out of the box.
- **macOS**: screen capture needs the Screen Recording permission.
- **Linux**: screen capture works on X11. On Wayland the compositor usually
  blocks global capture, and the app reports this.

## License

MIT. See [LICENSE](LICENSE).

## Acknowledgements

- [pix2tex (LaTeX-OCR)](https://github.com/lukas-blecher/LaTeX-OCR) by Lukas
  Blecher, MIT. OCR model, checkpoints, and tokenizer.
- [x-transformers](https://github.com/lucidrains/x-transformers) by Phil Wang,
  MIT. Transformer layers of the OCR model.
- [Tectonic](https://tectonic-typesetting.github.io), MIT. LaTeX engine.
- [pdfium](https://pdfium.googlesource.com/pdfium/) (Chromium), BSD-3-Clause.
  PDF rasterizer.
- [ONNX Runtime](https://onnxruntime.ai) by Microsoft, MIT. Local inference.
- DejaVu fonts, Bitstream Vera license. Bundled UI fonts.
