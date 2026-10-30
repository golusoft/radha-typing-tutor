"""Subset the bundled Latin UI fonts (Inter, JetBrains Mono) for the exe size budget.

Usage (fontTools 4.60.1, `pip install fonttools==4.60.1`):

    python tools/subset_fonts.py

Reads `assets/fonts/<name>.ttf` (the unmodified upstream files, kept in the
repo) and writes `assets/fonts/subset/<name>.ttf`, which `ui/theme/fonts.slint`
embeds. Only the code points below are kept: Basic Latin, Latin-1, Latin
Extended-A, the Windows-1252 extras (Kruti Dev's ANSI bytes are shown in these
faces when the Kruti Dev font is absent), punctuation and the symbol blocks the
UI uses. Cyrillic, Greek and Vietnamese are dropped; any character outside the
subset still renders through Slint's system-font fallback. Every OpenType
layout feature, the hinting and the name table are kept, so the kept glyphs
render exactly as before. Noto Sans Devanagari is NOT subset: its shaping
(GSUB/GPOS) must stay complete for correct Hindi.
"""

import pathlib

from fontTools import subset

ROOT = pathlib.Path(__file__).resolve().parent.parent
FONTS = ROOT / "assets" / "fonts"
OUT = FONTS / "subset"

FACES = [
    "Inter-Regular.ttf",
    "Inter-Medium.ttf",
    "Inter-SemiBold.ttf",
    "Inter-Bold.ttf",
    "JetBrainsMono-Regular.ttf",
    "JetBrainsMono-Bold.ttf",
]

UNICODES = (
    "U+0000-007F,"  # Basic Latin
    "U+00A0-00FF,"  # Latin-1 Supplement
    "U+0100-017F,"  # Latin Extended-A
    "U+0192,U+0218-021B,U+0237,"  # Windows-1252 f-hook, Romanian commas, dotless j
    "U+02BB-02BC,U+02C6-02C7,U+02C9,U+02D8-02DD,"  # spacing modifiers (ˆ ˇ ˘ ˙ ˚ ˛ ˜ ˝)
    "U+0300-0338,"  # combining marks (accents)
    "U+2000-206F,"  # General Punctuation (– — ‘ ’ “ ” • … ‰ ‹ ›)
    "U+2070-209F,"  # super- and subscripts
    "U+20A0-20CF,"  # currency (₹ €)
    "U+2100-218F,"  # letterlike symbols, number forms (™ №)
    "U+2190-21FF,"  # arrows (← → ⇧)
    "U+2200-22FF,"  # mathematical operators (− ≈ ≠ ≤ ≥)
    "U+2300-23FF,"  # miscellaneous technical (⌄ ⌘ ⏎)
    "U+2400-243F,"  # control pictures (␣)
    "U+2460-24FF,"  # enclosed alphanumerics
    "U+2500-259F,"  # box drawing, block elements
    "U+25A0-25FF,"  # geometric shapes (○ ●)
    "U+2600-26FF,"  # miscellaneous symbols (☀ ★)
    "U+2700-27BF,"  # dingbats (✓ ✕)
    "U+27C0-27FF,U+2900-297F,U+2B00-2BFF,"  # more arrows and symbols
    "U+FB00-FB04,U+FEFF,U+FFFD"  # ligatures, BOM, replacement character
)


def main() -> None:
    OUT.mkdir(exist_ok=True)
    for name in FACES:
        options = subset.Options()
        options.layout_features = ["*"]
        options.name_IDs = ["*"]
        options.name_languages = ["*"]
        options.notdef_outline = True
        options.hinting = True
        options.legacy_kern = True
        options.glyph_names = False
        font = subset.load_font(str(FONTS / name), options)
        sub = subset.Subsetter(options)
        sub.populate(unicodes=subset.parse_unicodes(UNICODES))
        sub.subset(font)
        target = OUT / name
        subset.save_font(font, str(target), options)
        before = (FONTS / name).stat().st_size
        print(f"{name}: {before} -> {target.stat().st_size} bytes")


if __name__ == "__main__":
    main()
