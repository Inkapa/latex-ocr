# LaTeX OCR

A small cross-platform LaTeX editor with OCR.

![Platform: Windows / macOS / Linux](https://img.shields.io/badge/platform-windows%20%7C%20macos%20%7C%20linux-lightgrey)
![License: MIT](https://img.shields.io/badge/license-MIT-blue)

## Features

- **Live preview.** Re-renders the document as you type, debounced and on a
  background thread so editing stays responsive.
- **LaTeX editor.** Monospace editor with line numbers, syntax highlighting
  (commands, comments, math mode) and Tab indentation.
- **Math snippets.** Menus for fractions, roots, sums, integrals, environments,
  text styles and Greek letters. Selections are wrapped in place.
- **Math toolbar.** Word-style buttons for bold, italic, roman and code styles
  that choose the text or math command by context, plus one-click superscript,
  subscript, roots and fractions.
- **Inline/display toggle.** Switch the math at the cursor between inline
  (`$...$`) and display (`\[...\]`) style without touching the content; the
  toolbar also wraps a selection when the cursor is not in math.
- **Screen-snip OCR.** Freeze the screen, select a formula, and OCR returns
  the corresponding LaTeX for review or insertion.
- **Two OCR backends.** An on-device ONNX engine (default) and an optional
  pix2tex-compatible HTTP server.
- **Export.** Save the rendered output as a PNG, JPEG or PDF from the preview.
- **Self-contained rendering.** Tectonic and a PDF renderer are downloaded and
  cached on first use, so no TeX distribution is required.

## Building

You need Rust edition 2024 (1.85 or newer). On Windows with the MSVC toolchain
you also need the Visual Studio Build Tools for tectonic's build step.

```sh
cargo build --release
cargo run
```

### Portable distribution

Tag a release (`git tag v0.1.0 && git push --tags`) and GitHub Actions builds
a self-contained folder for each platform and attaches the archives to the
release. Each folder ships the app binary, the ONNX Runtime libraries and the
OCR models, so nothing is downloaded at runtime.

To assemble the folder locally:

```sh
python tools/export_onnx.py --output models   # one-time, needs Python + PyTorch
cargo build --release
python tools/make_dist.py --exe target/release/latex-ocr --models models --output dist
```

The folder layout is:

```
latex-ocr/
  latex-ocr.exe            # the app (name varies by platform)
  onnxruntime.dll          # ONNX Runtime + companion shared libraries
  models/
    encoder.onnx
    decoder.onnx
    resizer.onnx
```

The app looks for models in a `models/` folder next to its executable first;
development builds fall back to the per-user data directory (see the OCR
section below).

## Usage

- Type LaTeX on the left; the preview updates automatically. Use **Auto** to
  toggle live re-rendering or **Render now** to force it.
- Add math with the **Insert** menu, or select text and wrap it with a tool.
  The toolbar above the editor switches math between inline and display style
  and applies text and math styles to the selection.
- Click **Snip & OCR**, drag a rectangle over a formula, and review the result
  in the OCR window. Insert it at the cursor or copy it.
- Use **Save…** in the preview toolbar to export the rendered output as PNG,
  JPEG or PDF.
- Open and save `.tex` files from the toolbar. Closing with unsaved changes
  asks for confirmation.

## OCR

OCR supports two backends. The default runs on-device through ONNX Runtime; a
pix2tex-compatible HTTP server can be used instead. Other model families are
not currently supported.

### On-device (ONNX)

The default backend runs the pix2tex model (LaTeX-OCR by Lukas Blecher, MIT
licensed) as three ONNX graphs: a resolution predictor, a vision transformer
encoder and a transformer decoder. ONNX Runtime is provisioned automatically
on first use, mirroring how the LaTeX engine and PDF renderer are fetched.

The model graphs are produced once by the export script:

```sh
pip install torch torchvision --index-url https://download.pytorch.org/whl/cpu
pip install pix2tex onnxscript onnx
python tools/export_onnx.py
```

The script writes `encoder.onnx`, `decoder.onnx` and `resizer.onnx` into the
app's OCR model directory. Development builds keep them in the per-user data
directory (`%APPDATA%\latex-ocr` on Windows, `~/.local/share/latex-ocr` on
Linux and macOS); portable builds ship them next to the executable.

Select **Local (ONNX)** in Settings (the default). Recognition then runs
entirely offline.

### HTTP server

As an alternative, set the backend to a pix2tex-compatible server that accepts
a multipart file upload and returns LaTeX:

```sh
pip install pix2tex[gui]
python -m pix2tex.api.service
```

The default endpoint is `http://127.0.0.1:8502`; change it in Settings, where
the connection can also be tested. The app accepts servers that answer with
`{"pred": "..."}` (pix2tex), `{"latex": "..."}`, `{"text": "..."}` or a
plain-text body.

## Configuration

Settings are stored as TOML at `%APPDATA%\latex-ocr\config.toml` on Windows or
`~/.config/latex-ocr/config.toml` on Linux and macOS:

```toml
ocr_backend = "local"       # "local" for on-device ONNX (default), "http" for a pix2tex server
ocr_beautify = true         # reformat recognized LaTeX (aligned, \mathbf, line breaks)
ocr_url = "http://127.0.0.1:8502"
ocr_model_dir = ""          # optional override for the ONNX models directory
preview_debounce_ms = 600
preview_zoom = 1.0
tectonic_path = ""
```

## Development

```sh
cargo fmt --check                          # formatting
cargo clippy --all-targets -- -D warnings  # lints
cargo test                                 # unit and integration tests
```

An end-to-end render check (downloads pdfium and needs a tectonic binary) is
available but not run by default:

```sh
TECTONIC_OVERRIDE=/path/to/tectonic cargo test --test render_manual -- --ignored
```

## Platform notes

- **Windows**: screen capture works out of the box.
- **macOS**: screen capture requires the Screen Recording permission.
- **Linux (X11)**: screen capture works. On Wayland the compositor generally
  blocks global screen capture and the app reports this.

## License

The project is MIT licensed. See [LICENSE](LICENSE).

## Acknowledgements

The project relies on permissively licensed components so they can be
distributed alongside the MIT-licensed app:

- **pix2tex (LaTeX-OCR)** by Lukas Blecher, MIT. The OCR model architecture,
  checkpoints and tokenizer used by the on-device backend.
  <https://github.com/lukas-blecher/LaTeX-OCR>
- **x-transformers** by Phil Wang, MIT. The transformer layers of the OCR
  model. <https://github.com/lucidrains/x-transformers>
- **Tectonic**, MIT. The LaTeX engine used to compile previews.
- **pdfium** (Chromium), BSD-3-Clause. The PDF renderer used to rasterize
  previews.
- **ONNX Runtime** by Microsoft, MIT. Local model inference.
- **DejaVu fonts**, Bitstream Vera license. The bundled UI fonts.

The Tectonic and pdfium binaries, the ONNX Runtime library and the OCR model
weights are not part of this repository; they are downloaded or exported as
described in the Building and OCR sections.

Other math-OCR systems exist (for example Texify, whose model weights are
GPL-3.0) but are not bundled here, because they cannot be redistributed under
this project's MIT license.

## Credits

Created by Liam CORNU. The source lives at
[https://github.com/Inkapa/latex-ocr](https://github.com/Inkapa/latex-ocr).
