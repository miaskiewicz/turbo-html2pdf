// The Jinja-escaping problem, solved.
//
// React's `renderToStaticMarkup` HTML-escapes text children (`<` -> `&lt;`,
// `&` -> `&amp;`, …) and attribute values (`"` -> `&quot;`, …). Our control-flow
// components must emit *literal* Jinja statements like `{% if total > 0 %}` whose
// expression strings contain `<`, `>`, `&`, `"` that React would mangle.
//
// Approach: control-flow components do NOT render Jinja text directly. They render
// a *sentinel token* — a base64url payload wrapped in two Unicode Private-Use-Area
// characters (`` … ``). Those characters are not HTML-special, so React
// passes them through `renderToStaticMarkup` byte-for-byte, no escaping. A single
// post-process pass (`expandSentinels`) decodes each payload back into the literal
// Jinja string. Plain HTML and `t:` elements are emitted by React normally and kept
// verbatim — they legitimately *want* HTML escaping.
//
// Base64url keeps the encoded payload free of `<`, `>`, `&`, `"`, `'`, `=` and the
// sentinel chars themselves, so a payload can never be confused with markup or with
// another sentinel.

const OPEN = "";
const CLOSE = "";

/** Encode arbitrary text to a base64url string (no padding, HTML-safe alphabet). */
function toBase64Url(text: string): string {
  const bytes = new TextEncoder().encode(text);
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** Decode a base64url string produced by {@link toBase64Url}. */
function fromBase64Url(encoded: string): string {
  const padded = encoded.replace(/-/g, "+").replace(/_/g, "/");
  const binary = atob(padded);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return new TextDecoder().decode(bytes);
}

/** Wrap literal Jinja text in a sentinel token so React emits it un-escaped. */
export function sentinel(literal: string): string {
  return `${OPEN}${toBase64Url(literal)}${CLOSE}`;
}

/** Replace every sentinel token in `markup` with its decoded literal Jinja text. */
export function expandSentinels(markup: string): string {
  const pattern = new RegExp(`${OPEN}([A-Za-z0-9_-]*)${CLOSE}`, "g");
  return markup.replace(pattern, (_match, payload: string) => fromBase64Url(payload));
}
