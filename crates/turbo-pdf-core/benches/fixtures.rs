//! Deterministic fixture generator for the render/layout benchmarks.
//!
//! Produces template + data pairs that exercise the pipeline that exists today
//! (compile -> render_nodes -> style cascade -> layout galley). Everything here
//! is data-driven and clock-free: row values are generated from the row index,
//! never from `Date`/random, so a given `n` always yields byte-identical markup.
//!
//! Templates use only documented engine features: the standard Jinja control
//! flow (`{% for %}`, `{% if %}`) plus the registered filters
//! (`currency`, `number`, `percent`, `pad`) verified against
//! `src/template/filters.rs`. No `t:` paged-media directives are used: those are
//! consumed later in the pipeline (pagination/PDF emit) which is out of scope
//! until Phase 9.

use serde_json::{json, Value};

/// The Evolventa + Go fonts loaded from the in-repo `assets/fonts/` directory
/// (never a system path), mirroring `tests/common/mod.rs`.
pub fn registry() -> turbo_pdf_core::text::FontRegistry {
    use turbo_pdf_core::text::FontRegistry;
    use turbo_pdf_core::FontFace;

    fn font_bytes(name: &str) -> Vec<u8> {
        let path = format!("{}/assets/fonts/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read(&path).unwrap_or_else(|_| panic!("font fixture {path}"))
    }

    let mut reg = FontRegistry::new();
    reg.add(
        FontFace::from_bytes(font_bytes("Evolventa-zLXL.ttf"), "Evolventa", 400, false).unwrap(),
    );
    reg.add(
        FontFace::from_bytes(
            font_bytes("EvolventaBold-55Xv.ttf"),
            "Evolventa",
            700,
            false,
        )
        .unwrap(),
    );
    reg.add(FontFace::from_bytes(font_bytes("Go-Regular.ttf"), "Go", 400, false).unwrap());
    reg
}

/// A named template + the JSON data to render it against.
pub struct Fixture {
    pub name: &'static str,
    pub template: String,
    pub data: Value,
}

/// A one-page invoice: a styled header block, a small line-item table driven by
/// `{% for %}`, and a totals row using the `currency`/`number`/`percent`
/// filters. Single page's worth of content.
pub fn invoice() -> Fixture {
    let template = r#"<div style="font-family: Evolventa; font-size: 11px">
  <div style="font-size: 20px; font-weight: 700">Invoice {{ invoice.number }}</div>
  <div>Billed to: {{ customer.name }}</div>
  <div>Date: {{ invoice.date }}</div>
  <table style="width: 100%">
    <tr style="font-weight: 700">
      <td>Item</td><td>Qty</td><td>Unit</td><td>Amount</td>
    </tr>
    {% for line in invoice.lines %}
    <tr>
      <td>{{ line.desc }}</td>
      <td>{{ line.qty | number }}</td>
      <td>{{ line.unit | currency("USD") }}</td>
      <td>{{ (line.qty * line.unit) | currency("USD") }}</td>
    </tr>
    {% endfor %}
  </table>
  <div>Subtotal: {{ invoice.subtotal | currency("USD") }}</div>
  <div>Tax ({{ invoice.tax_rate | percent }}): {{ invoice.tax | currency("USD") }}</div>
  <div style="font-weight: 700">Total: {{ invoice.total | currency("USD") }}</div>
</div>"#;

    // A fixed set of line items so the invoice is exactly one page and fully
    // deterministic.
    let descs = [
        "Consulting services",
        "Design review",
        "Implementation",
        "QA and testing",
        "Deployment support",
    ];
    let mut lines = Vec::new();
    let mut subtotal = 0.0_f64;
    for (i, desc) in descs.iter().enumerate() {
        let qty = (i + 1) as f64;
        let unit = 100.0 + (i as f64) * 25.0;
        subtotal += qty * unit;
        lines.push(json!({ "desc": desc, "qty": qty, "unit": unit }));
    }
    let tax_rate = 0.23_f64;
    let tax = subtotal * tax_rate;
    let total = subtotal + tax;

    let data = json!({
        "invoice": {
            "number": "INV-0001",
            "date": "2026-06-14",
            "lines": lines,
            "subtotal": subtotal,
            "tax_rate": tax_rate,
            "tax": tax,
            "total": total,
        },
        "customer": { "name": "Acme Corporation" },
    });

    Fixture {
        name: "invoice",
        template: template.to_string(),
        data,
    }
}

/// A tabular report of `n` rows. The template is fixed; only the row count in
/// the data varies, so the three report sizes share one template and differ
/// purely in volume of work.
pub fn report(name: &'static str, n: usize) -> Fixture {
    let template = r#"<div style="font-family: Evolventa; font-size: 10px">
  <div style="font-size: 18px; font-weight: 700">{{ title }}</div>
  <table style="width: 100%">
    <tr style="font-weight: 700">
      <td>#</td><td>Name</td><td>Region</td><td>Units</td><td>Revenue</td><td>Share</td>
    </tr>
    {% for row in rows %}
    <tr>
      <td>{{ row.id | pad(5) }}</td>
      <td>{{ row.name }}</td>
      <td>{{ row.region }}</td>
      <td>{{ row.units | number }}</td>
      <td>{{ row.revenue | currency("USD") }}</td>
      <td>{{ row.share | percent(1) }}</td>
    </tr>
    {% endfor %}
  </table>
</div>"#;

    let regions = ["North", "South", "East", "West"];
    let mut total_units: u64 = 0;
    for i in 0..n {
        total_units += (1 + (i % 97)) as u64;
    }
    let total_units = total_units.max(1) as f64;

    let mut rows = Vec::with_capacity(n);
    for i in 0..n {
        let units = (1 + (i % 97)) as f64;
        let revenue = units * (10.0 + (i % 13) as f64);
        rows.push(json!({
            "id": i + 1,
            "name": format!("Account {:04}", i + 1),
            "region": regions[i % regions.len()],
            "units": units,
            "revenue": revenue,
            "share": units / total_units,
        }));
    }

    let data = json!({
        "title": format!("Sales Report ({} rows)", n),
        "rows": rows,
    });

    Fixture {
        name,
        template: template.to_string(),
        data,
    }
}

/// All benchmark fixtures: the invoice plus report at N in {100, 1000, 10000}.
pub fn all() -> Vec<Fixture> {
    vec![
        invoice(),
        report("report-100", 100),
        report("report-1k", 1_000),
        report("report-10k", 10_000),
    ]
}
