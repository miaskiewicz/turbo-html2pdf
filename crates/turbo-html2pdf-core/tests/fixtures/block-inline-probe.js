(async () => {
  const rect = (r) => [r.x, r.y, r.width, r.height];
  const read = (document) => {
    const rows = {};
    for (const [name, selector] of Object.entries({
      html: "html",
      body: "body",
      parent: "#rso",
      link: "#rso a",
      heading: "#rso h3",
      nested: "#nested",
      sibling: "#sibling",
      dismiss: "#dismiss",
      form: "form",
      field: "textarea",
    })) {
      const node = document.querySelector(selector);
      if (!node) continue;
      const style = document.defaultView.getComputedStyle(node);
      rows[name] = {
        bounds: rect(node.getBoundingClientRect()),
        rects: Array.from(node.getClientRects(), rect),
        offset: [node.offsetLeft, node.offsetTop, node.offsetWidth, node.offsetHeight],
        client: [node.clientLeft, node.clientTop, node.clientWidth, node.clientHeight],
        style: Object.fromEntries(
          [
            "display",
            "fontFamily",
            "fontSize",
            "fontWeight",
            "lineHeight",
            "marginTop",
            "marginRight",
            "marginBottom",
            "marginLeft",
            "paddingTop",
            "paddingRight",
            "paddingBottom",
            "paddingLeft",
            "borderTopWidth",
            "borderRightWidth",
            "borderBottomWidth",
            "borderLeftWidth",
            "verticalAlign",
          ].map((k) => [k, style[k]]),
        ),
      };
      if (name === "link" || name === "heading") {
        const range = document.createRange();
        range.selectNodeContents(node);
        rows[name].contentsRange = {
          bounds: rect(range.getBoundingClientRect()),
          rects: Array.from(range.getClientRects(), rect),
        };
      }
    }
    const button = document.querySelector("#dismiss"),
      b = button.getBoundingClientRect();
    const hit = document.elementFromPoint(b.x + b.width / 2, b.y + b.height / 2);
    return {
      rows,
      buttonCenterHit: {
        tag: hit?.tagName,
        id: hit?.id,
        inside: button === hit || button.contains(hit),
      },
    };
  };
  const result = {
    actualEnvironment: {
      userAgent: navigator.userAgent,
      viewport: [innerWidth, innerHeight],
      deviceScaleFactor: devicePixelRatio,
    },
    originalFixture: read(document),
    variants: {},
  };
  const variants = {
    exactOneBlock: "<a><h3>Old result</h3></a>",
    mixedContinuation: "<a>Before<h3>Old result</h3>After</a>",
    nestedInline: '<a><span id="nested">Before<h3>Old result</h3>After</span></a>',
    nestedOuterInline: '<span id="nested">Outer<a>Before<h3>Old result</h3>After</a>Tail</span>',
    multipleBlocks:
      '<a>Before<h3>First</h3>Between<div>Second</div>After</a><section id="sibling">Sibling</section>',
    decorated: '<a style="border:2px solid;padding:3px">Before<h3>Old result</h3>After</a>',
  };
  for (const [name, markup] of Object.entries(variants)) {
    const frame = document.createElement("iframe");
    frame.style = "position:absolute;left:0;top:0;width:1280px;height:800px;border:0";
    const loaded = new Promise((resolve) => (frame.onload = resolve));
    frame.srcdoc =
      '<!doctype html><body><div id="rso">' +
      markup +
      '</div><button id="dismiss">Reject</button><input name="q" type="hidden" value="hidden"><form><textarea name="q"></textarea><button>Search</button></form>';
    document.body.append(frame);
    await loaded;
    result.variants[name] = read(frame.contentDocument);
    frame.remove();
  }
  return JSON.stringify(result);
})();
