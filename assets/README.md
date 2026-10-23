# Radha Typing Tutor — Assets

All non-code resources referenced by the UI. Kept at the workspace root (not
inside `crates/radha-app`) because later phases share them with content tooling.

## Layout

```
assets/
├── fonts/     Font binaries, registered at startup
├── icons/     SVG icons, referenced directly from .slint files
└── theme/     Design-token reference (the authoritative tokens are .slint globals)
```

## Icons

SVG, referenced from `.slint` via a relative `@image-url(...)`. Slint resolves and
embeds them at compile time, so a missing or malformed icon is a build error
rather than a blank space at runtime.

Authoring rules:

- Square viewBox (`0 0 24 24`) so icons align on a shared baseline.
- `fill="currentColor"` where the icon should take its colour from the theme;
  a hard-coded fill cannot follow light/dark switching.
- No black (`#000`) and no blue fills — see `DESIGN.md` Section 2.

## Fonts

Font **binaries are not committed in Phase 0.** Three are needed, each with its
own licensing considerations:

| Font | Purpose | Licence |
|---|---|---|
| Inter | Latin UI text | SIL OFL 1.1 — redistributable |
| JetBrains Mono | Typing target/input (monospace required for character alignment) | SIL OFL 1.1 — redistributable |
| Noto Sans Devanagari | Hindi GAIL/CBI (Unicode) | SIL OFL 1.1 — redistributable |
| Kruti Dev | Hindi Kruti Dev (legacy encoding) | Must be licensed/verified before bundling |

Until they are added, the app registers whatever `.ttf`/`.otf` files it finds in
`fonts/` and otherwise falls back to a system font. The registration code path is
live now (see `ui.rs::register_fonts`), so dropping the files in is the only
remaining step — no code change required.

`PRD.md` Section 12.5 is explicit that Kruti Dev must be bundled rather than
assumed present on the user's machine, since it is not a standard Windows font.
