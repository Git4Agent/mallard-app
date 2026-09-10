"""Build the storage icon study from the saved SVG source files."""

from pathlib import Path
from html import escape
import re
import xml.etree.ElementTree as ET

ROOT = Path(__file__).parent
REPO = ROOT.parents[2]
ET.register_namespace("", "http://www.w3.org/2000/svg")


def symbol(name, markup):
    svg = ET.fromstring(markup)
    for key in ("width", "height", "role"):
        svg.attrib.pop(key, None)
    svg.tag = "symbol"
    svg.set("id", name)
    if name.startswith("pixel-") or name.startswith("current-"):
        svg.set("shape-rendering", "crispEdges")
    if name.startswith("brand-"):
        svg.set("fill", "currentColor")
        for child in list(svg):
            if child.tag.endswith("title"):
                svg.remove(child)
    return ET.tostring(svg, encoding="unicode")


symbols = [symbol(path.stem, path.read_text()) for path in sorted((ROOT / "assets").glob("*.svg"))]
source = (REPO / "src/components/Icons.tsx").read_text()
for name in ("drive", "cloud"):
    block = source.split(f'{{name === "{name}" && (', 1)[1].split(")}", 1)[0]
    paths = re.findall(r'\bd="([^"]+)"', block)
    markup = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor">'
    markup += "".join(f'<path d="{path}"/>' for path in paths) + "</svg>"
    symbols.append(symbol(f"current-{name}", markup))

definitions = "\n".join(symbols)
page = (ROOT / "template.html").read_text().replace("<!-- SVG_SYMBOLS -->", definitions)
(ROOT / "index.html").write_text(page)
print("Built index.html with", len(symbols), "SVG symbols.")


def comparison(theme):
    dark = theme == "dark"
    bg, panel, active, ink, muted, line, accent = (
        ("#15120f", "#191512", "#2d2622", "#f3ede1", "#a79e90", "#4a433b", "#bf9169")
        if dark else
        ("#fefbf6", "#fdf9f4", "#f0e3d5", "#1e1817", "#6f6159", "#d8cec4", "#8a6347")
    )
    elements = [f'<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="824" viewBox="0 0 1200 824" role="img" aria-labelledby="title description"><title id="title">Mallard storage icon choices</title><desc id="description">Three SVG icon sets: A Pixel system, B Soft outline, and C Provider marks. C is recommended for mixed storage lists. Plan and mock only.</desc><defs>{definitions}</defs><rect width="1200" height="824" fill="{bg}"/>']

    def text(x, y, value, size=14, color=ink, weight=400):
        elements.append(f'<text x="{x}" y="{y}" fill="{color}" font-family="Arial, sans-serif" font-size="{size}" font-weight="{weight}">{escape(value)}</text>')

    def rect(x, y, width, height, fill, stroke="none", radius=0):
        elements.append(f'<rect x="{x}" y="{y}" width="{width}" height="{height}" rx="{radius}" fill="{fill}" stroke="{stroke}"/>')

    def glyph(name, x, y, size, color=ink):
        elements.append(f'<use href="#{name}" x="{x}" y="{y}" width="{size}" height="{size}" color="{color}"/>')

    text(38, 32, "MALLARD / STORAGE / DESIGN STUDY", 10, muted, 600)
    text(38, 79, "Storage icon choices", 32, ink, 600)
    text(38, 105, "Source SVGs. Same list. Three visual directions.", 14, muted)
    text(1000, 42, "PLAN AND MOCK ONLY", 10, muted, 600)
    rect(38, 126, 1124, 1, line)
    text(38, 157, "CURRENT", 10, muted, 600)
    glyph("current-drive", 126, 140, 20, muted)
    text(155, 156, "Storages / Local", 12, muted)
    glyph("current-cloud", 293, 140, 20, muted)
    text(322, 156, "R2", 12, muted)
    glyph("current-cloud", 383, 140, 20, muted)
    text(411, 156, "Dropbox", 12, muted)
    text(686, 156, "Archive-shaped drive; one cloud for two providers.", 12, muted)
    sets = [
        ("A", "Pixel system", "Keep Mallard’s pixel style", ["pixel-server", "pixel-folder", "pixel-cloud", "pixel-cloud"], ["Fits the current icon family.", "R2 and Dropbox need their text labels."]),
        ("B", "Soft outline", "Use smooth vector outlines", ["outline-hard-drive", "outline-folder", "outline-cloud", "outline-cloud"], ["Smooth at small sizes.", "Changes the style beside other app icons."]),
        ("C", "Provider marks", "Keep the pixel shell; identify providers", ["pixel-server", "pixel-folder", "brand-cloudflare", "brand-dropbox"], ["Distinct shapes for R2 and Dropbox.", "Recommended for mixed storage lists."]),
    ]
    for index, (key, title, subtitle, marks, note) in enumerate(sets):
        x = 38 + index * 384
        rect(x, 187, 356, 2, accent if key == "C" else line)
        text(x, 221, key, 13, accent if key == "C" else muted, 700)
        text(x + 26, 222, title, 22, ink, 600)
        text(x, 248, subtitle, 12, muted)
        for i, (mark, label) in enumerate(zip(marks, ["Storages", "Local folder", "R2", "Dropbox"])):
            gx = x + 17 + i * 87
            glyph(mark, gx, 278, 48)
            text(gx + (0 if i == 1 else 4), 348, label, 10, muted)
        text(x, 382, "IN THE STORAGE LIST · 24 PX", 10, muted, 600)
        rect(x, 398, 356, 207, panel, line, 8)
        rect(x + 10, 408, 336, 38, active, radius=5)
        glyph(marks[0], x + 23, 415, 24)
        text(x + 59, 434, "Storages", 15, ink, 600)
        for i, (label, provider) in enumerate([("Team folder", "Local folder"), ("Design R2", "Cloudflare R2"), ("Team Dropbox", "Dropbox")]):
            y = 459 + i * 48
            glyph(marks[i + 1], x + 24, y + 2, 24)
            text(x + 61, y + 12, label, 12, ink, 600)
            text(x + 61, y + 28, provider, 10, muted)
        text(x, 640, "Storage at", 11, muted)
        for i, size in enumerate([12, 16, 20, 24]):
            gx = x + 118 + i * 54
            glyph(marks[0], gx, 628 - size / 2, size)
            text(gx - 1, 659, f"{size}px", 9, muted)
        text(x, 695, note[0], 12, ink)
        text(x, 715, note[1], 12, muted)
    rect(38, 746, 1124, 1, line)
    text(38, 774, "Guidance: Apple HIG · Material Symbols · Microsoft · WCAG 1.4.11", 11, muted)
    text(38, 796, "Geometry: Pixelarticons · Lucide · Simple Icons. Provider marks shown as a monochrome study.", 11, muted)
    elements.append("</svg>")
    (ROOT / f"comparison-{theme}.svg").write_text("\n".join(elements))


comparison("dark")
comparison("light")
print("Built two SVG comparison boards.")
