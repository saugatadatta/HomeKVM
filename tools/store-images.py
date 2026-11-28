"""Microsoft Store screenshots and logo images for HomeKVM. See STORE.md.

Composes the window captures from tools/store-screenshot.ps1 onto 1920x1080 backgrounds
with a short headline, and draws the poster, box art and tile icon from the app icon.
Needs Pillow (pip install pillow) and Windows' Segoe UI fonts.

    cargo run --example icon_png -- 1024 target/store-art/icon-1024.png
    python tools/store-images.py target/store-art docs/store
"""

import os
import sys

from PIL import Image, ImageDraw, ImageFilter, ImageFont

art = sys.argv[1]
out = sys.argv[2]
os.makedirs(out, exist_ok=True)
fonts = os.path.join(os.environ.get("WINDIR", r"C:\Windows"), "Fonts")


def font(name, size):
    return ImageFont.truetype(os.path.join(fonts, name), size)


REG, SEMI = "segoeui.ttf", "seguisb.ttf"
DARK = {"bg": ((10, 15, 30), (22, 48, 96)), "title": (240, 244, 250), "body": (176, 188, 208), "shadow": 120}
LIGHT = {"bg": ((246, 248, 253), (214, 226, 250)), "title": (18, 24, 34), "body": (78, 90, 110), "shadow": 50}
ACCENT = (79, 140, 255)
icon = Image.open(os.path.join(art, "icon-1024.png")).convert("RGBA")


def gradient(w, h, top, bottom):
    img = Image.new("RGB", (w, h))
    px = ImageDraw.Draw(img)
    for y in range(h):
        for_t = y / (h - 1)
        c = tuple(int(top[i] + (bottom[i] - top[i]) * for_t) for i in range(3))
        px.line([(0, y), (w, y)], fill=c)
    # A soft glow toward the top right.
    glow = Image.new("L", (w, h), 0)
    ImageDraw.Draw(glow).ellipse([w * 0.45, -h * 0.6, w * 1.4, h * 0.7], fill=60)
    glow = glow.filter(ImageFilter.GaussianBlur(160))
    img = Image.composite(Image.new("RGB", (w, h), ACCENT), img, glow)
    return img.convert("RGBA")


def rounded(img, r):
    mask = Image.new("L", img.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, img.size[0] - 1, img.size[1] - 1], r, fill=255)
    img = img.convert("RGBA")
    img.putalpha(mask)
    return img


def paste_with_shadow(canvas, img, x, y, strength):
    shadow = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle(
        [x + 6, y + 26, x + img.size[0] - 6, y + img.size[1] + 18], 18, fill=(0, 0, 0, strength)
    )
    shadow = shadow.filter(ImageFilter.GaussianBlur(28))
    canvas.alpha_composite(shadow)
    canvas.alpha_composite(img, (x, y))


def screenshot(window_file, theme, title, bullets, name):
    w, h = 1920, 1080
    canvas = gradient(w, h, *theme["bg"])
    win = Image.open(os.path.join(art, window_file)).convert("RGBA")
    scale = min(1.0, 940 / win.size[1])
    if scale < 1.0:
        win = win.resize((int(win.size[0] * scale), int(win.size[1] * scale)), Image.LANCZOS)
    win = rounded(win, 12)
    wx, wy = w - 110 - win.size[0], (h - win.size[1]) // 2
    paste_with_shadow(canvas, win, wx, wy, theme["shadow"])

    d = ImageDraw.Draw(canvas)
    left = 120
    # Brand line.
    mark = icon.resize((64, 64), Image.LANCZOS)
    canvas.alpha_composite(mark, (left, 150))
    d.text((left + 84, 182), "HomeKVM", font=font(SEMI, 34), fill=theme["title"], anchor="lm")
    # Headline and points.
    y = 290
    for line in title.split("\n"):
        d.text((left, y), line, font=font(SEMI, 64), fill=theme["title"])
        y += 82
    y += 34
    for b in bullets:
        d.ellipse([left, y + 16, left + 12, y + 28], fill=ACCENT)
        d.text((left + 30, y), b, font=font(REG, 30), fill=theme["body"])
        y += 58
    canvas.convert("RGB").save(os.path.join(out, name))
    print("saved", name, canvas.size)


screenshot(
    "window-dark.png",
    DARK,
    "One keyboard and mouse\nfor all your PCs",
    ["Move the mouse off the edge onto the next PC", "Copy on one PC, paste on another", "Text, images and files"],
    "1-screenshot-dark.png",
)
screenshot(
    "window-pair.png",
    DARK,
    "Pair your PCs with\na 6-digit code",
    ["PCs on your network show up by themselves", "Check the code on both screens, then trust", "Encrypted, no account, no cloud"],
    "2-screenshot-pairing.png",
)
screenshot(
    "window-light.png",
    LIGHT,
    "Light or dark,\nit follows Windows",
    ["Runs quietly in the tray", "No admin rights needed", "Free and open source"],
    "3-screenshot-light.png",
)


def logo_art(w, h, icon_size, title_size, tag_size, name):
    canvas = gradient(w, h, *DARK["bg"])
    d = ImageDraw.Draw(canvas)
    big = icon.resize((icon_size, icon_size), Image.LANCZOS)
    iy = int(h * 0.42) - icon_size // 2
    canvas.alpha_composite(big, ((w - icon_size) // 2, iy))
    ty = iy + icon_size + int(title_size * 0.9)
    d.text((w // 2, ty), "HomeKVM", font=font(SEMI, title_size), fill=DARK["title"], anchor="mm")
    d.multiline_text(
        (w // 2, ty + int(title_size * 1.1)),
        "One keyboard and mouse\nfor all your PCs",
        font=font(REG, tag_size),
        fill=DARK["body"],
        anchor="ma",
        align="center",
        spacing=int(tag_size * 0.35),
    )
    canvas.convert("RGB").save(os.path.join(out, name))
    print("saved", name, canvas.size)


logo_art(1440, 2160, 620, 150, 60, "poster-9x16-1440x2160.png")
logo_art(2160, 2160, 820, 180, 70, "box-art-1x1-2160x2160.png")
icon.resize((300, 300), Image.LANCZOS).save(os.path.join(out, "app-tile-icon-300x300.png"))
print("saved app-tile-icon-300x300.png (300, 300)")
