// The same local text quad crosses different real CSS transform boundaries.
({ family = "Fira Code" } = {}) => {
  const rect = (r) => ({ x: r.x, y: r.y, width: r.width, height: r.height });
  const cases = [
    { name: "margin-flow", body: "margin:8px", spacer: 312, own: "rotate(15deg)" },
    { name: "moved-margin-flow", body: "margin:37.5px 0 0 13.25px", own: "rotate(15deg)" },
    { name: "padding-flow", body: "margin:0;padding:320px 0 0 8px", own: "rotate(15deg)" },
    { name: "absolute", body: "margin:0", absolute: true, own: "rotate(15deg)" },
    {
      name: "translated-parent",
      body: "margin:8px",
      spacer: 312,
      parent: "translate(5.25px,7.5px)",
      own: "rotate(15deg)",
    },
    {
      name: "rotated-parent",
      body: "margin:8px",
      spacer: 312,
      parent: "rotate(10deg)",
      own: "rotate(15deg)",
    },
    {
      name: "pivoted-parent",
      body: "margin:8px",
      spacer: 312,
      parent: "rotate(10deg)",
      pivot: "20px 12px",
      own: "rotate(15deg)",
    },
    { name: "negative-rotation", body: "margin:8px", spacer: 312, own: "rotate(-15deg)" },
    {
      name: "fixed-child",
      body: "margin:8px",
      spacer: 312,
      parent: "rotate(10deg)",
      fixed: true,
      own: "rotate(15deg)",
    },
    {
      name: "identity-parent",
      body: "margin:37.5px 0 0 13.25px",
      parent: "matrix(1,0,0,1,0,0)",
      own: "rotate(15deg)",
    },
    {
      name: "integer-translated-parent",
      body: "margin:37.5px 0 0 13.25px",
      parent: "translate(5px,7px)",
      own: "rotate(15deg)",
    },
  ].map((definition) => {
    document.body.innerHTML = "";
    document.body.style.cssText = definition.body;
    if (definition.spacer) {
      const spacer = document.createElement("div");
      spacer.style.height = `${definition.spacer}px`;
      document.body.append(spacer);
    }
    const parent = document.createElement("div");
    parent.style.cssText = `width:1000px;font-family:${JSON.stringify(family)};font-size:16px;line-height:24px;transform-origin:${definition.pivot ?? "0 0"}`;
    if (definition.absolute) parent.style.cssText += ";position:absolute;left:8px;top:320px";
    let container = parent;
    if (definition.parent) {
      parent.style.transform = definition.parent;
      container = document.createElement("div");
      container.style.cssText = "width:1000px;transform-origin:0 0";
      if (definition.fixed) container.style.cssText += ";position:fixed;left:5.25px;top:7.5px";
      parent.append(container);
    }
    container.style.transform = definition.own;
    const span = document.createElement("span");
    span.id = "a";
    span.textContent = "abc";
    container.append(span);
    document.body.append(parent);
    const selections = [
      [0, 0],
      [0, 1],
      [0, 2],
      [0, 3],
      [1, 2],
      [1, 3],
    ].map(([start, end]) => {
      const range = document.createRange();
      range.setStart(span.firstChild, start);
      range.setEnd(span.firstChild, end);
      return { start, end, range };
    });
    const measure = () => {
      const ranges = selections.map(({ start, end, range }) => ({
        start,
        end,
        rect: rect(range.getBoundingClientRect()),
        rects: Array.from(range.getClientRects(), rect),
      }));
      return {
        parent: rect(parent.getBoundingClientRect()),
        container: rect(container.getBoundingClientRect()),
        element: rect(span.getBoundingClientRect()),
        elementRects: Array.from(span.getClientRects(), rect),
        ranges,
      };
    };
    const transformed = measure();
    const parentTransform = parent.style.transform,
      ownTransform = container.style.transform;
    parent.style.transform = "none";
    container.style.transform = "none";
    const untransformed = measure();
    parent.style.transform = parentTransform;
    container.style.transform = ownTransform;
    return { ...definition, transformed, untransformed };
  });
  return { family, devicePixelRatio, cases };
};
