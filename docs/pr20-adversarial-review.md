# PR20 adversarial review receipts

The text checkpoint and native SVG checkpoint were reviewed independently by the
parent agent against actual source and independently captured Chrome controls.
These are bounded geometry reviews, not whole-browser parity certification.

## Round 1: native text and caret behavior

- Review found that a later fragment could outrank a nearer horizontal edge on
  the same line. A genuine native fallback-font source reproduces the defect;
  reversing the repair returns offset 3 instead of the expected offset 0.
- Review found that source queries materialized glyph cells for the entire
  document. The producer now filters native runs by source before allocation.
- An independently recorded overlapping-line control exposed 29px displacement
  from rejecting HTML negative leading. Actual pure-text HTML line heights now
  support negative leading; PDF and atomic-inline policies retain their scope.
- The final recorded control set covers 144 Range offset pairs and 89 caret hits
  over twelve controlled-font graphs. Rotated/skewed caret transforms, full bidi,
  internal ligature carets, document hit testing and cross-node assembly remain
  explicit unsupported or embedding-owned behavior.

## Round 2: native SVG viewport and failure behavior

- Review found invalid retained CSS could suppress valid presentation attributes.
  Real controls now distinguish invalid declaration fallback from computed
  variable invalidation and negative calculated values.
- Review found repeated-comma viewBox values could be accepted by split/filter
  parsing. The bounded SVG number scanner rejects repeated, leading and trailing
  commas while preserving actual sign, decimal and exponent grammar.
- Forty-nine independently captured Chrome graphs validate complete parent and
  viewport rectangles and client sizes. Shape, nested SVG and foreignObject
  identities are retained with explicit unsupported geometry diagnostics.
- Separate JS-DPR and fractional calibration controls establish physical layout
  scale independently. Conversion occurs in native sizing before fragments;
  returned rectangles and affine transforms are not rewritten.
- Fault tests remove a real captured glyph and require failure for incomplete
  nonwhitespace selections. Real native font-cache duplicate results and capture
  capacity boundaries verify accounting and fail-closed publication. Cross-platform
  font policy is tested using actual Go font metrics.

The source checkpoint passes 770 workspace tests, strict clippy, fmt, full Rust
complexity <= 5, notices checks and 100% line coverage (7723/7723). Release-candidate
version changes rerun the prescribed gates; CI results are recorded on the PR.
