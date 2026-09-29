// Oracle side: Chromium via Playwright. Sets the viewport, loads the fixture,
// waits for the pinned fonts, then reads each `data-cid` element's
// `getBoundingClientRect()` (page/viewport-relative, border box).
//
// Availability mirrors the competitive bench's Playwright adapter: if the
// package or a launchable Chromium is absent, the caller SKIPS cleanly rather
// than failing (exit 0). No browser is bundled with this harness.

import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { Box } from "./types.ts";

interface Page {
  setViewportSize(size: { width: number; height: number }): Promise<void>;
  setContent(html: string, opts: Record<string, unknown>): Promise<void>;
  // Playwright evaluates a string expression in the page; we pass source strings
  // so the harness itself never references browser globals (DOM is off the
  // typecheck lib on purpose).
  evaluate<T>(expression: string): Promise<T>;
  close(): Promise<void>;
}
interface Browser {
  newPage(): Promise<Page>;
  close(): Promise<void>;
}
interface Chromium {
  launch(opts: Record<string, unknown>): Promise<Browser>;
}
interface Playwright {
  chromium: Chromium;
}

const HERE = dirname(fileURLToPath(import.meta.url));
// Playwright is an optional dep of this package, but pnpm isolates workspace
// installs, so a bare `import("playwright")` may not resolve from here even when
// it is installed in a sibling bench. Resolve it from a few candidate roots
// (this package, the competitive bench that already depends on it, the repo
// root) before giving up — then the harness runs without a dedicated install
// yet still skips cleanly when Playwright is genuinely absent.
async function importPlaywright<T>(): Promise<T | null> {
  const bases = [
    resolve(HERE, ".."), // benches/conformance
    resolve(HERE, "../../competitive"), // sibling bench that depends on playwright
    resolve(HERE, "../../.."), // repo root
  ];
  for (const base of bases) {
    try {
      const req = createRequire(`${base}/`);
      const entry = req.resolve("playwright");
      const mod = (await import(entry)) as Record<string, unknown>;
      // Playwright is CommonJS: named exports (`chromium`) land on the module's
      // `default` under the ESM interop, so unwrap it when the top level lacks them.
      const ns = "chromium" in mod ? mod : ((mod.default as Record<string, unknown>) ?? mod);
      return ns as T;
    } catch {
      // try the next base
    }
  }
  return null;
}

// Source string evaluated in the page: read every tagged box (document order)
// as border-box rects. Kept as a string so the harness never references DOM
// globals at the type level.
const READ_BOXES = `(() => {
  const out = [];
  for (const el of document.querySelectorAll('[data-cid]')) {
    const r = el.getBoundingClientRect();
    out.push({ cid: el.getAttribute('data-cid'), x: r.x, y: r.y, width: r.width, height: r.height });
  }
  return out;
})()`;

export class Oracle {
  private browser: Browser | null = null;

  private constructor(browser: Browser) {
    this.browser = browser;
  }

  /** Launch Chromium, or return why it's unavailable (for a clean skip). */
  static async launch(): Promise<{ oracle: Oracle | null; reason: string | null }> {
    const pw = await importPlaywright<Playwright>();
    if (!pw) return { oracle: null, reason: "playwright package not installed" };
    try {
      const browser = await pw.chromium.launch({ headless: true });
      return { oracle: new Oracle(browser), reason: null };
    } catch (e) {
      const hint = "run: npx playwright install chromium";
      return { oracle: null, reason: `chromium launch failed (${hint}): ${(e as Error).message}` };
    }
  }

  /** Load `html` at `width`x`height` and return the tagged boxes keyed by cid. */
  async measure(html: string, width: number, height: number): Promise<Map<string, Box>> {
    if (!this.browser) throw new Error("oracle closed");
    const page = await this.browser.newPage();
    try {
      await page.setViewportSize({ width, height });
      await page.setContent(html, { waitUntil: "load" });
      // Ensure the pinned @font-face has loaded before measuring text metrics.
      await page.evaluate("document.fonts.ready");
      const list = await page.evaluate<Box[]>(READ_BOXES);
      const map = new Map<string, Box>();
      for (const b of list) map.set(b.cid, b);
      return map;
    } finally {
      await page.close();
    }
  }

  async close(): Promise<void> {
    if (this.browser) await this.browser.close();
    this.browser = null;
  }
}
