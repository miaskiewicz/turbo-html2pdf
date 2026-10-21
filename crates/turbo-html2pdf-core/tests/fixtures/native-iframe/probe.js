(async () => {
  const rect = (value) => [value.x, value.y, value.width, value.height];
  const cases = [
    ["default", ""],
    ["attributes", 'width="127" height="63"'],
    ["attribute-width", 'width="80"'],
    ["invalid-attributes", 'width="invalid" height="-1"'],
    ["css-auto-overrides-attributes", 'width="127" height="63" style="width:auto;height:auto"'],
    ["css-zero", 'style="width:0;height:0;border:0"'],
    [
      "css-zero-standards",
      'style="width:0;height:0;border:0"',
      "",
      '<!doctype html><body style="margin:0">',
    ],
    ["zero-width", 'style="width:0;border:0"'],
    ["zero-height", 'style="height:0;border:0"'],
    ["padding", 'style="width:100px;height:50px;padding:3px;border:2px solid"'],
    [
      "border-box",
      'style="box-sizing:border-box;width:80px;height:40px;padding:3px;border:2px solid"',
    ],
    ["minimum", 'style="width:10px;height:5px;min-width:20px;min-height:15px;border:0"'],
    ["ratio", 'style="width:80px;height:auto;aspect-ratio:4;border:0"'],
    ["percentage-attributes", 'width="40%" height="50%"'],
    ["display-none", 'style="display:none"'],
    ["zero-parent-percentage", 'style="width:10px;height:50%;border:0"', "height:0"],
    [
      "relative-parent-percentage",
      'style="position:relative;width:10px;height:50%;border:0"',
      "height:200px",
    ],
    [
      "relative-zero-parent-percentage",
      'style="position:relative;width:10px;height:50%;border:0"',
      "height:0",
    ],
    ["fallback-text", 'style="width:40px;height:20px;border:0"'],
    [
      "padded-positioned-zero",
      'style="position:absolute;width:10px;height:50%;border:0"',
      "height:0",
      null,
      "position:relative;height:50%;padding:10px",
    ],
  ];
  const result = [];
  for (const [name, attrs, hostStyle = "", srcdoc = null, parentStyle = null] of cases) {
    try {
      const frameHtml = `<iframe id="frame" ${attrs}></iframe>`;
      const contents =
        parentStyle === null
          ? frameHtml
          : `<div id="parent" style="${parentStyle}">${frameHtml}</div>`;
      document.body.innerHTML = `<style>body{margin:0;font:16px/24px monospace}#host{width:400px;height:200px}</style><div id="host" style="${hostStyle}">${contents}</div>`;
      const frame = document.getElementById("frame");
      if (srcdoc !== null) {
        const loaded = new Promise((resolve) => (frame.onload = resolve));
        frame.srcdoc = srcdoc;
        await loaded;
      }
      const win = frame.contentWindow;
      const row = {
        name,
        attrs,
        hostStyle,
        srcdoc,
        parentStyle,
        rect: rect(frame.getBoundingClientRect()),
        rects: Array.from(frame.getClientRects(), rect),
        client: [frame.clientLeft, frame.clientTop, frame.clientWidth, frame.clientHeight],
        offset: [frame.offsetLeft, frame.offsetTop, frame.offsetWidth, frame.offsetHeight],
        viewport: [win.innerWidth, win.innerHeight],
      };
      if (parentStyle !== null)
        row.parentRect = rect(document.getElementById("parent").getBoundingClientRect());
      if (name === "fallback-text") {
        const fallbackText = document.createTextNode("fallback \ud800 \udfff");
        const nested = document.createElement("span");
        nested.id = "fallback-nested";
        nested.textContent = "nested \udc00";
        frame.append(fallbackText, nested);
        const control = document.createElement("div");
        control.id = "fallback-control";
        control.textContent = "control \ud800";
        document.body.append(control);
        const selection = (node, contents) => {
          const range = document.createRange();
          if (contents) range.selectNodeContents(node);
          else range.selectNode(node);
          return {
            rect: rect(range.getBoundingClientRect()),
            rects: Array.from(range.getClientRects(), rect),
          };
        };
        const units = (text) =>
          Array.from({ length: text.length }, (_, index) => text.charCodeAt(index));
        row.fallback = {
          textUnits: units(fallbackText.data),
          contentUnits: units(frame.textContent),
          nestedUnits: units(nested.firstChild.data),
          controlUnits: units(control.firstChild.data),
          childCount: frame.childNodes.length,
          ownership: [
            fallbackText.parentNode === frame,
            nested.parentNode === frame,
            nested.firstChild.parentNode === nested,
            fallbackText.ownerDocument === document,
            nested.firstChild.ownerDocument === document,
          ],
          frameBox: rect(frame.getBoundingClientRect()),
          frameSelection: selection(frame, false),
          frameContents: selection(frame, true),
          textSelection: selection(fallbackText, true),
          nestedSelection: selection(nested, false),
          nestedContents: selection(nested, true),
          controlSelection: selection(control, true),
        };
      }
      if (["css-zero", "css-zero-standards", "zero-width", "zero-height"].includes(name)) {
        row.media = Object.fromEntries(
          [
            "(width:0px)",
            "(height:0px)",
            "(min-width:0px)",
            "(max-width:0px)",
            "(orientation:portrait)",
            "(orientation:landscape)",
            "(aspect-ratio)",
            "(aspect-ratio:0/1)",
            "(aspect-ratio:0/0)",
            "(aspect-ratio:1/1)",
            "(aspect-ratio:16/9)",
            "(aspect-ratio:1/0)",
            "(min-aspect-ratio:0/1)",
            "(max-aspect-ratio:0/1)",
            "(min-aspect-ratio:1/1)",
            "(max-aspect-ratio:1/1)",
            "(min-aspect-ratio:1/0)",
            "(max-aspect-ratio:1/0)",
            "(0/1 <= aspect-ratio <= 16/9)",
            "(0/1 < aspect-ratio < 16/9)",
            "(aspect-ratio > 1/1)",
            "(aspect-ratio < 1/1)",
          ].map((query) => [query, win.matchMedia(query).matches]),
        );
      }
      if (["css-zero", "css-zero-standards"].includes(name)) {
        const doc = frame.contentDocument;
        doc.body.style = "margin:0";
        row.childHtml =
          '<div id="fixed" style="width:27px;height:11px"></div><div id="units" style="width:50vw;height:50vh"></div><iframe id="nested" style="width:37px;height:19px;border:0"></iframe>';
        const childIds = ["fixed", "units", "nested"];
        if (name === "css-zero-standards") {
          row.childHtml += '<div id="percentage" style="width:50%;height:50%"></div>';
          childIds.push("percentage");
        }
        doc.body.innerHTML = row.childHtml;
        row.document = {
          compatMode: doc.compatMode,
          font: win.getComputedStyle(doc.body).font,
          lineHeight: win.getComputedStyle(doc.body).lineHeight,
        };
        const span = doc.createElement("span");
        span.textContent = "light";
        doc.body.append(span);
        const range = doc.createRange();
        range.selectNodeContents(span);
        row.document.textRange = rect(range.getBoundingClientRect());
        span.remove();
        row.children = Object.fromEntries(
          childIds.map((id) => [id, rect(doc.getElementById(id).getBoundingClientRect())]),
        );
        row.body = rect(doc.body.getBoundingClientRect());
        row.nestedViewport = [
          doc.getElementById("nested").contentWindow.innerWidth,
          doc.getElementById("nested").contentWindow.innerHeight,
        ];
      }
      result.push(row);
    } catch (error) {
      result.push({
        name,
        attrs,
        hostStyle,
        srcdoc,
        parentStyle,
        layoutError: {
          name: String(error?.name ?? "Error"),
          message: String(error?.message ?? error),
        },
      });
      continue;
    }
  }
  return result;
})();
