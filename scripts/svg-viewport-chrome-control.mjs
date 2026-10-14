// Independent authored outer-SVG viewport controls; shape boxes stay recorded separately.
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
const { values } = parseArgs({
  options: {
    runner: { type: "string" },
    out: { type: "string" },
    chrome: { type: "string" },
    scale: { type: "string" },
  },
});
if (!values.runner || !values.out)
  throw Error("Pass --runner /absolute/chrome-cdp-trace.mjs and --out /absolute/oracle.json");
const temporary = mkdtempSync(join(tmpdir(), "turbo-svg-oracle-"));
const server = createServer((_request, response) => {
  response.setHeader("Content-Type", "text/html");
  response.end("<!doctype html><html><head></head><body></body></html>");
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));
async function collect() {
  document.documentElement.style.cssText = "margin:0;padding:0";
  document.body.style.cssText = "margin:0;padding:0";
  const definitions = [
    { name: "attributes", attrs: { width: "32", height: "24" } },
    {
      name: "flex-fixed",
      attrs: { width: "32", height: "24" },
      containerStyle:
        "display:flex;align-items:flex-start;width:400px;height:200px;font-size:0;line-height:0",
    },
    {
      name: "flex-ratio-height",
      attrs: { height: "50", viewBox: "0 0 2 1" },
      containerStyle:
        "display:flex;align-items:flex-start;width:400px;height:200px;font-size:0;line-height:0",
    },
    {
      name: "auto-intrinsic-width",
      attrs: { width: "80", viewBox: "0 0 2 1" },
      style: "width:auto;height:auto",
    },
    {
      name: "auto-intrinsic-height",
      attrs: { height: "50", viewBox: "0 0 2 1" },
      style: "width:auto;height:auto",
    },
    { name: "mixed-calc-attribute", attrs: { width: "calc(50% + 10px)", height: "24" } },
    {
      name: "missing-css-variable",
      attrs: { width: "32", height: "24" },
      style: "width:var(--missing);height:48px",
    },
    { name: "exponent-viewbox", attrs: { viewBox: "0 0 2e0 1e0" } },
    { name: "invalid-exponent-viewbox", attrs: { viewBox: "0 0 2e 1" } },
    { name: "decimal-viewbox", attrs: { viewBox: "0 0 .2 .1" } },
    {
      name: "invalid-css-width",
      attrs: { width: "32", height: "24" },
      style: "width:nonsense;height:48px",
    },
    {
      name: "invalid-css-height",
      attrs: { width: "32", height: "24" },
      style: "width:64px;height:nonsense",
    },
    {
      name: "negative-css-width",
      attrs: { width: "32", height: "24" },
      style: "width:-5px;height:48px",
    },
    {
      name: "negative-css-calc",
      attrs: { width: "32", height: "24" },
      style: "width:calc(-5px);height:48px",
    },
    { name: "negative-attribute", attrs: { width: "-5", height: "24" } },
    { name: "malformed-attribute", attrs: { width: "nope", height: "24" } },
    { name: "calc-attribute", attrs: { width: "calc(20px + 20px)", height: "24" } },
    { name: "comma-viewbox", attrs: { viewBox: "0,0,2,1" } },
    { name: "repeated-comma-viewbox", attrs: { viewBox: "0,,0,2,1" } },
    { name: "trailing-comma-viewbox", attrs: { viewBox: "0 0 2 1," } },
    { name: "leading-comma-viewbox", attrs: { viewBox: ",0 0 2 1" } },
    { name: "adjacent-sign-viewbox", attrs: { viewBox: "0+0+2+1" } },
    { name: "extra-number-viewbox", attrs: { viewBox: "0 0 2 1 5" } },
    {
      name: "attribute-ratio-priority",
      attrs: { width: "32", height: "24", viewBox: "0 0 40 20" },
      style: "width:64px;height:auto",
    },
    { name: "layout-scale-disambiguation", attrs: { width: "32.504", height: "24.1" } },
    { name: "css-width-only", attrs: { width: "32", height: "24" }, style: "width:64px" },
    { name: "css-auto-width", attrs: { width: "32", height: "24" }, style: "width:auto" },
    {
      name: "css-auto-both",
      attrs: { width: "32", height: "24" },
      style: "width:auto;height:auto",
    },
    { name: "fractional-attrs", attrs: { width: "32.5", height: "24.25" } },
    { name: "fractional-layout-unit", attrs: { width: "32.51", height: "24.29" } },
    { name: "css-fractional-layout-unit", attrs: {}, style: "width:32.51px;height:24.29px" },
    { name: "ratio-layout-unit", attrs: { width: "80", viewBox: "0 0 40.1 20" } },
    { name: "case-sensitive-viewbox", attrs: { viewbox: "0 0 40 20" } },
    { name: "case-sensitive-dimensions", attrs: { WIDTH: "32", HEIGHT: "24" } },
    { name: "default", attrs: {} },
    { name: "viewbox-only", attrs: { viewBox: "0 0 40 20" } },
    { name: "width-only", attrs: { width: "40" } },
    { name: "height-only", attrs: { height: "30" } },
    { name: "ratio-width", attrs: { width: "80", viewBox: "0 0 40 20" } },
    { name: "ratio-height", attrs: { height: "50", viewBox: "0 0 40 20" } },
    { name: "percentage-attributes", attrs: { width: "50%", height: "50%" } },
    {
      name: "css-dimensions",
      attrs: { width: "32", height: "24" },
      style: "width:64px;height:48px",
    },
    {
      name: "border-padding",
      attrs: { width: "40", height: "20" },
      style: "border:2px solid;padding:3px",
    },
    {
      name: "css-border-box",
      attrs: {},
      style: "width:80px;height:60px;box-sizing:border-box;border:2px solid;padding:3px",
    },
    { name: "zero", attrs: { width: "0", height: "0" } },
    {
      name: "viewbox-none",
      attrs: { width: "80", viewBox: "0 0 40 20", preserveAspectRatio: "none" },
    },
    {
      name: "css-transform",
      attrs: { width: "40", height: "20" },
      style: "transform:translate(7px,9px) scale(1.5);transform-origin:0 0",
    },
    { name: "hidden", attrs: { width: "40", height: "20" }, style: "display:none" },
    { name: "block", attrs: { width: "40", height: "20" }, style: "display:block;margin:4px 6px" },
  ];
  const rect = (r) => ({ x: r.x, y: r.y, width: r.width, height: r.height });
  const boxes = (element) => ({
    rect: rect(element.getBoundingClientRect()),
    rects: Array.from(element.getClientRects(), rect),
    client: {
      width: element.clientWidth,
      height: element.clientHeight,
      left: element.clientLeft,
      top: element.clientTop,
    },
  });
  const htmlPrecisionElement = document.createElement("div");
  htmlPrecisionElement.style.cssText = "width:32.51px;height:24.29px";
  document.body.append(htmlPrecisionElement);
  const htmlPrecision = rect(htmlPrecisionElement.getBoundingClientRect());
  htmlPrecisionElement.style.width = "32.504px";
  const precisionProbe = rect(htmlPrecisionElement.getBoundingClientRect());
  const layoutScaleCalibration = {
    observations: [
      { input: 32.51, actual: htmlPrecision.width },
      { input: 32.504, actual: precisionProbe.width },
    ],
    matchingScales: [1, 2, 4, 8].filter((scale) =>
      [
        [32.51, htmlPrecision.width],
        [32.504, precisionProbe.width],
      ].every(([input, actual]) => Math.trunc(input * 64 * scale) / (64 * scale) === actual),
    ),
  };
  const cases = [];
  for (const definition of definitions) {
    const container = document.createElement("div");
    container.style.cssText =
      definition.containerStyle ?? "width:400px;height:200px;font-size:0;line-height:0";
    const viewport = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    for (const [key, value] of Object.entries(definition.attrs)) viewport.setAttribute(key, value);
    viewport.style.cssText = definition.style ?? "";
    const circle = document.createElementNS("http://www.w3.org/2000/svg", "circle");
    circle.setAttribute("cx", "10");
    circle.setAttribute("cy", "10");
    circle.setAttribute("r", "5");
    viewport.append(circle);
    container.append(viewport);
    document.body.replaceChildren(container);
    cases.push({
      ...definition,
      container: boxes(container),
      viewport: boxes(viewport),
      shape: boxes(circle),
      computedViewport: {
        width: getComputedStyle(viewport).width,
        height: getComputedStyle(viewport).height,
        zoom: getComputedStyle(viewport).zoom,
      },
    });
  }
  return {
    viewport: { width: innerWidth, height: innerHeight, deviceScaleFactor: devicePixelRatio },
    htmlPrecision,
    layoutScaleCalibration,
    cases,
  };
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
      deviceScaleFactor: Number(values.scale ?? "2"),
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
