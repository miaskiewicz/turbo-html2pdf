// PNG equivalence: rasterize each engine's PDF and pixelmatch-diff page 1 against a
// reference engine's render (spec AC-10.10). Records a similarity score 0..1 so the
// harness can prove outputs are comparable and flag "winning by doing less".

import { PNG } from "pngjs";
import pixelmatch from "pixelmatch";
import { pdfToPng, rasterizerAvailable } from "./raster.ts";

const DPI = 100;
/** Below this, layouts are deemed grossly divergent ("not equivalent"). */
export const EQUIVALENCE_FLOOR = 0.9;

export interface DiffResult {
  /** Fraction of matching pixels, 0..1; null when not computable. */
  readonly similarity: number | null;
  readonly reason: string | null;
}

function resize(png: PNG, w: number, h: number): PNG {
  if (png.width === w && png.height === h) return png;
  const out = new PNG({ width: w, height: h });
  png.bitblt(out, 0, 0, Math.min(png.width, w), Math.min(png.height, h), 0, 0);
  return out;
}

function compare(refPng: PNG, candPng: PNG): number {
  const w = Math.max(refPng.width, candPng.width);
  const h = Math.max(refPng.height, candPng.height);
  const a = resize(refPng, w, h);
  const b = resize(candPng, w, h);
  const diff = pixelmatch(a.data, b.data, null, w, h, { threshold: 0.1 });
  const total = w * h;
  return total > 0 ? 1 - diff / total : 0;
}

/** Rasterize both PDFs and return their page-1 similarity. */
export async function similarityOf(
  candidate: Uint8Array,
  reference: Uint8Array,
): Promise<DiffResult> {
  if (!(await rasterizerAvailable())) {
    return { similarity: null, reason: "pdftoppm (poppler) not installed" };
  }
  const refImg = await pdfToPng(reference, DPI);
  const candImg = await pdfToPng(candidate, DPI);
  if (!refImg || !candImg) return { similarity: null, reason: "rasterization failed" };
  const similarity = compare(PNG.sync.read(refImg), PNG.sync.read(candImg));
  return { similarity, reason: null };
}
