// Independent authored Range controls with the exact bundled font bytes.
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const { values } = parseArgs({
  options: { runner: { type: "string" }, out: { type: "string" }, chrome: { type: "string" } },
});
if (!values.runner || !values.out)
  throw Error("Pass --runner /absolute/chrome-cdp-trace.mjs and --out /absolute/oracle.json");
const font = readFileSync(join(root, "crates/turbo-html2pdf-core/assets/fonts/Go-Regular.ttf"));
const temporary = mkdtempSync(join(tmpdir(), "turbo-text-oracle-"));
const server = createServer((request, response) => {
  response.setHeader("Content-Type", request.url === "/font.ttf" ? "font/ttf" : "text/html");
  response.end(
    request.url === "/font.ttf" ? font : "<!doctype html><html><head></head><body></body></html>",
  );
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));
async function collect() {
  const face = new FontFace("ParityGo", "url(/font.ttf)");
  await face.load();
  document.fonts.add(face);
  await document.fonts.ready;
  document.documentElement.style.cssText = "margin:0;padding:0";
  document.body.style.cssText = "margin:0;padding:0";
  const definitions = [
    { name: "ascii", text: "ab", width: 400 },
    { name: "font32", text: "ab", width: 400, fontSize: 32, lineHeight: 48 },
    { name: "font96", text: "ab", width: 400, fontSize: 96, lineHeight: 128 },
    { name: "font128", text: "ab", width: 400, fontSize: 128, lineHeight: 180 },
    { name: "leading-collapsed-accent", text: "  ab  é", width: 400 },
    { name: "ligature", text: "ffi", width: 400 },
    { name: "combining", text: "e\u0301", width: 400 },
    { name: "wrapped-identical", text: "ab ab", width: 90 },
    { name: "astral-fallback", text: "A😀B", width: 400 },
    { name: "transformed", text: "ab", width: 400, transform: "translate(8px,12px) scale(1.5)" },
  ];
  const rect = (r) => ({ x: r.x, y: r.y, width: r.width, height: r.height });
  const cases = [];
  for (const definition of definitions) {
    document.body.replaceChildren();
    const element = document.createElement("div");
    element.style.cssText = `font-family:ParityGo;font-size:${definition.fontSize ?? 64}px;line-height:${definition.lineHeight ?? 80}px;width:${definition.width}px;${definition.transform ? `transform:${definition.transform};transform-origin:0 0;` : ""}`;
    const text = document.createTextNode(definition.text);
    element.append(text);
    document.body.append(element);
    const ranges = {};
    for (let start = 0; start <= text.length; start++)
      for (let end = start; end <= text.length; end++) {
        const range = document.createRange();
        range.setStart(text, start);
        range.setEnd(text, end);
        ranges[`${start}:${end}`] = {
          rect: rect(range.getBoundingClientRect()),
          rects: Array.from(range.getClientRects(), rect),
        };
      }
    const hits = [];
    for (const [key, value] of Object.entries(ranges)) {
      if (value.rect.width <= 0) continue;
      const point = {
        x: value.rect.x + value.rect.width / 2,
        y: value.rect.y + value.rect.height / 2,
      };
      const caret = document.caretPositionFromPoint(point.x, point.y);
      hits.push({
        range: key,
        point,
        sameText: caret?.offsetNode === text,
        offset: caret?.offset ?? null,
      });
    }
    cases.push({ ...definition, ranges, hits });
  }
  return { font: "ParityGo", fontSize: 64, lineHeight: 80, cases };
}
let complete = false;
try {
  const probe = join(temporary, "probe.js");
  writeFileSync(probe, `(${collect})()`);
  const config = join(temporary, "config.json");
  writeFileSync(
    config,
    JSON.stringify({
      url: `http://127.0.0.1:${server.address().port}/`,
      waitMs: 25,
      disableExtensions: true,
      steps: [],
      probes: [probe],
      ...(values.chrome ? { chromePath: resolve(values.chrome) } : {}),
    }),
  );
  const output = join(temporary, "capture");
  const child = spawn(process.execPath, [resolve(values.runner), config, output], {
    stdio: "inherit",
  });
  const status = await new Promise((done) => child.once("exit", done));
  if (status !== 0) throw Error(`Chrome capture exited ${status}`);
  const trace = JSON.parse(readFileSync(join(output, "trace.json"), "utf8"));
  if (trace.errors.length || trace.probes[0]?.error || !trace.probes[0]?.result)
    throw Error("Chrome control failed; inspect preserved diagnostics");
  const oracle = {
    browser: trace.browserVersion,
    v8: trace.browserV8Version,
    recordedAt: new Date().toISOString(),
    fontSha256: createHash("sha256").update(font).digest("hex"),
    effectiveExtensionFlags: trace.effectiveExtensionFlags,
    result: trace.probes[0].result,
  };
  writeFileSync(resolve(values.out), `${JSON.stringify(oracle, null, 2)}\n`);
  console.log(JSON.stringify({ browser: oracle.browser, out: resolve(values.out) }));
  complete = true;
} finally {
  await new Promise((done) => server.close(done));
  if (complete) rmSync(temporary, { recursive: true, force: true });
  else console.error(`Failed capture preserved at ${temporary}`);
}
