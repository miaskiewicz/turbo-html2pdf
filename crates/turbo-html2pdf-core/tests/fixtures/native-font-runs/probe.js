// Synchronous geometry controls shared by Chrome and actual native font registries.
({ family = "monospace" } = {}) => {
  const familyText = family === "monospace" ? family : JSON.stringify(family);
  const rect = (r) => ({ x: r.x, y: r.y, width: r.width, height: r.height });
  const fontHost = () => {
    const host = document.createElement("div");
    host.style.cssText = `width:1000px;font:16px/24px ${familyText}`;
    document.body.append(host);
    return host;
  };
  document.body.innerHTML = "";
  const host = fontHost();
  host.innerHTML = '<span id="a">abc</span><span id="b">z</span>';
  const a = document.getElementById("a"),
    b = document.getElementById("b");
  const plain = a.getBoundingClientRect(),
    before = b.getBoundingClientRect();
  a.style = "padding:3px 5px;border:2px solid;margin:0 7px";
  const decorated = a.getBoundingClientRect(),
    after = b.getBoundingClientRect();
  const range = document.createRange();
  range.selectNodeContents(a);
  const text = range.getBoundingClientRect();
  const checks = {
    decorationWidth: decorated.width === plain.width + 14,
    followingShift: after.x === before.x + 28,
    textWidth: text.width === plain.width,
    textShift: text.x === plain.x + 14,
    clientWidth: a.clientWidth === 0,
    clientHeight: a.clientHeight === 0,
    offsetWidth: a.offsetWidth === Math.round(decorated.width),
  };
  const decoration = {
    checks,
    plain: rect(plain),
    before: rect(before),
    decorated: rect(decorated),
    after: rect(after),
    text: rect(text),
    textRects: Array.from(range.getClientRects(), rect),
    client: [a.clientWidth, a.clientHeight],
    offset: [a.offsetWidth, a.offsetHeight],
  };
  const strings = ["a", "ab", "abc", "abcd", "abcde", "abcdefgh", "AV", "fi", "a b", "a b c"];
  const runs = strings.map((value) => {
    const parent = fontHost(),
      span = document.createElement("span");
    span.textContent = value;
    parent.append(span);
    const node = span.firstChild,
      offsets = new Map();
    for (let end = 0; end <= value.length; end++) offsets.set(`0:${end}`, [0, end]);
    for (let start = 0; start < value.length; start++)
      offsets.set(`${start}:${start + 1}`, [start, start + 1]);
    if (value.length > 2) offsets.set(`1:${value.length - 1}`, [1, value.length - 1]);
    const ranges = Array.from(offsets.values(), ([start, end]) => {
      const selected = document.createRange();
      selected.setStart(node, start);
      selected.setEnd(node, end);
      return {
        start,
        end,
        rect: rect(selected.getBoundingClientRect()),
        rects: Array.from(selected.getClientRects(), rect),
      };
    });
    return {
      text: value,
      element: rect(span.getBoundingClientRect()),
      ranges,
    };
  });
  const wholeHost = fontHost(),
    wholeSpan = document.createElement("span"),
    splitHost = fontHost();
  wholeSpan.textContent = "abc";
  wholeHost.append(wholeSpan);
  const measurePart = (span) => {
    const selection = document.createRange();
    selection.selectNodeContents(span);
    return {
      text: span.textContent,
      element: rect(span.getBoundingClientRect()),
      textRect: rect(selection.getBoundingClientRect()),
      textRects: Array.from(selection.getClientRects(), rect),
    };
  };
  const splitParts = ["a", "bc"].map((value) => {
    const span = document.createElement("span");
    span.textContent = value;
    splitHost.append(span);
    return span;
  });
  const splits = { whole: measurePart(wholeSpan), parts: splitParts.map(measurePart) };
  const rotated = fontHost();
  rotated.style.transform = "rotate(15deg)";
  rotated.style.transformOrigin = "0 0";
  const rotatedText = document.createElement("span");
  rotatedText.textContent = "abc";
  rotated.append(rotatedText);
  const rotatedRanges = [
    [0, 1],
    [0, 2],
    [0, 3],
    [1, 2],
    [1, 3],
  ].map(([start, end]) => {
    const selection = document.createRange();
    selection.setStart(rotatedText.firstChild, start);
    selection.setEnd(rotatedText.firstChild, end);
    return {
      start,
      end,
      rect: rect(selection.getBoundingClientRect()),
      rects: Array.from(selection.getClientRects(), rect),
    };
  });
  return {
    family,
    devicePixelRatio,
    decoration,
    runs,
    splits,
    rotated: {
      container: rect(rotated.getBoundingClientRect()),
      element: rect(rotatedText.getBoundingClientRect()),
      ranges: rotatedRanges,
    },
  };
};
