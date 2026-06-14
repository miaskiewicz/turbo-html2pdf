// Font resolution. Reuse the engine crate's bundled fonts so every adapter shapes
// the SAME glyphs — no copying large binaries, just reference by relative path.

import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));

/** Repo-root-relative path to the shared font assets. */
export const FONT_DIR = resolve(HERE, "../../../../crates/turbo-pdf-core/assets/fonts");

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
