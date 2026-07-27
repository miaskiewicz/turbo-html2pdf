// Candidate side: the turbo-html2pdf engine, via the napi `layoutBoxes` debug
// export. It lays the HTML out with the bundled default fonts and returns the
// placed border-box geometry of every `data-cid`-tagged box as JSON.

import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { Box } from "./types.ts";

const HERE = dirname(fileURLToPath(import.meta.url));
// benches/conformance/src -> repo root -> the napi package entry.
const NAPI_ENTRY = resolve(HERE, "../../../crates/turbo-pdf-napi/index.js");
const require = createRequire(import.meta.url);

interface Napi {
  layoutBoxes(html: string, css: string, width: number, height: number): string;
}

let napi: Napi | null = null;

function load(): Napi {
  if (!napi) napi = require(NAPI_ENTRY) as Napi;
  return napi;
}

/** Whether the native addon loads and exposes `layoutBoxes`. */
export function engineAvailable(): { ok: boolean; reason: string | null } {
  try {
    const n = load();
    if (typeof n.layoutBoxes !== "function") {
      return { ok: false, reason: "napi addon is stale (no layoutBoxes) — run `pnpm build:addon`" };
    }
    return { ok: true, reason: null };
  } catch (e) {
    return { ok: false, reason: `napi addon not built: ${(e as Error).message}` };
  }
}

/** Lay `html` out and return the tagged boxes keyed by `cid`. */
export function engineBoxes(html: string, width: number, height: number): Map<string, Box> {
  const json = load().layoutBoxes(html, "", width, height);
  const list = JSON.parse(json) as Box[];
  const map = new Map<string, Box>();
  for (const b of list) map.set(b.cid, b);
  return map;
}
