(() => {
  const rect = (r) => [r.x, r.y, r.width, r.height];
  const measure = (node) => ({
    rect: rect(node.getBoundingClientRect()),
    rects: Array.from(node.getClientRects(), rect),
    client: [node.clientLeft, node.clientTop, node.clientWidth, node.clientHeight],
    offset: [node.offsetLeft, node.offsetTop, node.offsetWidth, node.offsetHeight],
  });
  const cases = [
    [
      "baseline-top",
      '<span id="outer" style="vertical-align:baseline;padding:2px;border:1px solid">a<img id="inner" style="display:inline;width:12px;height:8px;vertical-align:top">b</span>',
    ],
    [
      "baseline-bottom",
      '<span id="outer" style="vertical-align:baseline;padding:2px;border:1px solid">a<img id="inner" style="display:inline;width:12px;height:8px;vertical-align:bottom">b</span>',
    ],
    [
      "super-top",
      '<span id="outer" style="vertical-align:super;padding:2px;border:1px solid">a<img id="inner" style="display:inline;width:12px;height:8px;vertical-align:top">b</span>',
    ],
    [
      "super-bottom",
      '<span id="outer" style="vertical-align:super;padding:2px;border:1px solid">a<img id="inner" style="display:inline;width:12px;height:8px;vertical-align:bottom">b</span>',
    ],
    [
      "sub-top",
      '<span id="outer" style="vertical-align:sub;padding:2px;border:1px solid">a<img id="inner" style="display:inline;width:12px;height:8px;vertical-align:top">b</span>',
    ],
    [
      "sub-bottom",
      '<span id="outer" style="vertical-align:sub;padding:2px;border:1px solid">a<img id="inner" style="display:inline;width:12px;height:8px;vertical-align:bottom">b</span>',
    ],
    ["empty-top-margin", '<span id="outer" style="margin-top:15px;margin-bottom:7px"></span>'],

    ["plain", '<span id="outer">abc</span>'],
    [
      "super-image",
      '<span id="outer" style="vertical-align:super;padding:2px;border:1px solid">a<img id="inner" style="display:inline;width:12px;height:8px">b</span>',
    ],
    [
      "sub-inline-block",
      '<span id="outer" style="vertical-align:sub;padding:2px;border:1px solid">a<span id="inner" style="display:inline-block;width:12px;height:8px"></span>b</span>',
    ],
    ["empty-transparent", '<span id="outer"></span>'],
    ["only-space", '<span id="outer"> </span>'],
    ["empty-offset", '<span id="outer"></span>'],
    [
      "empty-vertical-decoration",
      '<span id="outer" style="padding-top:3px;border-top:2px solid"></span>',
    ],
    ["empty-background", '<span id="outer" style="background:rgba(255,0,0,.5)"></span>'],
    ["empty-margin", '<span id="outer" style="margin:0 7px"></span>'],
    [
      "margin-transparent-child",
      '<span id="outer" style="margin:0 7px"><span id="inner">abc</span></span>',
    ],
    ["trailing-break", '<span id="outer" style="padding:2px;border:1px solid">abc<br></span>'],
    [
      "consecutive-break",
      '<span id="outer" style="padding:2px;border:1px solid">abc<br><br>def</span>',
    ],
    [
      "transparent-background",
      '<span id="outer" style="background:rgba(255,0,0,0)">a<span id="inner">b</span>c</span>',
    ],
    [
      "alpha-background",
      '<span id="outer" style="background:rgba(255,0,0,.5)">a<span id="inner">b</span>c</span>',
    ],
    ["fractional-negative-leading", '<span id="outer">abc</span>'],
    [
      "relative",
      '<span id="outer" style="position:relative;left:10px;top:6px;padding:2px;border:1px solid">a<span id="inner">b</span>c</span><span id="after">z</span>',
    ],
    ["zero-width-fragment", '<span id="outer"><br>abc</span>'],
    [
      "parent24-super",
      '<span id="outer" style="font-size:16px;vertical-align:super;padding:2px;border:1px solid">abc</span>',
    ],
    [
      "parent24-sub",
      '<span id="outer" style="font-size:16px;vertical-align:sub;padding:2px;border:1px solid">abc</span>',
    ],
    [
      "nested-super",
      '<span id="outer" style="vertical-align:super;padding:2px;border:1px solid">a<span id="inner" style="font-size:24px;vertical-align:super">bc</span>d</span>',
    ],
    [
      "super",
      '<span id="outer" style="vertical-align:super;padding:2px;border:1px solid">abc</span>',
    ],
    ["sub", '<span id="outer" style="vertical-align:sub;padding:2px;border:1px solid">abc</span>'],
    [
      "ignored-transform",
      '<span id="outer" style="transform:translate(50px,30px);padding:2px;border:1px solid">a<span id="inner">b</span>c</span>',
    ],
    ["transparent-child", '<span id="outer"><span id="inner">abc</span></span>'],
    [
      "transparent-decorated-child",
      '<span id="outer"><span id="inner" style="padding:2px;border:1px solid">a<span>b</span>c</span></span>',
    ],
    [
      "replaced-image",
      '<img id="outer" style="display:inline;width:20px;height:10px;padding:3px;border:2px solid">',
    ],
    ["split-space", '<span id="outer">a <span id="inner"> b</span> c</span>'],
    ["empty-space", '<span id="outer">a<span id="inner"> </span> b</span>'],
    [
      "nowrap-units",
      '<span id="outer" style="white-space:nowrap">a😀<span id="inner">b\uD800</span> c</span>',
    ],
    [
      "decorated",
      '<span id="outer" style="padding:3px 5px;border:2px solid;margin:0 7px">abc</span><span id="after">z</span>',
    ],
    [
      "empty",
      '<span id="outer" style="padding:3px 5px;border:2px solid;margin:0 7px"></span><span id="after">z</span>',
    ],
    [
      "mixed-font",
      '<span id="outer" style="padding:2px 3px;border:1px solid">a<span id="inner" style="font-size:24px">bc</span>d</span>',
    ],
    [
      "wrapped",
      '<span id="outer" style="padding:3px 5px;border:2px solid;margin:0 7px">one two three four five</span>',
    ],
    [
      "nested-wrapped",
      '<span id="outer" style="padding:3px 5px;border:2px solid">one <span id="inner" style="padding:1px 3px;border:1px solid">two three four</span> five</span>',
    ],
    [
      "unbreakable-nested",
      '<span id="outer" style="padding:0 3px;border:1px solid">ab<span id="inner" style="padding:0 2px;border:1px solid">cd</span>ef</span>',
    ],
    ["forced-break", '<span id="outer" style="padding:3px 5px;border:2px solid">one<br>two</span>'],
  ];
  const out = [];
  for (const [name, html] of cases) {
    document.body.innerHTML =
      '<style>body{margin:0;font:16px/24px monospace}#box{width:75px}span{vertical-align:baseline}</style><div id="box">' +
      html +
      "</div>";
    if (name.startsWith("parent24")) {
      document.getElementById("box").style.fontSize = "24px";
      document.getElementById("box").style.lineHeight = "36px";
    }
    if (name === "fractional-negative-leading")
      document.getElementById("box").style.lineHeight = "18.9921875px";
    if (name === "empty-offset")
      document.getElementById("box").style = "margin-left:11px;padding-top:7px";
    if (name === "zero-width-fragment") document.getElementById("box").style.textIndent = "100px";
    const outer = document.getElementById("outer"),
      inner = document.getElementById("inner"),
      after = document.getElementById("after");
    const range = document.createRange();
    range.selectNodeContents(outer);
    out.push({
      name,
      box: rect(document.getElementById("box").getBoundingClientRect()),
      outer: measure(outer),
      inner: inner ? measure(inner) : null,
      after: after ? measure(after) : null,
      range: {
        rect: rect(range.getBoundingClientRect()),
        rects: Array.from(range.getClientRects(), rect),
      },
    });
    if (name === "decorated") {
      outer.style.paddingLeft = "11px";
      out.push({
        name: "mutated-decoration",
        box: rect(document.getElementById("box").getBoundingClientRect()),
        outer: measure(outer),
        after: measure(after),
      });
    }
  }
  return out;
})();
