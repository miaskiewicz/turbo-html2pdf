// Extract ALL pages before timing. A page-1 image cannot detect omitted prose or notes.
import { execFile } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import type { RowData, Workload } from "../types.ts";

const exec = promisify(execFile);

function rowText(row: RowData): string {
  return `${row.id} ${row.description} ${row.qty} ${row.unit.toFixed(2)} ${row.amount.toFixed(2)}`;
}

function requiredText(w: Workload): string {
  return [
    w.content.title,
    w.content.subtitle,
    ...w.content.rows.map(rowText),
    ...w.content.paragraphs,
    ...w.content.footnotes,
  ].join(" ");
}

function wordCounts(text: string): Map<string, number> {
  const counts = new Map<string, number>();
  const words =
    text
      .normalize("NFKC")
      .toLowerCase()
      .match(/[\p{L}\p{N}]+/gu) ?? [];
  for (const word of words) counts.set(word, (counts.get(word) ?? 0) + 1);
  return counts;
}

/** Ignore wrapping/header repetition, but require every source word occurrence. */
export function assertWorkloadText(text: string, w: Workload): void {
  const actual = wordCounts(text);
  for (const [word, count] of wordCounts(requiredText(w))) {
    if ((actual.get(word) ?? 0) < count) {
      throw new Error(
        `incomplete PDF content for ${w.id}: missing occurrences of ${JSON.stringify(word)}`,
      );
    }
  }
}

/** Poppler's pdftotext reads every page by default; unavailable extraction fails closed. */
export async function assertWorkloadPdf(pdf: Uint8Array, w: Workload): Promise<void> {
  const dir = await mkdtemp(join(tmpdir(), "tpdf-content-"));
  try {
    const input = join(dir, "input.pdf");
    await writeFile(input, pdf);
    const { stdout } = await exec("pdftotext", ["-enc", "UTF-8", input, "-"], {
      timeout: 30000,
      maxBuffer: 32 * 1024 * 1024,
    });
    assertWorkloadText(stdout, w);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}
