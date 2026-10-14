# Native geometry v0.3.7 review receipts

These reviews cover the native geometry producer and its isolated turbo-surf
consumer; they do not certify whole-browser parity.

## Round 1: independently measured physical font and range behavior

The real crates.io 0.3.6 consumer passes 18 of 19 initial Chrome 155 controls. A
retained caret after Times 64→32 changes reports 36px height rather than Chrome's
35.5px. Go 13/15 controls separately expose physical ascent/descent/half-leading
and fractional Range-edge differences. The repair occurs in native font/line
placement and source selection before affine transforms. Raw glyph advances and
existing Fragment/PDF font policies retain their own representation.

New mixed-node Chrome controls found omitted zero-width Range endpoint boxes,
missing text-line scroll overflow, and a host panic on empty rectangle lists.
The embedding repairs include those regression cases; they are not changes to
the core's public single-source Range ordering contract.

## Round 2: independent parent-agent review and resulting controls

The parent reviewed physical/generic policy isolation and source rounding before
transforms, then requested concrete flex, nonzero fractional origin and overflow
checks. HTML flex scratch measurement now inherits the final placement's font
policy; a controlled real-font negative-leading case verifies the actual 10px
item height. Text-enabled snapshots reject nonfinite metadata before publishing
fragment, font, glyph or transform geometry. Layout scale remains bounded to
finite values from 0.25 through 16.

The fractional padding/relative/text-indent controls expose separate general
HTML used-value/indentation gaps. They remain recorded in the optional
`--adversarial` fixture and are outside this patch's numerical certification.
They must be resolved by actual producer placement, not rectangle adjustments.

The clarified div/img rotation controls further found float origin/composition
drift and incorrect BODY hit ancestry in the embedding. Native CSS transform
coefficients are captured during actual box layout; double composition is kept
until final float points, while the existing Fragment/PDF matrix type remains.
The core supports finite invertible affine caret queries and converts physical
hit positions to LayoutUnits. A real decoded-image regression ensures its inner
paint fragment inherits its border transform once, despite their shared layout
node ID. The embedding includes exact div/img geometry, hit and caret controls
for both zero and center transform origins.

Final local gates and the exact tested candidate SHA are recorded on the PR.
No release tags are authorized by this receipt.
