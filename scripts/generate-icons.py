#!/usr/bin/env python3
"""Generate original clean-room Local Humanizer app icons using Pillow."""

import argparse
from pathlib import Path
from PIL import Image, ImageDraw

ROOT_DIR = Path(__file__).resolve().parent.parent
MASTER_PATH = ROOT_DIR / "scripts" / "assets" / "icon-1024.png"
ICONS_DIR = ROOT_DIR / "src-tauri" / "icons"


def lerp_color(c1: tuple[int, int, int], c2: tuple[int, int, int], t: float) -> tuple[int, int, int, int]:
    t = max(0.0, min(1.0, t))
    return (
        int(c1[0] + (c2[0] - c1[0]) * t),
        int(c1[1] + (c2[1] - c1[1]) * t),
        int(c1[2] + (c2[2] - c1[2]) * t),
        255,
    )


def render_master_image(size: int = 1024) -> Image.Image:
    scale = 2
    w = size * scale
    h = size * scale

    # 1. Background squircle with diagonal slate gradient (#0F172A -> #1E293B)
    canvas = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    grad = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    grad_draw = ImageDraw.Draw(grad)
    c_top = (15, 23, 42)
    c_bot = (30, 41, 59)
    for y in range(h):
        color = lerp_color(c_top, c_bot, y / max(1, h - 1))
        grad_draw.line([(0, y), (w, y)], fill=color)

    pad = int(w * 0.05)
    radius = int(w * 0.22)
    mask = Image.new("L", (w, h), 0)
    mask_draw = ImageDraw.Draw(mask)
    mask_draw.rounded_rectangle([pad, pad, w - pad, h - pad], radius=radius, fill=255)
    canvas.paste(grad, (0, 0), mask)

    draw = ImageDraw.Draw(canvas)
    # Subtle slate inner border (#334155)
    border_w = max(2, int(w * 0.018))
    draw.rounded_rectangle(
        [pad, pad, w - pad, h - pad],
        radius=radius,
        outline=(51, 65, 85, 255),
        width=border_w,
    )

    # 2. Emerald-teal accent layer for dual-arc quill nib + three ascending prose bars
    accent_grad = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    accent_draw = ImageDraw.Draw(accent_grad)
    c_teal = (20, 184, 166)
    c_emerald = (52, 211, 153)
    for x in range(w):
        color = lerp_color(c_teal, c_emerald, x / max(1, w - 1))
        accent_draw.line([(x, 0), (x, h)], fill=color)

    motif_mask = Image.new("L", (w, h), 0)
    md = ImageDraw.Draw(motif_mask)

    def pt(nx: float, ny: float) -> tuple[float, float]:
        return (nx * w, ny * h)

    # Smooth cubic Bezier sampling for the dual-arc quill nib
    def cubic_bezier(p0, p1, p2, p3, steps=48):
        pts = []
        for i in range(steps + 1):
            t = i / steps
            mt = 1.0 - t
            x = (mt**3) * p0[0] + 3 * (mt**2) * t * p1[0] + 3 * mt * (t**2) * p2[0] + (t**3) * p3[0]
            y = (mt**3) * p0[1] + 3 * (mt**2) * t * p1[1] + 3 * mt * (t**2) * p2[1] + (t**3) * p3[1]
            pts.append((x, y))
        return pts

    upper_arc = cubic_bezier(pt(0.28, 0.72), pt(0.28, 0.48), pt(0.42, 0.28), pt(0.67, 0.22))
    lower_arc = cubic_bezier(pt(0.67, 0.22), pt(0.61, 0.42), pt(0.51, 0.56), pt(0.36, 0.64))
    nib_poly = upper_arc + lower_arc[1:] + [pt(0.28, 0.72)]
    md.polygon(nib_poly, fill=255)

    # Nib slit & breather hole carved out
    md.line([pt(0.28, 0.72), pt(0.45, 0.47)], fill=0, width=int(w * 0.026))
    cx, cy = pt(0.475, 0.435)
    r_hole = w * 0.028
    md.ellipse([cx - r_hole, cy - r_hole, cx + r_hole, cy + r_hole], fill=0)

    # Three ascending prose / waveform bars
    bars = [
        (0.515, 0.59, 0.578, 0.735),
        (0.617, 0.48, 0.680, 0.735),
        (0.719, 0.37, 0.782, 0.735),
    ]
    bar_radius = int(w * 0.031)
    for x0, y0, x1, y1 in bars:
        md.rounded_rectangle([x0 * w, y0 * h, x1 * w, y1 * h], radius=bar_radius, fill=255)

    canvas.paste(accent_grad, (0, 0), motif_mask)
    return canvas.resize((size, size), Image.Resampling.LANCZOS)


def write_fallback_icons(master: Image.Image) -> None:
    ICONS_DIR.mkdir(parents=True, exist_ok=True)
    master.resize((32, 32), Image.Resampling.LANCZOS).save(ICONS_DIR / "32x32.png", format="PNG")
    master.resize((128, 128), Image.Resampling.LANCZOS).save(ICONS_DIR / "128x128.png", format="PNG")
    master.resize((256, 256), Image.Resampling.LANCZOS).save(ICONS_DIR / "128x128@2x.png", format="PNG")
    master.resize((512, 512), Image.Resampling.LANCZOS).save(ICONS_DIR / "icon.png", format="PNG")
    master.save(
        ICONS_DIR / "icon.ico",
        format="ICO",
        sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )
    master.save(ICONS_DIR / "icon.icns", format="ICNS")


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate Local Humanizer master PNG and optional fallback icons.")
    parser.add_argument("--fallback-pillow", action="store_true", help="Also emit Tauri icon set directly via Pillow")
    args = parser.parse_args()

    MASTER_PATH.parent.mkdir(parents=True, exist_ok=True)
    master = render_master_image(1024)
    master.save(MASTER_PATH, format="PNG")
    print(f"Generated {MASTER_PATH} ({master.size[0]}x{master.size[1]} {master.mode})")

    if args.fallback_pillow:
        write_fallback_icons(master)
        print(f"Generated fallback icons in {ICONS_DIR}")


if __name__ == "__main__":
    main()
