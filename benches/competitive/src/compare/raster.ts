// Rasterize page 1 of a PDF to PNG via poppler's `pdftoppm`. Skips gracefully if
// poppler is absent (PNG equivalence is then simply not scored).

import { execFile } from "node:child_process";
import { mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { detectBinary } from "../adapters/util.ts";

const exec = promisify(execFile);

/** True when `pdftoppm` is on PATH. */
export async function rasterizerAvailable(): Promise<boolean> {
  const a = await detectBinary("pdftoppm", ["-v"]);
  return a.available;
}

async function firstPng(dir: string): Promise<Buffer | null> {
  const files = (await readdir(dir)).filter((f) => f.endsWith(".png")).sort();
  const first = files[0];
  if (!first) return null;
  return readFile(join(dir, first));
}

/** Render page 1 of a PDF to a PNG buffer at the given DPI; null on failure. */
export async function pdfToPng(pdf: Uint8Array, dpi: number): Promise<Buffer | null> {
  const dir = await mkdtemp(join(tmpdir(), "tpdf-raster-"));
  try {
    const input = join(dir, "in.pdf");
    await writeFile(input, pdf);
    const args = ["-png", "-r", String(dpi), "-f", "1", "-l", "1", input, join(dir, "page")];
    await exec("pdftoppm", args, { timeout: 30000 });
    return await firstPng(dir);
  } catch {
    return null;
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}
