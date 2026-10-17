(() => {
  const rect = (r) => [r.x, r.y, r.width, r.height];
  const out = [];
  for (const family of ["serif", "Times", "Times New Roman", "Helvetica", "Courier"])
    for (const size of [13, 16, 24]) {
      document.body.innerHTML = `<style>body{margin:0}#box{font-family:"${family}";font-size:${size}px;line-height:normal}</style><div id="box">light</div>`;
      const box = document.getElementById("box");
      if (family === "serif") box.style.fontFamily = "serif";
      const range = document.createRange();
      range.selectNodeContents(box);
      const canvas = document.createElement("canvas"),
        ctx = canvas.getContext("2d");
      ctx.font = `${size * 2}px ${family === "serif" ? "serif" : `"${family}"`}`;
      const m = ctx.measureText("light");
      out.push({
        family,
        size,
        range: rect(range.getBoundingClientRect()),
        box: rect(box.getBoundingClientRect()),
        fontBoundingBoxAscent: m.fontBoundingBoxAscent,
        fontBoundingBoxDescent: m.fontBoundingBoxDescent,
        width: m.width,
      });
    }
  return out;
})();
