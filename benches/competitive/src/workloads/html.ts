// HTML/CSS renderer for a workload. Shared by every HTML-based engine (Puppeteer,
// Playwright, Gotenberg, wkhtmltopdf, WeasyPrint) so they all consume identical
// markup, fonts and geometry — the basis for PNG equivalence (AC-10.10).

import { readFileSync } from "node:fs";
import type { RowData, Workload } from "../types.ts";
import { FONTS, FONT_FAMILY } from "./fonts.ts";

function fontFace(): string {
  // Inline as base64 data: URIs so there is zero network and zero @font-face path
  // ambiguity across the browser/weasyprint engines.
  const reg = readFileSync(FONTS.regular).toString("base64");
  const bold = readFileSync(FONTS.bold).toString("base64");
  return [
    `@font-face{font-family:'${FONT_FAMILY}';font-weight:400;`,
    `src:url(data:font/ttf;base64,${reg}) format('truetype');}`,
    `@font-face{font-family:'${FONT_FAMILY}';font-weight:700;`,
    `src:url(data:font/ttf;base64,${bold}) format('truetype');}`,
  ].join("");
}

function css(w: Workload): string {
  const mm = (w.geometry.marginPt / 2.834645669).toFixed(2);
  return [
    fontFace(),
    `@page{size:${w.geometry.size};margin:${mm}mm;}`,
    `*{box-sizing:border-box;}`,
    `body{font-family:'${FONT_FAMILY}',sans-serif;font-size:10pt;color:#111;margin:0;}`,
    `h1{font-size:20pt;font-weight:700;margin:0 0 4pt;}`,
    `.sub{color:#666;margin:0 0 12pt;}`,
    `table{width:100%;border-collapse:collapse;font-size:9pt;}`,
    `th{text-align:left;border-bottom:1.5pt solid #111;padding:3pt 4pt;}`,
    `td{border-bottom:0.5pt solid #ccc;padding:3pt 4pt;}`,
    `.right{text-align:right;}`,
    `p{margin:0 0 8pt;line-height:1.4;}`,
    `.fn{font-size:7.5pt;color:#444;border-top:0.5pt solid #999;margin-top:12pt;padding-top:6pt;}`,
  ].join("");
}

function rowHtml(r: RowData): string {
  return (
    `<tr><td>${r.id}</td><td>${r.description}</td>` +
    `<td class="right">${r.qty}</td>` +
    `<td class="right">${r.unit.toFixed(2)}</td>` +
    `<td class="right">${r.amount.toFixed(2)}</td></tr>`
  );
}

function tableHtml(w: Workload): string {
  if (w.content.rows.length === 0) return "";
  const head =
    "<thead><tr><th>#</th><th>Description</th><th class='right'>Qty</th>" +
    "<th class='right'>Unit</th><th class='right'>Amount</th></tr></thead>";
  const body = w.content.rows.map(rowHtml).join("");
  return `<table>${head}<tbody>${body}</tbody></table>`;
}

function proseHtml(w: Workload): string {
  const paras = w.content.paragraphs.map((p) => `<p>${p}</p>`).join("");
  const notes = w.content.footnotes.map((f, i) => `<div class="fn">${i + 1}. ${f}</div>`).join("");
  return paras + notes;
}

/** Full standalone HTML document for a workload. */
export function renderHtml(w: Workload): string {
  return [
    "<!doctype html><html><head><meta charset='utf-8'>",
    `<style>${css(w)}</style></head><body>`,
    `<h1>${w.content.title}</h1>`,
    `<div class="sub">${w.content.subtitle}</div>`,
    tableHtml(w),
    proseHtml(w),
    "</body></html>",
  ].join("");
}
