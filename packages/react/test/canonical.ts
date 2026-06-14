// The canonical hand-written turbo-pdf template for the representative invoice
// document — exactly what an author would write by hand with no frontend at all
// (spec §8.4: a hand-written template is a first-class input). The React frontend
// (AC-8.7) and the second template-string frontend (AC-8.9) must each compile to
// THIS byte-for-byte (modulo the insignificant outer whitespace `compileTemplate`
// trims).
//
// Kept as a single literal so the test is a true string-equality check, not a
// re-derivation through the same code under test.
//
// Note `colSpan="3"` (camelCase): React's `renderToStaticMarkup` keeps `colSpan`
// in its own casing rather than lowercasing it. html5ever lowercases all attribute
// names anyway, so `colSpan`/`colspan` parse identically in the core — the casing
// here only matters for *string* equality, and we match what React actually emits.

export const CANONICAL_INVOICE_TEMPLATE =
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
