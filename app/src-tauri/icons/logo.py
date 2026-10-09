"""Generates the Cairn logo.

  python3 logo.py          writes icon.svg (app icon: terracotta on charcoal), icon-alt.svg (secondary:
                           cream on terracotta) and logo-mark.svg (transparent mark)
  python3 logo.py --build  also regenerates every platform icon: runs `tauri icon` on icon.svg, then
                           redoes iOS full-bleed (iOS masks the corners itself), the Android
                           adaptive icon (charcoal background + mark inside the 66dp safe zone) and
                           the Windows installer images
  python3 logo.py --installer  only redoes the Windows installer images: nsis-header.bmp (top of
                           each page) and nsis-sidebar.bmp (welcome and finish pages)
Needs rsvg-convert and the app's node_modules.
"""
import os, shutil, subprocess, tempfile, struct, zlib
import math, sys, json

def blob(cx, cy, a, b, rot=0, p=2.6, harm=(), n=36, squash_bottom=0.0):
    """Closed rock outline: superellipse with low-frequency wobble."""
    pts = []
    for i in range(n):
        t = 2*math.pi*i/n
        c, s = math.cos(t), math.sin(t)
        x = a*math.copysign(abs(c)**(2/p), c)
        y = b*math.copysign(abs(s)**(2/p), s)
        r = 1.0
        for k, amp, ph in harm:
            r += amp*math.cos(k*t+ph)
        x *= r; y *= r
        if y > 0: y *= (1 - squash_bottom*abs(s))
        ra = math.radians(rot)
        xr = x*math.cos(ra) - y*math.sin(ra)
        yr = x*math.sin(ra) + y*math.cos(ra)
        pts.append((cx+xr, cy+yr))
    return pts

def path(pts):
    # Catmull-Rom -> cubic bezier, closed
    n = len(pts); d = f"M{pts[0][0]:.0f} {pts[0][1]:.0f}"
    for i in range(n):
        p0, p1, p2, p3 = pts[i-1], pts[i], pts[(i+1)%n], pts[(i+2)%n]
        c1 = (p1[0]+(p2[0]-p0[0])/6, p1[1]+(p2[1]-p0[1])/6)
        c2 = (p2[0]-(p3[0]-p1[0])/6, p2[1]-(p3[1]-p1[1])/6)
        d += f"C{c1[0]:.0f} {c1[1]:.0f} {c2[0]:.0f} {c2[1]:.0f} {p2[0]:.0f} {p2[1]:.0f}"
    return d + "Z"

# bottom -> top. cy of each upper stone is solved so it rests OVERLAP px into the one below.
SPEC = [
    dict(cx=500, a=300, b=150, rot=-4, p=2.2, harm=[(2,0.035,0.6),(3,0.04,2.2),(5,0.01,1.0)], squash_bottom=0.22),
    dict(cx=530, a=215, b=105, rot=5,  p=2.3, harm=[(2,0.03,2.0),(3,0.045,0.3),(4,0.012,1.7)], squash_bottom=0.12),
    dict(cx=505, a=160, b=78,  rot=-6, p=2.3, harm=[(2,0.035,1.1),(3,0.035,2.8)], squash_bottom=0.1),
    dict(cx=528, a=104, b=58,  rot=7,  p=2.1, harm=[(2,0.05,0.2),(3,0.03,1.5)]),
]
OVERLAP = 30
def stack(spec, base_cy=720):
    out = [dict(spec[0], cy=base_cy)]
    for s in spec[1:]:
        lower = blob(**out[-1])
        trial = dict(s, cy=0)
        up = blob(**trial)
        x0 = min(p[0] for p in up); x1 = max(p[0] for p in up)
        # top surface of lower stone under the upper stone's middle half
        mid = [(p[0],p[1]) for p in lower if x0+(x1-x0)*.3 < p[0] < x0+(x1-x0)*.7 and p[1] < out[-1]["cy"]]
        top = min(p[1] for p in mid)
        bottom = max(p[1] for p in up)
        out.append(dict(s, cy=top - bottom + OVERLAP))
    # vertically centre the whole stack
    pts = [p for st in out for p in blob(**st)]
    y0 = min(p[1] for p in pts); y1 = max(p[1] for p in pts)
    shift = 512 - (y0+y1)/2
    for st in out: st["cy"] += shift
    return out
