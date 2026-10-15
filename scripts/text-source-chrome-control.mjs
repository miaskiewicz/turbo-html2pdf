// Independent authored Range controls with the exact bundled font bytes.
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const { values } = parseArgs({
  options: {
    runner: { type: "string" },
    out: { type: "string" },
    chrome: { type: "string" },
    adversarial: { type: "boolean", default: false },
  },
});
if (!values.runner || !values.out)
  throw Error("Pass --runner /absolute/chrome-cdp-trace.mjs and --out /absolute/oracle.json");
const font = readFileSync(join(root, "crates/turbo-html2pdf-core/assets/fonts/Go-Regular.ttf"));
const recovery = join(root, "target/parity-recovery/text-controls");
mkdirSync(recovery, { recursive: true });
const temporary = mkdtempSync(join(recovery, "capture-"));
const server = createServer((request, response) => {
  response.setHeader("Content-Type", request.url === "/font.ttf" ? "font/ttf" : "text/html");
  response.end(
    request.url === "/font.ttf" ? font : "<!doctype html><html><head></head><body></body></html>",
  );
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));
async function collect(adversarial) {
  const face = new FontFace("ParityGo", "url(/font.ttf)");
  await face.load();
  document.fonts.add(face);
  await document.fonts.ready;
  document.documentElement.style.cssText = "margin:0;padding:0";
  document.body.style.cssText = "margin:0;padding:0";
  const definitions = [
    { name: "ascii", text: "ab", width: 400 },
    { name: "nonbreaking-space", text: "a\u00a0b", width: 50 },
    { name: "em-space", text: "a\u2003b", width: 400 },
    { name: "font13", text: "ab", width: 400, fontSize: 13, lineHeight: 24 },
    { name: "font15", text: "ab", width: 400, fontSize: 15, lineHeight: 24 },
    { name: "font32", text: "ab", width: 400, fontSize: 32, lineHeight: 48 },
    { name: "font96", text: "ab", width: 400, fontSize: 96, lineHeight: 128 },
    { name: "font128", text: "ab", width: 400, fontSize: 128, lineHeight: 180 },
    { name: "leading-collapsed-accent", text: "  ab  é", width: 400 },
    { name: "ligature", text: "ffi", width: 400 },
    { name: "combining", text: "e\u0301", width: 400 },
    { name: "wrapped-identical", text: "ab ab", width: 90 },
    { name: "overlapping-lines", text: "ab ab", width: 90, lineHeight: 16 },
    { name: "astral-fallback", text: "A😀B", width: 400 },
    { name: "transformed", text: "ab", width: 400, transform: "translate(8px,12px) scale(1.5)" },
    { name: "rotated", text: "ab", width: 400, transform: "translate(100px,80px) rotate(10deg)" },
    { name: "skewed", text: "ab", width: 400, transform: "translate(50px,80px) skewX(15deg)" },
  ];
  if (adversarial)
    definitions.push(
      {
        name: "font13-flex",
        text: "ab",
        width: 400,
        fontSize: 13,
        lineHeight: 10,
        outerStyle: "display:flex;width:450px",
      },
      {
        name: "fractional-origin",
        text: "ab",
        width: 400,
        fontSize: 13,
        lineHeight: 24,
        extraStyle:
          "padding-left:3.145px;padding-top:2.17px;position:relative;left:.31px;top:1.24px;text-indent:2.61px",
      },
      {
        name: "fractional-negative-indent",
        text: "ab",
        width: 400,
        fontSize: 13,
        lineHeight: 24,
        extraStyle:
          "padding-left:8.145px;padding-top:8.17px;position:relative;left:-.31px;top:-1.24px;text-indent:-2.61px",
      },
      {
        name: "fractional-percent-indent",
        text: "ab",
        width: 41.145,
        fontSize: 13,
        lineHeight: 24,
        extraStyle: "text-indent:10%",
      },
      {
        name: "fractional-indent-wrapped",
        text: "ab ab ab",
        width: 24,
        fontSize: 13,
        lineHeight: 24,
        extraStyle:
          "padding-left:3.145px;padding-top:2.17px;position:relative;left:.31px;top:1.24px;text-indent:2.61px",
      },
      {
        name: "fractional-narrow-float",
        text: "ab",
        width: 100,
        fontSize: 13,
        lineHeight: 24,
        extraStyle: "text-indent:2.61px",
        floatStyle: "float:left;width:5px;height:37.145px",
      },
      {
        name: "fractional-unbreakable-zero-float",
        text: "abcdefabcdef",
        width: 40,
        fontSize: 13,
        lineHeight: 24,
        floatStyle: "float:left;width:10px;height:0px",
      },
      {
        name: "fractional-indent-after-float",
        text: "ab ab",
        width: 40.145,
        fontSize: 13,
        lineHeight: 24,
        extraStyle: "text-indent:2.61px",
        floatStyle: "float:left;width:100%;height:37.145px",
      },
      {
        name: "fractional-negative-transformed",
        text: "ab",
        width: 400,
        fontSize: 13,
        lineHeight: 24,
        extraStyle:
          "padding-left:1.145px;padding-top:1.17px;position:relative;left:-3.31px;top:-3.24px;text-indent:-2.61px",
        transform: "translate(20px,20px) rotate(10deg)",
      },
      {
        name: "fractional-scale-x",
        text: "ab",
        width: 400,
        fontSize: 13,
        lineHeight: 24,
        extraStyle:
          "padding-left:3.145px;padding-top:2.17px;position:relative;left:.31px;top:1.24px;text-indent:2.61px",
        transform: "translate(.1px,1.3px) scaleX(1.5)",
      },
      {
        name: "fractional-scale-y",
        text: "ab",
        width: 400,
        fontSize: 13,
        lineHeight: 24,
        extraStyle:
          "padding-left:3.145px;padding-top:2.17px;position:relative;left:.31px;top:1.24px;text-indent:2.61px",
        transform: "translate(.1px,1.3px) scaleY(1.5)",
      },
      {
        name: "fractional-translation",
        text: "ab",
        width: 400,
        fontSize: 13,
        lineHeight: 24,
        extraStyle:
          "padding-left:1.145px;padding-top:1.17px;position:relative;left:-3.31px;top:-3.24px;text-indent:-2.61px",
        transform: "translate(20px,20px)",
      },
      {
        name: "fractional-nested-transforms",
        text: "ab",
        width: 400,
        fontSize: 13,
        lineHeight: 24,
        outerStyle:
          "padding-left:3.145px;padding-top:2.17px;position:relative;left:.31px;top:1.24px;transform:rotate(10deg);transform-origin:0 0",
        extraStyle: "position:relative;left:.31px;top:1.24px;text-indent:2.61px",
        transform: "scale(1.5)",
      },
      {
        name: "fractional-transformed",
        text: "ab",
        width: 400,
        fontSize: 15,
        lineHeight: 24,
        extraStyle:
          "padding-left:3.145px;padding-top:2.17px;position:relative;left:.31px;top:1.24px;text-indent:2.61px",
        transform: "translate(.1px,1.3px) scale(1.5)",
      },
    );
  const rect = (r) => ({ x: r.x, y: r.y, width: r.width, height: r.height });
  const cases = [];
  for (const definition of definitions) {
    document.body.replaceChildren();
    const element = document.createElement("div");
    element.style.cssText = `font-family:ParityGo;font-size:${definition.fontSize ?? 64}px;line-height:${definition.lineHeight ?? 80}px;width:${definition.width}px;${definition.transform ? `transform:${definition.transform};transform-origin:0 0;` : ""}`;
    element.style.cssText += definition.extraStyle || "";
    const text = document.createTextNode(definition.text);
    if (definition.floatStyle) {
      const float = document.createElement("div");
      float.style.cssText = definition.floatStyle;
      element.append(float);
    }
    element.append(text);
    if (definition.outerStyle) {
      const outer = document.createElement("div");
      outer.style.cssText = definition.outerStyle;
      outer.append(element);
      document.body.append(outer);
    } else document.body.append(element);
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
    const canvas = document.createElement("canvas");
    const context = canvas.getContext("2d");
    context.font = `${definition.fontSize ?? 64}px ParityGo`;
    const measuredGlyphs = Array.from(definition.text, (character) => ({
      character,
      width: context.measureText(character).width,
    }));
    const sweep = [];
    if (adversarial && definition.name.startsWith("fractional-")) {
      for (const hit of hits) {
        for (const delta of [-0.03, -0.015, -0.007, 0, 0.007, 0.015, 0.03]) {
          const point = { x: hit.point.x + delta, y: hit.point.y };
          const caret = document.caretPositionFromPoint(point.x, point.y);
          sweep.push({
            range: hit.range,
            delta,
            point,
            sameText: caret?.offsetNode === text,
            offset: caret?.offset ?? null,
          });
        }
      }
    }
    cases.push({ ...definition, ranges, hits, measuredGlyphs, sweep });
  }
  return { font: "ParityGo", fontSize: 64, lineHeight: 80, cases };
}
let complete = false;
try {
  const probe = join(temporary, "probe.js");
  writeFileSync(probe, `(${collect})(${values.adversarial})`);
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
    collectorSha256: createHash("sha256")
      .update(readFileSync(fileURLToPath(import.meta.url)))
      .digest("hex"),
    probeSha256: createHash("sha256").update(readFileSync(probe)).digest("hex"),
    runnerSha256: createHash("sha256")
      .update(readFileSync(resolve(values.runner)))
      .digest("hex"),
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
