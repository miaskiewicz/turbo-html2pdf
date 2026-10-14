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

The previously recorded fractional used-value and indentation failures are now
resolved by native placement. Typed lengths quantize before layout, while float
clearance preserves indentation on the first formatted line below y=0. Negative
and percentage indentation, fractional padding/relative origins, one-axis scales,
pure translation and nested rotation controls pass. The 30 controlled graphs
cover 391 Range lists, 273 caret points and 1141 fractional sweeps. Policy reuse
checks HTML scale 2 → generic/PDF → HTML scale 1/2 against fresh intrinsic caches.

Native paint hit scopes follow the pinned Blink paint-property source: rounded
origins discard residuals only on non-propagating transform axes. No global query
pre-floor remains; float WebIDL input survives inverse mapping to local physical
LayoutUnits. The separate hit transform does not change layout/Range matrices.
Containment/isolation, masks, opacity/compositing and animation remain outside this
certified hit scope.

The stronger original-unit control found a real scalar projection loss and invalid
font fallback. The additive unit input preserves native identity and raw cache keys;
safe CoreText cascade wrappers select actual Lucida Grande/Menlo/STIX fallback faces
using their real glyph metrics. MacRoman-only names are retained privately without
changing scalar/PDF family resolution. The independent mixed source keeps all 26
original-versus-replacement Range differences and twelve font/size controls. Unknown,
duplicate, mismatched or over-budget original inputs fail before layout.

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
