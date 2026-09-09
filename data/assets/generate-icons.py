#!/usr/bin/env python3
"""Generate Halogen's PNG/ICO assets from icon.svg. Requires Pillow.

Run without arguments for every platform, or --ios-only to sync AppIcon.
"""

import argparse
import json
import shutil
import xml.etree.ElementTree as ET
from pathlib import Path

from PIL import Image, ImageDraw

OUT = Path(__file__).resolve().parent
IOS_ASSETS = OUT.parents[1] / "ios" / "Halogen" / "Assets.xcassets"
CANVAS = 1024
BACKGROUND = "#0B0E14"


def render(size: int, scale: float = 1.0) -> Image.Image:
    """Render the source mark with optional padding for launcher masks."""
    supersample = 4
    image = Image.new("RGB", (CANVAS * supersample, CANVAS * supersample), BACKGROUND)
    draw = ImageDraw.Draw(image)

    def coordinate(value: float) -> float:
        return (CANVAS / 2 + (value - CANVAS / 2) * scale) * supersample

    def shape(element: ET.Element, inherited_fill: str = BACKGROUND) -> None:
        kind = element.tag.rsplit("}", 1)[-1]
        fill = element.get("fill", inherited_fill)
        if kind in ("svg", "g"):
            for child in element:
                shape(child, fill)
            return
        if kind == "rect":
            x, y = float(element.get("x", 0)), float(element.get("y", 0))
            width, height = float(element.attrib["width"]), float(element.attrib["height"])
            if x == y == 0 and width == height == CANVAS:
                draw.rectangle((0, 0, CANVAS * supersample, CANVAS * supersample), fill=fill)
                return
            box = (coordinate(x), coordinate(y), coordinate(x + width), coordinate(y + height))
            draw.rounded_rectangle(box, radius=float(element.get("rx", 0)) * scale * supersample, fill=fill)
        elif kind == "circle":
            x, y, radius = (float(element.attrib[key]) for key in ("cx", "cy", "r"))
            draw.ellipse(
                (coordinate(x - radius), coordinate(y - radius), coordinate(x + radius), coordinate(y + radius)),
                fill=fill,
            )
        else:
            raise ValueError(f"Unsupported icon.svg element: {kind}")

    source = ET.parse(OUT / "icon.svg").getroot()
    if source.get("viewBox") != "0 0 1024 1024":
        raise ValueError("icon.svg must use a 1024x1024 viewBox")
    shape(source)
    return image.resize((size, size), Image.Resampling.LANCZOS)


def sync_ios_appicon() -> None:
    appicon = IOS_ASSETS / "AppIcon.appiconset"
    appicon.mkdir(parents=True, exist_ok=True)
    contents = {
        "images": [{"filename": "icon-1024.png", "idiom": "universal", "platform": "ios", "size": "1024x1024"}],
        "info": {"author": "xcode", "version": 1},
    }
    (appicon / "Contents.json").write_text(json.dumps(contents, indent=2) + "\n")
    shutil.copyfile(OUT / "icon.png", appicon / "icon-1024.png")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ios-only", action="store_true")
    args = parser.parse_args()
    if not args.ios_only:
        render(1024).save(OUT / "icon.png")
        for size in (192, 512):
            render(size).save(OUT / f"icon-{size}.png")
            render(size, scale=0.72).save(OUT / f"icon-maskable-{size}.png")
        render(180).save(OUT / "apple-touch-icon.png")
        render(64).save(OUT / "favicon.ico", sizes=[(16, 16), (32, 32), (48, 48)])
    sync_ios_appicon()
    print("Updated Halogen icons in", IOS_ASSETS if args.ios_only else OUT)


if __name__ == "__main__":
    main()
