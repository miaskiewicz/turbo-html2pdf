import { createElement as h } from "react";
import { describe, expect, test } from "vitest";
import {
  Case,
  compileTemplate,
  Counter,
  Default,
  Each,
  Else,
  ElseIf,
  Expr,
  Footnote,
  If,
  Include,
  Page,
  PageMaster,
  Pages,
  Raw,
  Region,
  Running,
  RunningFooter,
  RunningHeader,
  Switch,
  UseMaster,
  Variant,
} from "../src/index.js";
import { CANONICAL_INVOICE_TEMPLATE } from "./canonical.js";
import { InvoiceDoc } from "./fixture.js";

describe("AC-8.7 byte-equality with hand-written template", () => {
  test("React output equals the canonical t:+Jinja template", () => {
    expect(compileTemplate(h(InvoiceDoc, null))).toBe(CANONICAL_INVOICE_TEMPLATE);
    // TODO(phase11): once the napi/wasm binding lands, also assert the React
    // template and the hand-written template compile to a byte-identical Program
    // (and identical PDF output) via Program::to_bytes — the deeper AC-8.7 check.
  });
});

describe("control-flow -> Jinja statements", () => {
  test("If/ElseIf/Else chain", () => {
    const out = compileTemplate(
      h(
        If,
        { cond: "a > 0" },
        h("p", null, "pos"),
        h(ElseIf, { cond: "a < 0" }, h("p", null, "neg")),
        h(Else, null, h("p", null, "zero")),
      ),
    );
    expect(out).toBe(
      "{% if a > 0 %}<p>pos</p>{% elif a < 0 %}<p>neg</p>{% else %}<p>zero</p>{% endif %}",
    );
  });

  test("Each without index", () => {
    const out = compileTemplate(h(Each, { of: "rows", as: "row" }, h(Expr, { value: "row.id" })));
    expect(out).toBe("{% for row in rows %}{{ row.id }}{% endfor %}");
  });

  test("Each with named index emits a loop.index0 binding", () => {
    const out = compileTemplate(h(Each, { of: "rows", as: "row", index: "i" }, "x"));
    expect(out).toBe("{% for row in rows %}{% set i = loop.index0 %}x{% endfor %}");
  });

  test("Each with empty-collection Else", () => {
    const out = compileTemplate(h(Each, { of: "rows", as: "row" }, "row", h(Else, null, "empty")));
    expect(out).toBe("{% for row in rows %}row{% else %}empty{% endfor %}");
  });

  test("Switch/Case/Default with membership", () => {
    const out = compileTemplate(
      h(
        Switch,
        { on: "tier" },
        h(Case, { value: '"a"' }, "A"),
        h(Case, { value: '"b", "c"' }, "BC"),
        h(Default, null, "D"),
      ),
    );
    expect(out).toBe(
      '{% switch tier %}{% case "a" %}A{% case "b", "c" %}BC{% default %}D{% endswitch %}',
    );
  });

  test("Include with and without context", () => {
    expect(compileTemplate(h(Include, { src: '"a"' }))).toBe('{% include "a" %}');
    expect(compileTemplate(h(Include, { src: '"a"', with: "ctx" }))).toBe(
      '{% include "a" with ctx %}',
    );
  });

  test("Expr interpolation", () => {
    expect(compileTemplate(h(Expr, { value: "x.y | currency(c)" }))).toBe(
      "{{ x.y | currency(c) }}",
    );
  });
});

describe("Jinja-escaping: expression strings survive verbatim", () => {
  test("comparison operators are not HTML-escaped", () => {
    expect(compileTemplate(h(If, { cond: "total > 0 && q < 5 && x == y" }, "ok"))).toBe(
      "{% if total > 0 && q < 5 && x == y %}ok{% endif %}",
    );
  });

  test("plain HTML text IS escaped (React's normal behavior)", () => {
    expect(compileTemplate(h("p", null, "a < b & c > d"))).toBe("<p>a &lt; b &amp; c &gt; d</p>");
  });

  test("attribute values are escaped", () => {
    expect(compileTemplate(h("div", { title: 'x"y&z' }, "t"))).toBe(
      '<div title="x&quot;y&amp;z">t</div>',
    );
  });
});

describe("paged-media -> t: directive elements", () => {
  test("RunningHeader / RunningFooter with extent", () => {
    expect(compileTemplate(h(RunningHeader, { extent: "14mm" }, "H"))).toBe(
      '<t:running-header extent="14mm">H</t:running-header>',
    );
    expect(compileTemplate(h(RunningFooter, null, "F"))).toBe(
      "<t:running-footer>F</t:running-footer>",
    );
  });

  test("PageMaster with region and variant", () => {
    const out = compileTemplate(
      h(
        PageMaster,
        { name: "default", size: "A4", margin: "20mm" },
        h(Region, { slot: "header", extent: "14mm" }, "hdr"),
        h(Variant, { kind: "first" }, h(Region, { slot: "header", extent: "0mm" })),
      ),
    );
    expect(out).toBe(
      '<t:page-master name="default" size="A4" margin="20mm">' +
        '<t:region slot="header" extent="14mm">hdr</t:region>' +
        '<t:variant kind="first"><t:region slot="header" extent="0mm"></t:region></t:variant>' +
        "</t:page-master>",
    );
  });

  test("Page / Pages convenience elements", () => {
    expect(compileTemplate(h(Page, null))).toBe("<t:page></t:page>");
    expect(compileTemplate(h(Pages, null))).toBe("<t:pages></t:pages>");
  });

  test("Footnote with reset attribute maps to t:footnote-reset", () => {
    expect(compileTemplate(h(Footnote, { reset: "page" }, "note"))).toBe(
      '<t:footnote t:footnote-reset="page">note</t:footnote>',
    );
  });

  test("Counter directive", () => {
    expect(compileTemplate(h(Counter, { name: "figure", action: "increment" }))).toBe(
      '<t:counter name="figure" action="increment"></t:counter>',
    );
  });

  test("UseMaster directive", () => {
    expect(compileTemplate(h(UseMaster, { name: "cover" }))).toBe(
      '<t:use-master name="cover"></t:use-master>',
    );
  });

  test("Running emits the t:running attribute on a host element, not a t: element", () => {
    expect(compileTemplate(h(Running, { name: "section", as: "h1" }, "Title"))).toBe(
      '<h1 t:running="section">Title</h1>',
    );
  });
});

describe("plain HTML passthrough", () => {
  test("className + object style render through React's intrinsic model", () => {
    const out = compileTemplate(
      h("div", { className: "box", style: { color: "#111" } }, h("span", null, "hi")),
    );
    expect(out).toBe('<div class="box" style="color:#111"><span>hi</span></div>');
  });

  test("Raw escape hatch emits literal string attributes verbatim", () => {
    const out = compileTemplate(
      h(Raw, { html: '<div class="box" style="color:#111"><span>hi</span></div>' }),
    );
    expect(out).toBe('<div class="box" style="color:#111"><span>hi</span></div>');
  });
});

describe("compileTemplate options", () => {
  test("trims outer whitespace by default", () => {
    expect(compileTemplate(h("p", null, "x"))).toBe("<p>x</p>");
  });

  test("trim:false preserves the raw render", () => {
    expect(compileTemplate(h(InvoiceDoc, null), { trim: false }).trim()).toBe(
      CANONICAL_INVOICE_TEMPLATE,
    );
  });
});