STONES = stack(SPEC)
def svg(fill, bg=None, gap=20, rx=200, inset=64, shapes=STONES, size=1024, shades=None, scale=1.0, dy=0, ids="cairn"):
    """Each stone is defined once; the stones above it are drawn into its mask with a thick
    stroke, which carves the gap. `ids` prefixes element ids so the SVG can be inlined in HTML."""
    out = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {size} {size}">']
    if bg: out.append(f'  <rect x="{inset}" y="{inset}" width="{size-2*inset}" height="{size-2*inset}" rx="{rx}" fill="{bg}"/>')
    paths = [path(blob(**s)) for s in shapes]
    out.append('  <defs>')
    for i, d in enumerate(paths):
        out.append(f'    <path id="{ids}-s{i}" d="{d}"/>')
    for i in range(len(paths)-1):
        out.append(f'    <mask id="{ids}-m{i}" maskUnits="userSpaceOnUse" x="0" y="0" width="{size}" height="{size}">')
        out.append(f'      <rect width="{size}" height="{size}" fill="#fff"/>')
        for j in range(i+1, len(paths)):
            out.append(f'      <use href="#{ids}-s{j}" stroke="#000" stroke-width="{2*gap}" stroke-linejoin="round"/>')
        out.append('    </mask>')
    out.append('  </defs>')
    tx = size/2*(1-scale)
    out.append(f'  <g transform="translate({tx:g} {tx+dy:g}) scale({scale})">')
    for i in range(len(paths)):
        f = shades[i] if shades else fill
        m = f' mask="url(#{ids}-m{i})"' if i < len(paths)-1 else ''
        out.append(f'    <use href="#{ids}-s{i}" fill="{f}"{m}/>')
    out.append('  </g>')
    out.append('</svg>')
    return "\n".join(out)

TERRACOTTA, BRIGHT_TERRACOTTA, CHARCOAL, CREAM = "#c4583a", "#e06d45", "#25282d", "#fbfaf7"
HERE = os.path.dirname(os.path.abspath(__file__))
IOS = {"20x20@1x": 20, "20x20@2x": 40, "20x20@2x-1": 40, "20x20@3x": 60, "29x29@1x": 29, "29x29@2x": 58,
       "29x29@2x-1": 58, "29x29@3x": 87, "40x40@1x": 40, "40x40@2x": 80, "40x40@2x-1": 80, "40x40@3x": 120,
       "60x60@2x": 120, "60x60@3x": 180, "76x76@1x": 76, "76x76@2x": 152, "83.5x83.5@2x": 167, "512@2x": 1024}
ANDROID = {"mdpi": 108, "hdpi": 162, "xhdpi": 216, "xxhdpi": 324, "xxxhdpi": 432}

def render(svg_text, px, out):
    with tempfile.NamedTemporaryFile("w", suffix=".svg", delete=False) as f:
        f.write(svg_text)
    subprocess.run(["rsvg-convert", "-w", str(px), "-h", str(px), f.name, "-o", out], check=True)
    os.unlink(f.name)

