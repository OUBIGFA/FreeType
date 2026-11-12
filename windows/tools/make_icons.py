"""Generate FreeType icons from the square logo PNG.

Usage: python -X utf8 windows/tools/make_icons.py <logo.png> <out-dir>

Writes:
  freetype.ico   16-256 px, for the input method list, the executable and shortcuts
  tray.png       32 px, for the notification area (16 pt at 200% scale)
  tray-16.png    16 px
  logo.png       128 px, shown in the settings window
  icon.png       1024 px, the app icon mygo builds the executable icon from
"""

from pathlib import Path
import sys

from PIL import Image

ICO_SIZES = (16, 20, 24, 32, 40, 48, 64, 96, 128, 256)


def resized(image, size):
    # Resize from the full-size source each time so small icons stay sharp.
    return image.resize((size, size), Image.Resampling.LANCZOS, reducing_gap=3.0)


def main(source, out_dir):
    image = Image.open(source).convert("RGBA")
    if image.width != image.height:
        raise SystemExit(f"logo must be square, got {image.size}")
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    frames = [resized(image, size) for size in ICO_SIZES]
    frames[-1].save(
        out / "freetype.ico",
        format="ICO",
        sizes=[(size, size) for size in ICO_SIZES],
        append_images=frames[:-1],
    )
    resized(image, 32).save(out / "tray.png")
    resized(image, 16).save(out / "tray-16.png")
    resized(image, 128).save(out / "logo.png")
    resized(image, 1024).save(out / "icon.png")
    with Image.open(out / "freetype.ico") as ico:
        stored = sorted(ico.info.get("sizes", ()))
    expected = sorted((size, size) for size in ICO_SIZES)
    if stored != expected:
        raise SystemExit(f"icon sizes {stored} differ from {expected}")
    print(f"icons written to {out.as_posix()}: ico {', '.join(str(s) for s in ICO_SIZES)}")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    main(sys.argv[1], sys.argv[2])
