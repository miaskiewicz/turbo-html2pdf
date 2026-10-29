// Font resolution. Reuse the layout engine crate's fonts (turbo-dom-layout's
// assets/fonts, located through cargo) so every adapter shapes the SAME glyphs — no
// copying large binaries, just reference them where they live.

import { existsSync } from "node:fs";
import { join } from "node:path";
import { layoutFontsDir } from "../../../../scripts/layout-crate-dir.mjs";

/** turbo-dom-layout's font assets. */
export const FONT_DIR = layoutFontsDir();

/** The regular + bold faces every workload uses. Evolventa = our base sans. */
export const FONTS = {
  regular: join(FONT_DIR, "Evolventa-zLXL.ttf"),
  bold: join(FONT_DIR, "EvolventaBold-55Xv.ttf"),
  italic: join(FONT_DIR, "EvolventaOblique-yPLV.ttf"),
} as const;

/** Family name embedded in CSS / draw APIs so output is labelled consistently. */
export const FONT_FAMILY = "Evolventa";

/** True when the shared fonts exist (they should, in-repo). */
export function fontsAvailable(): boolean {
  return existsSync(FONTS.regular) && existsSync(FONTS.bold);
}
