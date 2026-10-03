"""Draws the app icon (a red microphone in front of a singer's lips) and writes assets/icon.ico and assets/icon.png.

Needs Pillow (pip install pillow). Run from the project folder: python tools/make_icon.py
"""
import math
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

SIZE = 1024          # design canvas
SS = 2               # supersampling factor
S = SIZE * SS
OUT = Path(__file__).resolve().parent.parent / "assets"


def pt(x, y):
    return (x * SS, y * SS)


def bezier(p0, p1, p2, p3, n=60):
    out = []
    for i in range(n + 1):
        t = i / n
        a, b, c, d = (1 - t) ** 3, 3 * (1 - t) ** 2 * t, 3 * (1 - t) * t ** 2, t ** 3
        out.append((a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0], a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1]))
    return out


def path(*segments):
    """Join bezier segments (each 4 points) into one polyline."""
    pts = []
    for seg in segments:
        pts += bezier(*seg)
    return [pt(x, y) for x, y in pts]


def lerp_color(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


def vertical_gradient(box, top, bottom, mask):
    """Fill the masked area with a vertical gradient."""
    x0, y0, x1, y1 = [int(v) for v in box]
    grad = Image.new("RGB", (x1 - x0, y1 - y0))
    d = ImageDraw.Draw(grad)
    for y in range(y1 - y0):
        d.line([(0, y), (x1 - x0, y)], fill=lerp_color(top, bottom, y / max(1, y1 - y0 - 1)))
    return grad, (x0, y0)


img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
draw = ImageDraw.Draw(img)

# Background: rounded square in the app's deep midnight blue
bg = Image.new("RGBA", (S, S), (0, 0, 0, 0))
ImageDraw.Draw(bg).rounded_rectangle([pt(24, 24), pt(1000, 1000)], radius=190 * SS, fill=(11, 17, 32, 255))
# soft glow behind the lips
glow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
ImageDraw.Draw(glow).ellipse([pt(120, 160), pt(904, 760)], fill=(236, 72, 153, 70))
glow = glow.filter(ImageFilter.GaussianBlur(70 * SS))
bg = Image.alpha_composite(bg, glow)
mask = Image.new("L", (S, S), 0)
ImageDraw.Draw(mask).rounded_rectangle([pt(24, 24), pt(1000, 1000)], radius=190 * SS, fill=255)
img = bg

draw = ImageDraw.Draw(img)

# ---------------------------------------------------------------- lips (behind the microphone)
LIP_LIGHT = (255, 120, 150)
LIP_DARK = (214, 52, 92)
MOUTH = (74, 12, 30)

# Whole lips silhouette (open mouth singing): upper lip with a cupid's bow, fuller lower lip
left, right, mid_y = (110, 470), (914, 470), 470
upper_outline = path(
    (left, (150, 420), (250, 300), (330, 290)),                      # left corner up to the left peak
    ((330, 290), (400, 285), (450, 330), (512, 335)),                  # peak down into the bow
    ((512, 335), (574, 330), (624, 285), (694, 290)),                  # bow up to the right peak
    ((694, 290), (774, 300), (874, 420), right),                       # right peak down to the corner
)
lower_outline = path(
    (right, (860, 640), (720, 760), (512, 768)),                       # corner round to the bottom
    ((512, 768), (304, 760), (164, 640), left),                        # bottom back to the left corner
)
# Open mouth between the lips
mouth_top = path((left, (230, 520), (390, 500), (512, 500)), ((512, 500), (634, 500), (794, 520), right))
mouth_bottom = path((right, (800, 600), (650, 640), (512, 640)), ((512, 640), (374, 640), (224, 600), left))

lips_mask = Image.new("L", (S, S), 0)
ImageDraw.Draw(lips_mask).polygon(upper_outline + lower_outline, fill=255)
lips_img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
grad, origin = vertical_gradient((0, 250 * SS, S, 780 * SS), LIP_LIGHT, LIP_DARK, None)
lips_img.paste(grad, (0, 250 * SS))
lips_img.putalpha(lips_mask)
img = Image.alpha_composite(img, lips_img)
draw = ImageDraw.Draw(img)
draw.polygon(mouth_top + mouth_bottom, fill=MOUTH)
# teeth
draw.polygon(path(((250, 512), (330, 506), (430, 504), (512, 504)), ((512, 504), (594, 504), (694, 506), (774, 512)))
             + path(((774, 512), (700, 556), (610, 566), (512, 566)), ((512, 566), (414, 566), (324, 556), (250, 512))),
             fill=(250, 244, 240))
# lip outlines / highlights
draw.line(upper_outline, fill=(150, 30, 66), width=int(7 * SS), joint="curve")
draw.line(lower_outline, fill=(150, 30, 66), width=int(7 * SS), joint="curve")
hl = path(((330, 330), (380, 318), (440, 345), (480, 362)))
draw.line(hl, fill=(255, 190, 205), width=int(14 * SS), joint="curve")
hl2 = path(((330, 700), (400, 735), (470, 744), (540, 744)))
draw.line(hl2, fill=(255, 170, 190), width=int(14 * SS), joint="curve")

# ---------------------------------------------------------------- microphone (in front)
RED = (232, 30, 48)
RED_DARK = (150, 10, 28)
RED_LIGHT = (255, 120, 120)
SILVER = (232, 237, 244)
SILVER_DARK = (125, 135, 155)
OUTLINE = (84, 6, 18)

cx, cy, r = 512, 515, 188   # head

def paste_gradient(img, shape_mask, top, bottom, y0, y1):
    layer = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    grad, _ = vertical_gradient((0, y0 * SS, S, y1 * SS), top, bottom, None)
    layer.paste(grad, (0, int(y0 * SS)))
    layer.putalpha(shape_mask)
    return Image.alpha_composite(img, layer)

# handle: tapered with a rounded end, running to the bottom of the icon
hw_top, hw_bot, h_top, h_bot = 78, 54, cy + r - 10, 968
handle_poly = [pt(cx - hw_top, h_top), pt(cx + hw_top, h_top), pt(cx + hw_bot, h_bot), pt(cx - hw_bot, h_bot)]
h_mask = Image.new("L", (S, S), 0)
hd = ImageDraw.Draw(h_mask)
hd.polygon(handle_poly, fill=255)
hd.ellipse([pt(cx - hw_bot, h_bot - hw_bot), pt(cx + hw_bot, h_bot + hw_bot)], fill=255)
img = paste_gradient(img, h_mask, RED, RED_DARK, h_top, h_bot + hw_bot)
draw = ImageDraw.Draw(img)
draw.line([handle_poly[0], handle_poly[3]], fill=OUTLINE, width=int(7 * SS))
draw.line([handle_poly[1], handle_poly[2]], fill=OUTLINE, width=int(7 * SS))
draw.arc([pt(cx - hw_bot, h_bot - hw_bot), pt(cx + hw_bot, h_bot + hw_bot)], 0, 180, fill=OUTLINE, width=int(7 * SS))
# highlight stripe on the handle
draw.line([pt(cx - 36, h_top + 80), pt(cx - 26, h_bot - 40)], fill=(255, 140, 140), width=int(14 * SS))
# a button ring on the handle
draw.rounded_rectangle([pt(cx - 70, 800), pt(cx + 70, 836)], radius=18 * SS, fill=SILVER, outline=SILVER_DARK, width=int(5 * SS))

# head: shadow, red sphere with a gradient, grille lines, highlight
shadow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
ImageDraw.Draw(shadow).ellipse([pt(cx - r + 14, cy - r + 30), pt(cx + r + 14, cy + r + 30)], fill=(0, 0, 0, 130))
img = Image.alpha_composite(img, shadow.filter(ImageFilter.GaussianBlur(22 * SS)))
sphere = Image.new("RGBA", (S, S), (0, 0, 0, 0))
sd = ImageDraw.Draw(sphere)
steps = 100
for i in range(steps):
    t = i / (steps - 1)
    rr = r * (1 - t * 0.97)
    ox, oy = -r * 0.30 * t, -r * 0.34 * t
    col = lerp_color(RED_DARK, RED_LIGHT, t ** 1.7)
    sd.ellipse([pt(cx + ox - rr, cy + oy - rr), pt(cx + ox + rr, cy + oy + rr)], fill=col)
sphere_mask = Image.new("L", (S, S), 0)
ImageDraw.Draw(sphere_mask).ellipse([pt(cx - r, cy - r), pt(cx + r, cy + r)], fill=255)
sphere.putalpha(sphere_mask)
img = Image.alpha_composite(img, sphere)

# mesh grille, clipped to the sphere (fewer, lighter lines than before)
grille = Image.new("RGBA", (S, S), (0, 0, 0, 0))
gd = ImageDraw.Draw(grille)
line = (120, 8, 24, 120)
for k in (-1, 0, 1):
    x = cx + k * r * 0.48
    half = r * math.sqrt(max(0.0, 1 - ((x - cx) / r) ** 2))
    gd.line([pt(x, cy - half), pt(x, cy + half)], fill=line, width=int(8 * SS))
    y = cy + k * r * 0.48
    half = r * math.sqrt(max(0.0, 1 - ((y - cy) / r) ** 2))
    gd.line([pt(cx - half, y), pt(cx + half, y)], fill=line, width=int(8 * SS))
grille.putalpha(Image.composite(grille.getchannel("A"), Image.new("L", (S, S), 0), sphere_mask))
img = Image.alpha_composite(img, grille)
draw = ImageDraw.Draw(img)
draw.ellipse([pt(cx - r, cy - r), pt(cx + r, cy + r)], outline=OUTLINE, width=int(8 * SS))

# silver collar where the head meets the handle (drawn over both)
collar = [pt(cx - 120, cy + r - 52), pt(cx + 120, cy + r - 52)]
draw.rounded_rectangle([pt(cx - 112, cy + r - 44), pt(cx + 112, cy + r + 30)], radius=30 * SS, fill=SILVER, outline=SILVER_DARK, width=int(6 * SS))
draw.line([pt(cx - 86, cy + r - 22), pt(cx + 86, cy + r - 22)], fill=(255, 255, 255), width=int(8 * SS))
draw.line([pt(cx - 86, cy + r + 8), pt(cx + 86, cy + r + 8)], fill=(170, 180, 198), width=int(6 * SS))

# glossy highlight on the head
hi = Image.new("RGBA", (S, S), (0, 0, 0, 0))
ImageDraw.Draw(hi).ellipse([pt(cx - 112, cy - 140), pt(cx - 30, cy - 80)], fill=(255, 255, 255, 170))
img = Image.alpha_composite(img, hi.filter(ImageFilter.GaussianBlur(8 * SS)))

# clip everything to the rounded square
final = Image.new("RGBA", (S, S), (0, 0, 0, 0))
final.paste(img, (0, 0), mask)
final = final.resize((SIZE, SIZE), Image.LANCZOS)

OUT.mkdir(exist_ok=True)
final.resize((256, 256), Image.LANCZOS).save(OUT / "icon.png")
final.save(OUT / "icon.ico", format="ICO", sizes=[(256, 256), (128, 128), (64, 64), (48, 48), (32, 32), (24, 24), (16, 16)])
final.save(OUT / "icon_preview_1024.png")
print("wrote", OUT / "icon.ico")
