// Typst markup renderer for a workload. Produces equivalent content (same fonts via
// --font-path, same A4 geometry, same text) for the Typst typesetting engine.

import type { RowData, Workload } from "../types.ts";
import { FONT_FAMILY } from "./fonts.ts";

function esc(s: string): string {
  return s.replace(/([#*_`@$\\])/g, "\\$1");
}

function tableSrc(w: Workload): string {
  if (w.content.rows.length === 0) return "";
  const cells = (r: RowData) =>
    `[${r.id}], [${esc(r.description)}], [${r.qty}], [${r.unit.toFixed(2)}], [${r.amount.toFixed(2)}],`;
  const body = w.content.rows.map(cells).join("\n");
  return [
    "#table(columns: 5, stroke: 0.5pt,",
    "[*\\#*], [*Description*], [*Qty*], [*Unit*], [*Amount*],",
    body,
    ")",
  ].join("\n");
}

function proseSrc(w: Workload): string {
  const paras = w.content.paragraphs.map((p) => esc(p)).join("\n\n");
  const notes = w.content.footnotes.map((f, i) => `${i + 1}. ${esc(f)}`).join("\n");
  return `${paras}\n\n${notes}`;
}

/** Full Typst source document for a workload. */
export function renderTypst(w: Workload): string {
  const mm = (w.geometry.marginPt / 2.834645669).toFixed(2);
  return [
    `#set page(paper: "a4", margin: ${mm}mm)`,
    `#set text(font: "${FONT_FAMILY}", size: 10pt)`,
    `= ${esc(w.content.title)}`,
    `#text(fill: gray)[${esc(w.content.subtitle)}]`,
    "",
    tableSrc(w),
    "",
    proseSrc(w),
  ].join("\n");
}
