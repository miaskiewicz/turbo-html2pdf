import { describe, expect, test } from "vitest";
import {
  caseBlock,
  compileTemplate,
  counter,
  defaultBlock,
  each,
  elseBlock,
  elseIf,
  expr,
  footnote,
  ifBlock,
  include,
  leader,
  page,
  pages,
  pageMaster,
  region,
  running,
  runningFooter,
  switchBlock,
  useMaster,
  variant,
} from "../src/index.js";

// The same canonical template the React frontend (AC-8.7) targets. Independent copy
// so AC-8.9 is a genuine cross-frontend equality check: a second, unrelated frontend
// must drive the *unchanged* core to the same template source.
const CANONICAL_INVOICE_TEMPLATE =
  '<t:running-footer extent="12mm">Page <t:page></t:page> of <t:pages></t:pages>' +
  "<t:leader></t:leader><span>&lt;flux &amp; co&gt;</span></t:running-footer>" +
  '<h1 t:running="section">{{ invoice.title }}</h1>' +
  '{% if invoice.status == "paid" %}<p class="muted">Paid in full.</p>' +
  '{% elif invoice.status == "overdue" %}<p>Overdue by {{ invoice.days_overdue }} days.</p>' +
  "{% else %}<p>Due soon.</p>{% endif %}" +
  '<table class="lines"><tbody>' +
  "{% for line in invoice.lines %}" +
  "<tr><td>{{ loop.index }}</td>" +
  "<td>{{ line.description }}{% if line.note %}<t:footnote>{{ line.note }}</t:footnote>{% endif %}</td>" +
  '<td class="right">{{ line.amount | currency(invoice.ccy) }}</td></tr>' +
  '{% else %}<tr><td colSpan="3" class="muted">No line items.</td></tr>{% endfor %}' +
  "</tbody></table>" +
  "{% switch customer.tier %}" +
  '{% case "enterprise" %}<p>Dedicated support included.</p>' +
  '{% case "pro", "plus" %}<p>Priority support included.</p>' +
  "{% default %}<p>Standard support.</p>{% endswitch %}" +
  '<t:region slot="footer">{{ doc.confidential }}</t:region>' +
  '{% include "remittance" with company.bank %}';

// Build the same document with the template-string frontend. Unlike React this
// frontend does not HTML-escape, so plain text/attributes are written literally
// (the `<flux & co>` text is hand-escaped, mirroring hand-authored HTML).
function buildInvoice(): string {
  return compileTemplate(
    runningFooter(
      { extent: "12mm" },
      "Page ",
      page(),
      " of ",
      pages(),
      leader(),
      "<span>&lt;flux &amp; co&gt;</span>",
    ),
    running({ name: "section", tag: "h1" }, expr("invoice.title")),
    ifBlock(
      'invoice.status == "paid"',
      '<p class="muted">Paid in full.</p>',
      elseIf(
        'invoice.status == "overdue"',
        `<p>Overdue by ${expr("invoice.days_overdue")} days.</p>`,
      ),
      elseBlock("<p>Due soon.</p>"),
    ),
    '<table class="lines"><tbody>',
    each(
      "invoice.lines",
      "line",
      `<tr><td>${expr("loop.index")}</td>`,
      `<td>${expr("line.description")}${ifBlock("line.note", footnote({}, expr("line.note")))}</td>`,
      `<td class="right">${expr("line.amount | currency(invoice.ccy)")}</td></tr>`,
      elseBlock('<tr><td colSpan="3" class="muted">No line items.</td></tr>'),
    ),
    "</tbody></table>",
    switchBlock(
      "customer.tier",
      caseBlock('"enterprise"', "<p>Dedicated support included.</p>"),
      caseBlock('"pro", "plus"', "<p>Priority support included.</p>"),
      defaultBlock("<p>Standard support.</p>"),
    ),
    region({ slot: "footer" }, expr("doc.confidential")),
    include('"remittance"', "company.bank"),
  );
}

describe("AC-8.9 frontend-agnostic core", () => {
  test("template-string frontend produces the canonical template too", () => {
    expect(buildInvoice()).toBe(CANONICAL_INVOICE_TEMPLATE);
  });
});

describe("template-string frontend unit coverage", () => {
  test("control flow", () => {
    expect(compileTemplate(ifBlock("a", "x", elseBlock("y")))).toBe(
      "{% if a %}x{% else %}y{% endif %}",
    );
    expect(compileTemplate(each("rows", "r", expr("r")))).toBe(
      "{% for r in rows %}{{ r }}{% endfor %}",
    );
    expect(compileTemplate(switchBlock("t", caseBlock('"a"', "A"), defaultBlock("D")))).toBe(
      '{% switch t %}{% case "a" %}A{% default %}D{% endswitch %}',
    );
    expect(compileTemplate(include('"p"'))).toBe('{% include "p" %}');
  });

  test("paged media", () => {
    expect(compileTemplate(pageMaster({ name: "d", size: "A4" }, region({ slot: "header" })))).toBe(
      '<t:page-master name="d" size="A4"><t:region slot="header"></t:region></t:page-master>',
    );
    expect(compileTemplate(variant("first", page()))).toBe(
      '<t:variant kind="first"><t:page></t:page></t:variant>',
    );
    expect(compileTemplate(useMaster("cover"))).toBe('<t:use-master name="cover"></t:use-master>');
    expect(compileTemplate(counter({ name: "fig", action: "increment" }))).toBe(
      '<t:counter name="fig" action="increment"></t:counter>',
    );
    expect(compileTemplate(footnote({ reset: "page" }, "n"))).toBe(
      '<t:footnote t:footnote-reset="page">n</t:footnote>',
    );
  });
});
