// Run authored layout controls in the independent installed-Chrome CDP oracle.
import { execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
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
    fixture: { type: "string" },
  },
});
if (!values.runner || !values.out)
  throw new Error("Pass --runner /absolute/chrome-cdp-trace.mjs and --out /absolute/oracle.json");
// The layout-snapshot fixtures moved with the layout tests to the turbo-dom-layout
// crate (turbo-dom repo); find its checkout through cargo when --fixture is omitted.
const layoutCrate = () =>
  dirname(
    JSON.parse(
      execFileSync("cargo", ["metadata", "--format-version", "1"], { cwd: root, encoding: "utf8" }),
    ).packages.find((pkg) => pkg.name === "turbo-dom-layout").manifest_path,
  );
const fixturePath = resolve(
  values.fixture ?? join(layoutCrate(), "tests/fixtures/layout-snapshot/cases.json"),
);
const fixtureBytes = readFileSync(fixturePath);
const fixture = JSON.parse(fixtureBytes);
const fontBytes =
  fixture.font === "Go"
    ? readFileSync(join(root, "crates/turbo-html2pdf-core/assets/fonts/Go-Regular.ttf"))
    : null;
const temporary = mkdtempSync(join(tmpdir(), "turbo-layout-oracle-"));
const server = createServer((request, response) => {
  response.setHeader("Content-Type", request.url === "/font.ttf" ? "font/ttf" : "text/html");
  response.end(
    request.url === "/font.ttf"
      ? fontBytes
      : "<!doctype html><html><head></head><body></body></html>",
  );
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));

async function collect(fixture) {
  if (fixture.font === "Go") {
    const face = new FontFace("ParityGo", "url(/font.ttf)");
    await face.load();
    document.fonts.add(face);
    await document.fonts.ready;
  }
  document.documentElement.style.cssText = "margin:0;padding:0;height:100%;";
  document.body.style.cssText = "margin:0;padding:0;height:100%;";
  const canvas = document.createElement("canvas");
  canvas.width = fixture.intrinsicImage.width;
  canvas.height = fixture.intrinsicImage.height;
  const imageSource = canvas.toDataURL("image/png");
  const rect = (r) => ({ x: r.x, y: r.y, width: r.width, height: r.height });
  const cases = [];
  for (const test of fixture.cases) {
    // Each graph starts with the same document, including after real-root cases.
    document.documentElement.style.cssText = "margin:0;padding:0;height:100%;";
    document.body.style.cssText = "margin:0;padding:0;height:100%;";
    document.body.replaceChildren();
    const elements = [];
    const ready = [];
    const node = (definition) => {
      if (typeof definition.text === "string") return document.createTextNode(definition.text);
      const element =
        test.documentRoot && definition.tag === "html"
          ? document.documentElement
          : document.createElement(definition.tag);
      if (element === document.documentElement) element.replaceChildren();
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
    for (const definition of test.nodes) {
      const element = node(definition);
      if (element !== document.documentElement) document.body.append(element);
    }
    await Promise.all(ready);
    cases.push({
      name: test.name,
      boxes: elements.map(({ sourceId, element }) => ({
        sourceId,
        isDocumentRoot: element === document.documentElement,
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
    fixture: fixturePath.slice(fixturePath.lastIndexOf("/") + 1),
    fixtureSha256: createHash("sha256").update(fixtureBytes).digest("hex"),
    probeSha256: createHash("sha256").update(readFileSync(probe)).digest("hex"),
    collectorSha256: createHash("sha256")
      .update(readFileSync(fileURLToPath(import.meta.url)))
      .digest("hex"),
    ...(fontBytes ? { fontSha256: createHash("sha256").update(fontBytes).digest("hex") } : {}),
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
