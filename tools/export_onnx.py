#!/usr/bin/env python3
"""Export the pix2tex (LaTeX-OCR) checkpoints to ONNX for the latex-ocr app.

The app runs OCR fully on-device via ONNX Runtime, so this one-time conversion
is the only step that needs Python. It produces three graphs and writes them
straight into the app's OCR model directory:

    encoder.onnx   vision transformer (image -> context)
    decoder.onnx   transformer decoder step (tokens + context -> logits)
    resizer.onnx   resolution predictor used by the pre-processing

The model weights and the tokenizer come from the pix2tex project
(https://github.com/lukas-blecher/LaTeX-OCR), which is MIT licensed. The
checkpoints are downloaded automatically on first run.

Usage:
    pip install torch torchvision --index-url https://download.pytorch.org/whl/cpu
    pip install pix2tex onnxscript onnx
    python tools/export_onnx.py
"""

import argparse
import os
import sys
import urllib.request
from pathlib import Path

import torch
import torch.nn as nn

WEIGHTS_URL = "https://github.com/lukas-blecher/LaTeX-OCR/releases/download/v0.0.1/weights.pth"
RESIZER_URL = "https://github.com/lukas-blecher/LaTeX-OCR/releases/download/v0.0.1/image_resizer.pth"


def app_data_dir() -> Path:
    """The per-user data directory, matching the app's `dirs::data_dir`."""
    if sys.platform == "win32":
        return Path(os.environ.get("APPDATA") or (Path.home() / "AppData" / "Roaming"))
    if sys.platform == "darwin":
        return Path.home() / "Library" / "Application Support"
    return Path(os.environ.get("XDG_DATA_HOME") or (Path.home() / ".local" / "share"))


class EncoderWrapper(nn.Module):
    def __init__(self, encoder):
        super().__init__()
        self.encoder = encoder

    def forward(self, x):
        return self.encoder(x)


class DecoderStep(nn.Module):
    def __init__(self, net):
        super().__init__()
        self.net = net

    def forward(self, tokens, mask, context):
        return self.net(tokens, mask=mask, context=context)


def download(url: str, dest: Path) -> None:
    if dest.exists():
        return
    print(f"downloading {url}")
    urllib.request.urlretrieve(url, dest)
    print(f"saved {dest}")


def export(model, out_path: Path, input_names, output_names, dynamic_axes, dummy):
    print(f"exporting {out_path.name} ...")
    torch.onnx.export(
        model,
        dummy,
        str(out_path),
        input_names=input_names,
        output_names=output_names,
        dynamic_axes=dynamic_axes,
        opset_version=17,
        do_constant_folding=False,
    )
    # The current exporter writes large weights to a `*.data` sidecar; fold
    # them back into the single `.onnx` file the app loads.
    import onnx

    graph = onnx.load(str(out_path))
    onnx.save_model(graph, str(out_path), save_as_external_data=False)
    sidecar = out_path.with_name(out_path.name + ".data")
    if sidecar.exists():
        sidecar.unlink()
    print(f"wrote {out_path}")


def main() -> int:
    default_output = app_data_dir() / "latex-ocr" / "ocr"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output",
        default=str(default_output),
        help=f"directory for the ONNX files (default: {default_output})",
    )
    parser.add_argument(
        "--config",
        default=None,
        help="path to the pix2tex config.yaml (auto-detected from the install)",
    )
    args = parser.parse_args()

    try:
        import yaml
        from munch import Munch
        from timm.models.layers import StdConv2dSame
        from timm.models.resnetv2 import ResNetV2
        from pix2tex.models import get_model
        from pix2tex.utils import parse_args
    except ImportError as e:
        print(
            "missing dependency: "
            f"{e}\nInstall with: pip install torch torchvision --index-url "
            "https://download.pytorch.org/whl/cpu && pip install pix2tex onnxscript onnx",
            file=sys.stderr,
        )
        return 1

    import pix2tex

    package_dir = Path(pix2tex.__file__).parent
    config_path = Path(args.config) if args.config else package_dir / "model" / "settings" / "config.yaml"
    checkpoints = app_data_dir() / "latex-ocr" / "checkpoints"
    checkpoints.mkdir(parents=True, exist_ok=True)
    download(WEIGHTS_URL, checkpoints / "weights.pth")
    download(RESIZER_URL, checkpoints / "image_resizer.pth")

    with open(config_path) as f:
        params = yaml.load(f, Loader=yaml.FullLoader)
    model_args = parse_args(Munch(params))
    model_args.wandb = False
    model_args.device = "cpu"
    model_args.checkpoint = str(checkpoints / "weights.pth")

    model = get_model(model_args)
    model.load_state_dict(torch.load(checkpoints / "weights.pth", map_location="cpu"))
    model.eval()

    resizer = ResNetV2(
        layers=[2, 3, 3],
        num_classes=max(model_args.max_dimensions) // 32,
        global_pool="avg",
        in_chans=1,
        drop_rate=0.05,
        preact=True,
        stem_type="same",
        conv_layer=StdConv2dSame,
    )
    resizer.load_state_dict(torch.load(checkpoints / "image_resizer.pth", map_location="cpu"))
    resizer.eval()

    torch.manual_seed(0)
    out_dir = Path(args.output)
    out_dir.mkdir(parents=True, exist_ok=True)

    export(
        EncoderWrapper(model.encoder).eval(),
        out_dir / "encoder.onnx",
        ["image"],
        ["context"],
        {"image": {2: "height", 3: "width"}, "context": {1: "seq"}},
        torch.zeros(1, 1, 64, 64),
    )

    export(
        DecoderStep(model.decoder.net).eval(),
        out_dir / "decoder.onnx",
        ["tokens", "mask", "context"],
        ["logits"],
        {
            "tokens": {0: "batch", 1: "tlen"},
            "mask": {0: "batch", 1: "tlen"},
            "context": {0: "batch", 1: "slen"},
            "logits": {0: "batch", 1: "tlen"},
        },
        (
            torch.zeros(1, 4, dtype=torch.long),
            torch.ones(1, 4, dtype=torch.bool),
            torch.zeros(1, 16, model_args.dim),
        ),
    )

    export(
        resizer,
        out_dir / "resizer.onnx",
        ["image"],
        ["logits"],
        {"image": {2: "height", 3: "width"}},
        torch.zeros(1, 1, 64, 64),
    )

    print(
        f"\nDone. ONNX models written to '{out_dir}'. Select 'Local (ONNX)' in "
        f"the app's Settings; ONNX Runtime is downloaded automatically on "
        f"first use."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
