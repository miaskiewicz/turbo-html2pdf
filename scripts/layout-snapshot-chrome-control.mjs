// Run authored layout controls in the independent installed-Chrome CDP oracle.
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { parseArgs } from "node:util";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const { values } = parseArgs({
  options: {
    runner: { type: "string" },
    out: { type: "string" },
    chrome: { type: "string" },
  },
});
if (!values.runner || !values.out)
  throw new Error("Pass --runner /absolute/chrome-cdp-trace.mjs and --out /absolute/oracle.json");
const fixture = JSON.parse(
  readFileSync(
    join(root, "crates/turbo-html2pdf-core/tests/fixtures/layout-snapshot/cases.json"),
    "utf8",
  ),
);
const temporary = mkdtempSync(join(tmpdir(), "turbo-layout-oracle-"));
const server = createServer((_request, response) =>
  response.end("<!doctype html><html><head></head><body></body></html>"),
);
await new Promise((done) => server.listen(0, "127.0.0.1", done));

async function collect(fixture) {
  document.documentElement.style.cssText = "margin:0;padding:0;height:100%;";
  document.body.style.cssText = "margin:0;padding:0;height:100%;";
  const canvas = document.createElement("canvas");
  canvas.width = fixture.intrinsicImage.width;
  canvas.height = fixture.intrinsicImage.height;
  const imageSource = canvas.toDataURL("image/png");
  const rect = (r) => ({ x: r.x, y: r.y, width: r.width, height: r.height });
  const cases = [];
  for (const test of fixture.cases) {
    document.body.replaceChildren();
    const elements = [];
    const ready = [];
    const node = (definition) => {
      const element = document.createElement(definition.tag);
      element.style.cssText = definition.style;
      elements.push({ sourceId: definition.sourceId, element });
      for (const child of definition.children) element.append(node(child));
      if (definition.tag === "img") {
        ready.push(
          new Promise((done, reject) => {
            element.onload = done;
            element.onerror = reject;
          }),
        );
        element.src = imageSource;
      }
      return element;
    };
    for (const definition of test.nodes) document.body.append(node(definition));
    await Promise.all(ready);
    cases.push({
      name: test.name,
      boxes: elements.map(({ sourceId, element }) => ({
        sourceId,
        rect: rect(element.getBoundingClientRect()),
        clientRects: Array.from(element.getClientRects(), rect),
        client: {
          left: element.clientLeft,
          top: element.clientTop,
          width: element.clientWidth,
          height: element.clientHeight,
        },
        offset: { width: element.offsetWidth, height: element.offsetHeight },
      })),
    });
  }
  return { viewport: { width: innerWidth, height: innerHeight }, cases };
}

let completed = false;
try {
  const probe = join(temporary, "probe.js");
  writeFileSync(probe, `(${collect.toString()})(${JSON.stringify(fixture)})`);
  const config = join(temporary, "config.json");
  writeFileSync(
    config,
    JSON.stringify({
      url: `http://127.0.0.1:${server.address().port}/`,
      viewport: fixture.viewport,
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
  if (status !== 0) throw new Error(`Chrome capture exited ${status}`);
  const trace = JSON.parse(readFileSync(join(output, "trace.json"), "utf8"));
  const result = trace.probes[0];
  if (!result?.result || result.error || trace.errors.length)
    throw new Error("Chrome control failed; inspect capture diagnostics");
  const oracle = {
    browser: trace.browserVersion,
    v8: trace.browserV8Version,
    recordedAt: new Date().toISOString(),
    fixture: "cases.json",
    effectiveExtensionFlags: trace.effectiveExtensionFlags,
    ...result.result,
  };
  writeFileSync(resolve(values.out), JSON.stringify(oracle, null, 2) + "\n");
  console.log(
    JSON.stringify({
      out: resolve(values.out),
      browser: oracle.browser,
      cases: oracle.cases.length,
    }),
  );
  completed = true;
} finally {
  await new Promise((done) => server.close(done));
  if (completed) rmSync(temporary, { recursive: true, force: true });
  else console.error(`Failed capture retained at ${temporary}`);
}
