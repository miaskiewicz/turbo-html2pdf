({ family = "IBM Plex Mono" } = {}) => {
  const rect = (r) => ({ x: r.x, y: r.y, width: r.width, height: r.height });
  const definitions = [];
  for (const atom of ["image", "inline-block"])
    for (const prefix of ["", " ", "  "]) definitions.push({ atom, prefix });
  definitions.push(
    { atom: "image", prefix: " ", mode: "direct" },
    { atom: "inline-block", prefix: " ", mode: "direct" },
    { atom: "inline-block", prefix: " ", mode: "nested-wrapper" },
    { atom: "inline-block", prefix: " ", mode: "line-wrapped" },
    { atom: "inline-block", prefix: " ", mode: "decorated" },
    { atom: "none", prefix: " ", mode: "paragraph-start" },
    { atom: "none", prefix: " ", mode: "after-br" },
    { atom: "image", prefix: " ", mode: "direct-wrapped" },
    { atom: "inline-block", prefix: " ", mode: "direct-wrapped" },
  );
  for (const text of ["abc def", "abc  def"])
    for (const width of [1000, 45])
      definitions.push({ atom: "none", mode: "ordinary", text, width });
  const ordinaryCase = ({ text, width, mode, atom }) => {
    document.body.innerHTML = "";
    document.body.style.cssText = "margin:0";
    const host = document.createElement("div");
    host.id = "a";
    host.style.cssText = `width:${width}px;font-family:${JSON.stringify(family)};font-size:16px;line-height:24px`;
    host.textContent = text;
    document.body.append(host);
    const node = host.firstChild;
    const range = (start, end) => {
      const r = document.createRange();
      r.setStart(node, start);
      r.setEnd(node, end);
      return {
        start,
        end,
        rect: rect(r.getBoundingClientRect()),
        rects: Array.from(r.getClientRects(), rect),
      };
    };
    const second = text.length - 3;
    const intervals = [];
    for (let offset = 3; offset <= second; offset++) {
      if (offset < second) intervals.push([offset, offset + 1]);
      intervals.push([offset, offset]);
    }
    return {
      atom,
      mode,
      text,
      width,
      host: rect(host.getBoundingClientRect()),
      firstWordRange: range(0, 3),
      secondWordRange: range(second, text.length),
      fullRange: range(0, text.length),
      spacesRange: range(3, second),
      prefixRanges: Array.from({ length: second - 2 }, (_, index) => range(0, index + 3)),
      individualRanges: intervals.map(([a, b]) => range(a, b)),
    };
  };
  const cases = definitions.map((definition) => {
    if (definition.mode === "ordinary") return ordinaryCase(definition);
    const { atom, prefix, mode } = definition;
    const direct = mode === "direct" || mode === "direct-wrapped";
    document.body.innerHTML = "";
    document.body.style.cssText = "margin:0";
    const host = document.createElement("div");
    host.style.cssText = `width:${mode === "line-wrapped" || mode === "direct-wrapped" ? 45 : 1000}px;font-family:${JSON.stringify(family)};font-size:16px;line-height:24px`;
    let a = null;
    if (atom !== "none") {
      a = document.createElement(atom === "image" ? "img" : "span");
      a.id = "atom";
      a.style.cssText = `display:${atom === "image" ? "inline" : "inline-block"};width:10px;height:10px`;
      host.append(a);
    }
    if (mode === "after-br") {
      const before = document.createElement("span");
      before.textContent = "z";
      host.append(before, document.createElement("br"));
    }
    const caption = direct
      ? document.createTextNode(prefix + "abc")
      : document.createElement("span");
    if (!direct) {
      caption.id = "a";
      caption.textContent = prefix + "abc";
    }
    if (mode === "decorated")
      caption.style.cssText = "padding:3px 5px;border:2px solid;margin:0 7px";
    let wrapper = null;
    if (mode === "nested-wrapper") {
      wrapper = document.createElement("span");
      wrapper.append(caption);
      host.append(wrapper);
    } else host.append(caption);
    const following = document.createElement("span");
    following.textContent = "z";
    if (direct) following.id = "a";
    host.append(following);
    document.body.append(host);
    const text = direct ? caption : caption.firstChild;
    const range = (start, end) => {
      const r = document.createRange();
      r.setStart(text, start);
      r.setEnd(text, end);
      return {
        start,
        end,
        rect: rect(r.getBoundingClientRect()),
        rects: Array.from(r.getClientRects(), rect),
      };
    };
    const result = {
      atom,
      prefix,
      host: rect(host.getBoundingClientRect()),
      atomBox: a ? rect(a.getBoundingClientRect()) : null,
      captionBox: direct ? null : rect(caption.getBoundingClientRect()),
      followingBox: rect(following.getBoundingClientRect()),
      visibleRange: range(prefix.length, prefix.length + 3),
      fullRange: range(0, prefix.length + 3),
      prefixRange: range(0, prefix.length),
    };
    if (mode) result.mode = mode;
    if (!direct && mode) result.captionRects = Array.from(caption.getClientRects(), rect);
    if (wrapper) result.wrapperBox = rect(wrapper.getBoundingClientRect());
    if (prefix.length === 2)
      result.individualRanges = [
        [0, 1],
        [1, 2],
        [1, 1],
        [2, 2],
      ].map(([a, b]) => range(a, b));
    return result;
  });
  return { family, devicePixelRatio, cases };
};