def png_rgb(png):
    """Pixel rows of an 8-bit RGB or RGBA PNG (what rsvg-convert writes), as RGB."""
    data = open(png, "rb").read()
    pos, idat = 8, b""
    while pos < len(data):
        n, kind = struct.unpack_from(">I4s", data, pos)
        body = data[pos+8:pos+8+n]
        if kind == b"IHDR":
            w, h, depth, ctype = struct.unpack_from(">IIBB", body)
            assert depth == 8 and ctype in (2, 6), "expected an 8-bit RGB or RGBA PNG"
            px = 4 if ctype == 6 else 3
        elif kind == b"IDAT":
            idat += body
        pos += 12 + n
    raw, stride, rows = zlib.decompress(idat), w*px, []
    prev = bytearray(stride)
    for y in range(h):
        f, line = raw[y*(stride+1)], bytearray(raw[y*(stride+1)+1:(y+1)*(stride+1)])
        for i in range(stride):
            a = line[i-px] if i >= px else 0
            b = prev[i]
            c = prev[i-px] if i >= px else 0
            if f == 1: line[i] = (line[i] + a) & 255
            elif f == 2: line[i] = (line[i] + b) & 255
            elif f == 3: line[i] = (line[i] + (a + b)//2) & 255
            elif f == 4:
                pa, pb, pc = abs(b-c), abs(a-c), abs(a+b-2*c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(bytes(v for i in range(0, stride, px) for v in line[i:i+3]))
        prev = line
    return w, h, rows

def write_bmp(png, out):
    """NSIS takes only BMP images: 24-bit, rows bottom-up, each padded to 4 bytes."""
    w, h, rows = png_rgb(png)
    pad = b"\0" * (-w*3 % 4)
    pixels = b"".join(bytes(row[i+2-j] for i in range(0, w*3, 3) for j in range(3)) + pad for row in reversed(rows))
    head = struct.pack("<2sIHHI", b"BM", 54 + len(pixels), 0, 0, 54)
    info = struct.pack("<IiiHHIIiiII", 40, w, h, 1, 24, 0, len(pixels), 2835, 2835, 0, 0)
    open(out, "wb").write(head + info + pixels)

def installer():
    """The mark on the Windows installer's pages: small on white in the header (150x57, which
    NSIS puts left of the page title), on charcoal in the sidebar of the welcome and finish
    pages (164x314). The backgrounds are opaque, so the PNG's alpha can be dropped."""
    mark = svg(BRIGHT_TERRACOTTA)
    for name, w, h, bg, size, x, y in (("nsis-header.bmp", 150, 57, "#ffffff", 57, 8, 0),
                                       ("nsis-sidebar.bmp", 164, 314, CHARCOAL, 128, 18, 72)):
        page = (f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}">'
                f'<rect width="{w}" height="{h}" fill="{bg}"/>'
                f'<svg x="{x}" y="{y}" width="{size}" height="{size}"{mark[4:]}</svg>')
        with tempfile.TemporaryDirectory() as tmp:
            src, png = os.path.join(tmp, "page.svg"), os.path.join(tmp, "page.png")
            open(src, "w").write(page)
            subprocess.run(["rsvg-convert", src, "-o", png], check=True)
            write_bmp(png, os.path.join(HERE, name))

def build():
    app = os.path.join(HERE, "..", "..")
    with tempfile.TemporaryDirectory() as tmp:
        src = os.path.join(tmp, "icon.png")
        render(svg(BRIGHT_TERRACOTTA, bg=CHARCOAL, scale=0.88), 1024, src)
        subprocess.run(["npx", "tauri", "icon", src], cwd=app, check=True)
    full = svg(BRIGHT_TERRACOTTA, bg=CHARCOAL, inset=0, rx=0, scale=0.8)
    for name, px in IOS.items():
        render(full, px, os.path.join(HERE, "ios", f"AppIcon-{name}.png"))
    # `tauri icon` writes Android launcher icons only into gen/android once that project exists;
    # mirror them so icons/android doesn't keep a stale copy.
    gen_res = os.path.join(app, "src-tauri/gen/android/app/src/main/res")
    if os.path.isdir(gen_res):
        for dpi in ANDROID:
            for name in ("ic_launcher.png", "ic_launcher_round.png"):
                shutil.copyfile(os.path.join(gen_res, f"mipmap-{dpi}", name), os.path.join(HERE, "android", f"mipmap-{dpi}", name))
    fg = svg(BRIGHT_TERRACOTTA, scale=0.78)
    res_dirs = [os.path.join(HERE, "android"), os.path.join(app, "src-tauri/gen/android/app/src/main/res")]
    for res in res_dirs:
        if not os.path.isdir(res):
            continue
        for dpi, px in ANDROID.items():
            render(fg, px, os.path.join(res, f"mipmap-{dpi}", "ic_launcher_foreground.png"))
        with open(os.path.join(res, "values", "ic_launcher_background.xml"), "w") as f:
            f.write('<?xml version="1.0" encoding="utf-8"?>\n<resources>\n'
                    f'  <color name="ic_launcher_background">{CHARCOAL}</color>\n</resources>\n')
    installer()

if __name__ == "__main__":
    open(os.path.join(HERE, "icon.svg"), "w").write(svg(BRIGHT_TERRACOTTA, bg=CHARCOAL, scale=0.88) + "\n")
    open(os.path.join(HERE, "icon-alt.svg"), "w").write(svg(CREAM, bg=TERRACOTTA, scale=0.88) + "\n")
    open(os.path.join(HERE, "logo-mark.svg"), "w").write(svg(BRIGHT_TERRACOTTA) + "\n")
    if "--build" in sys.argv:
        build()
    elif "--installer" in sys.argv:
        installer()
