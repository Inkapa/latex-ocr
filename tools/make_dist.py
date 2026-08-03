#!/usr/bin/env python3
"""Assemble the portable distribution folder for the current platform.

The portable build is a self-contained folder: the app binary, the ONNX
Runtime shared libraries and the OCR models, so nothing is downloaded at
runtime. CI calls this script for every platform and zips the result.

Usage:
    python tools/make_dist.py --exe target/release/latex-ocr --models models --output dist

The ONNX Runtime libraries are downloaded from the Microsoft releases unless
`--runtime` points at an extracted package directory.
"""

import argparse
import os
import shutil
import sys
import tarfile
import urllib.request
import zipfile
from pathlib import Path

ONNX_RUNTIME_VERSION = "1.27.0"


def onnxruntime_asset() -> tuple[str, str]:
    if sys.platform == "win32":
        return f"onnxruntime-win-x64-{ONNX_RUNTIME_VERSION}.zip", "zip"
    if sys.platform == "darwin":
        arch = "arm64" if os.uname().machine == "arm64" else "x86_64"
        return f"onnxruntime-osx-{arch}-{ONNX_RUNTIME_VERSION}.tgz", "tgz"
    return f"onnxruntime-linux-x64-{ONNX_RUNTIME_VERSION}.tgz", "tgz"


def is_library(name: str) -> bool:
    return name.endswith(".dll") or name.endswith(".so") or name.endswith(".dylib")


def copy_library_files(root: Path, dest: Path) -> None:
    libs = [p for p in root.rglob("*") if p.is_file() and is_library(p.name)]
    if not libs:
        raise SystemExit(f"no ONNX Runtime library found under {root}")
    for lib in libs:
        shutil.copy2(lib, dest / lib.name)


def fetch_onnxruntime(dest: Path) -> None:
    file, kind = onnxruntime_asset()
    url = (
        "https://github.com/microsoft/onnxruntime/releases/download/"
        f"v{ONNX_RUNTIME_VERSION}/{file}"
    )
    print(f"downloading {url}")
    archive = Path(file)
    urllib.request.urlretrieve(url, archive)
    staging = Path("_onnxruntime_staging")
    shutil.rmtree(staging, ignore_errors=True)
    staging.mkdir()
    if kind == "zip":
        with zipfile.ZipFile(archive) as z:
            z.extractall(staging)
    else:
        with tarfile.open(archive) as t:
            t.extractall(staging)
    archive.unlink()
    copy_library_files(staging, dest)
    shutil.rmtree(staging)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, help="path to the built app binary")
    parser.add_argument("--models", required=True, help="directory with the ONNX model files")
    parser.add_argument("--output", default="dist", help="directory for the distribution folder")
    parser.add_argument(
        "--runtime",
        default=None,
        help="directory with extracted ONNX Runtime libraries (default: download)",
    )
    args = parser.parse_args()

    exe = Path(args.exe)
    models = Path(args.models)
    if not exe.is_file():
        raise SystemExit(f"app binary not found: {exe}")
    for name in ("encoder.onnx", "decoder.onnx", "resizer.onnx"):
        if not (models / name).is_file():
            raise SystemExit(f"missing model file: {models / name}")

    out = Path(args.output) / "latex-ocr"
    shutil.rmtree(out, ignore_errors=True)
    (out / "models").mkdir(parents=True)

    print(f"copying {exe}")
    shutil.copy2(exe, out / exe.name)

    if args.runtime:
        copy_library_files(Path(args.runtime), out)
    else:
        fetch_onnxruntime(out)

    for name in ("encoder.onnx", "decoder.onnx", "resizer.onnx"):
        print(f"copying {name}")
        shutil.copy2(models / name, out / "models" / name)

    archive = Path(args.output) / f"latex-ocr-{sys.platform}.zip"
    print(f"archiving {out} -> {archive}")
    shutil.make_archive(str(archive.with_suffix("")), "zip", Path(args.output), "latex-ocr")
    print(f"done: {archive}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
