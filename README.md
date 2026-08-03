# LaTeX OCR

A small cross-platform desktop widget for writing LaTeX with a live preview,
and for turning math on the screen back into LaTeX with a snipping tool.

![Platform: Windows / macOS / Linux](https://img.shields.io/badge/platform-windows%20%7C%20macos%20%7C%20linux-lightgrey)
![License: MIT](https://img.shields.io/badge/license-MIT-blue)

## Features

- **Live preview.** The right pane re-renders your document as you type,
  debounced and on a background thread so typing stays responsive.
- **Export.** Save the rendered output as a PNG, JPEG or PDF from the preview
  toolbar.
- **LaTeX editor.** Monospace editor with line numbers, syntax highlighting
  (commands, comments, math mode) and Tab indentation.
- **Math tools.** Menus for fractions, roots, sums, integrals, environments,
  text styles and Greek letters. Selections are wrapped in place.
- **Files.** Open and save `.tex` files through the native file dialog.
- **Snip & OCR.** Freeze the desktop, drag a rectangle over any math, and turn
  it into LaTeX. Works with the bundled local ONNX models or any pix2tex
  (LaTeX-OCR) compatible server.
- **First-run friendly.** The LaTeX engine and PDF renderer are downloaded
  automatically on first use and cached. No system packages required.

## Building

You need a Rust toolchain (edition 2024, Rust 1.85 or newer) and a C compiler
only if you build on Windows with MSVC (required by `tectonic`'s build step
when compiling from source).

```sh
cargo build --release
cargo run
```

On Windows you can build with the GNU or MSVC toolchain; MSVC needs Visual
Studio Build Tools installed.

### Portable distribution

Tag a release (`git tag v0.1.0 && git push --tags`) and GitHub Actions builds
a portable folder for Windows, macOS and Linux and attaches the zips to the
release. Each folder is self-contained (app binary, ONNX Runtime libraries and
the OCR models), so nothing is downloaded at runtime.

To assemble the folder locally:

```sh
python tools/export_onnx.py --output models   # needs Python + PyTorch once
cargo build --release
python tools/make_dist.py --exe target/release/latex-ocr --models models --output dist
```

The distribution layout is:

```
latex-ocr/
  latex-ocr.exe            # the app (name varies by platform)
  onnxruntime.dll          # ONNX Runtime + companion shared libraries
  models/
    encoder.onnx
    decoder.onnx
    resizer.onnx
```

The app looks for the models in a `models/` folder next to its executable
first; development builds fall back to the per-user data directory (see the
local OCR setup below).

### Platform notes

- **Windows**: screen capture works out of the box.
- **macOS**: screen capture requires Screen Recording permission.
- **Linux (X11)**: screen capture works; on Wayland the compositor generally
  blocks global screen capture and the app reports this gracefully.

## How the preview works

The app ships no TeX engine itself. On first use it downloads two small tools
into the per-user data directory (`%APPDATA%\latex-ocr` on Windows,
`~/.local/share/latex-ocr` on Linux):

1. **Tectonic**, a modernized TeX/LaTeX engine, used to compile the document
   to PDF.
2. **pdfium**, Google Chromium's PDF renderer, used to turn the PDF into an
   image.

If `pdftoppm` or `mutool` is already on your PATH, the app uses it instead of
downloading pdfium.

The first preview also downloads Tectonic's support file bundle (about 90 MB).
Both downloads happen once and are cached.

## OCR setup

OCR can run against a local HTTP server, or fully on-device with no Python
involved once the models have been prepared once.

### Local (ONNX) backend

The app runs the pix2tex models itself through ONNX Runtime. ONNX Runtime is
downloaded automatically on first use (like the LaTeX engine). The model
graphs are produced once from the pix2tex checkpoints; the script writes them
straight into the app's OCR model directory:

```sh
pip install torch torchvision --index-url https://download.pytorch.org/whl/cpu
pip install pix2tex onnxscript onnx
python tools/export_onnx.py
```

The script exports three ONNX graphs into the app's OCR model directory. Use
the CPU PyTorch wheels as shown to keep the install small; the exported
models are identical either way.

Then pick **Local (ONNX)** in **Settings**. Everything after that is offline:
no Python or pix2tex installation is needed to run the app.

### Server backend

Alternatively, OCR can run against a local HTTP server that accepts a
multipart file upload and returns LaTeX. [pix2tex](https://github.com/lukas-blecher/LaTeX-OCR)
is the reference implementation:

```sh
pip install pix2tex[gui]
python -m pix2tex.api.service
```

The default endpoint is `http://127.0.0.1:8502`; change it in **Settings**,
where you can also test the connection. The app accepts any server that
answers with `{"pred": "..."}` (pix2tex), `{"latex": "..."}`, `{"text": "..."}`
or a plain-text body.

## Usage

- Type LaTeX on the left; the preview updates automatically (toggle with
  **Auto**).
- Use **Insert** to add snippets, or select text and wrap it with a tool.
- Click **Snip & OCR**, drag a rectangle over any math, and the recognized
  LaTeX appears in a result window where it can be reviewed, copied or
  inserted at the cursor.
- Use **Save…** in the preview toolbar to export the rendered output as PNG,
  JPEG or PDF.
- **Save** your document to keep it for later. Closing with unsaved changes
  asks for confirmation.

## Configuration

Settings are stored as TOML at
`%APPDATA%\latex-ocr\config.toml` (Windows) or
`~/.config/latex-ocr/config.toml` (Linux and macOS):

```toml
ocr_backend = "local"       # "local" for on-device ONNX (default), "http" for a pix2tex server
ocr_beautify = true         # reformat recognized LaTeX (aligned, \mathbf, line breaks)
ocr_url = "http://127.0.0.1:8502"
ocr_model_dir = ""          # optional override for the ONNX models directory
preview_debounce_ms = 600
preview_zoom = 1.0
tectonic_path = ""
```

In development builds the local OCR models and ONNX Runtime library live in
the per-user data directory (`%APPDATA%\latex-ocr` on Windows,
`~/.local/share/latex-ocr` on Linux and macOS); portable builds ship them next
to the executable.

## Development

```sh
cargo fmt --check     # formatting
cargo clippy --all-targets -- -D warnings   # lints
cargo test            # unit and integration tests
```

An end-to-end render check (downloads pdfium, needs a tectonic binary) is
available but not run by default:

```sh
TECTONIC_OVERRIDE=/path/to/tectonic cargo test --test render_manual -- --ignored
```

## Roadmap

- Undo history and find/replace in the editor.

## License

MIT. See [LICENSE](LICENSE).

## Credits

Created by Liam CORNU. The source lives at
[https://github.com/Inkapa/latex-ocr](https://github.com/Inkapa/latex-ocr).

Rendering is powered by [Tectonic](https://tectonic-typesetting.github.io/)
and [pdfium](https://pdfium.googlesource.com/). The local OCR backend runs the
[pix2tex](https://github.com/lukas-blecher/LaTeX-OCR) models through ONNX
Runtime.
