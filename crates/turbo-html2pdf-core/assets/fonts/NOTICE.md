# Bundled fonts — attribution & licenses

These fonts are embedded into `turbo-html2pdf-core` when the default `bundled-fonts`
feature is enabled (see turbo-dom-layout's `src/text/bundled.rs`). They back the CSS generic
families so a document renders with zero caller-supplied fonts. Every face below
is redistributed under a permissive open license; the full upstream license text
ships alongside each family.

| CSS generic | Role      | Family          | Faces bundled                     | License | License file                  |
|-------------|-----------|-----------------|-----------------------------------|---------|-------------------------------|
| sans-serif  | primary   | Inter           | Regular, Bold, Italic, BoldItalic | OFL 1.1 | `inter/LICENSE.txt`           |
| sans-serif  | secondary | Roboto          | Regular, Bold, Italic, BoldItalic | OFL 1.1 | `roboto/OFL.txt`              |
| serif       | primary   | Liberation Serif| Regular, Bold, Italic, BoldItalic | OFL 1.1 | `liberation-serif/LICENSE`    |
| serif       | secondary | PT Serif        | Regular, Bold, Italic, BoldItalic | OFL 1.1 | `pt-serif/OFL.txt`            |
| monospace   | primary   | Fira Code       | Regular, Bold                     | OFL 1.1 | `fira-code/OFL.txt`           |
| monospace   | secondary | IBM Plex Mono   | Regular, Bold, Italic, BoldItalic | OFL 1.1 | `ibm-plex-mono/OFL.txt`       |

Notes:

- Fira Code ships no italic faces upstream (it is a programming-ligature
  monospace), so only Regular and Bold are bundled; italic monospace falls
  through to IBM Plex Mono Italic / BoldItalic.
- Roboto is the current Google Fonts release, which is distributed under the SIL
  Open Font License 1.1 (the project relicensed from Apache-2.0). The OFL text is
  shipped in `roboto/OFL.txt`.
- Inter is the static OTF (CFF) build from the upstream `rsms/inter` v4.1
  release; all other families are TrueType (`glyf`) outlines.

## Sources

- Inter            — https://github.com/rsms/inter (release v4.1, `extras/otf/`)
- Roboto           — https://fonts.google.com/specimen/Roboto (latin subset)
- Liberation Serif — https://github.com/liberationfonts/liberation-fonts (2.1.5)
- PT Serif         — https://fonts.google.com/specimen/PT+Serif
- Fira Code        — https://github.com/tonsky/FiraCode (release 6.2)
- IBM Plex Mono    — https://fonts.google.com/specimen/IBM+Plex+Mono

The SIL Open Font License 1.1 permits embedding the font (including subsetted) in
documents and bundling with software, provided the copyright + license notices
travel with the font. Those notices are the per-family license files in this
directory.
