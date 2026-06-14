// Workload registry. Each workload is one logical document (spec §10.2) rendered
// identically by every engine. Geometry is shared (A4, uniform margin) so PNG
// equivalence (compare/) is meaningful.

import type { PageGeometry, Workload, WorkloadId } from "../types.ts";
import { generateContent } from "./data.ts";

/** A4 at 72 dpi (PDF points). 1mm = 2.834645669 pt; 18mm margin. */
export const A4: PageGeometry = {
  size: "A4",
  widthPt: 595.28,
  heightPt: 841.89,
  marginPt: 51.02,
};

interface WorkloadSpec {
  readonly id: WorkloadId;
  readonly title: string;
  readonly subtitle: string;
  readonly rows: number;
  readonly paragraphs: number;
  readonly footnotes: number;
}

const SPECS: readonly WorkloadSpec[] = [
  { id: "invoice", title: "Invoice", subtitle: "Acme Corp", rows: 12, paragraphs: 0, footnotes: 0 },
  {
    id: "report-100",
    title: "Report",
    subtitle: "100 rows",
    rows: 100,
    paragraphs: 0,
    footnotes: 0,
  },
  {
    id: "report-1k",
    title: "Report",
    subtitle: "1k rows",
    rows: 1000,
    paragraphs: 0,
    footnotes: 0,
  },
  {
    id: "report-10k",
    title: "Report",
    subtitle: "10k rows",
    rows: 10000,
    paragraphs: 0,
    footnotes: 0,
  },
  {
    id: "legal",
    title: "Agreement",
    subtitle: "Master Services",
    rows: 0,
    paragraphs: 40,
    footnotes: 12,
  },
  {
    id: "mixed",
    title: "Mixed",
    subtitle: "Headings + table + flow",
    rows: 30,
    paragraphs: 8,
    footnotes: 0,
  },
];

function build(spec: WorkloadSpec): Workload {
  return {
    id: spec.id,
    geometry: A4,
    content: generateContent(spec),
  };
}

/** All workloads, keyed by id. */
export const WORKLOADS: ReadonlyMap<WorkloadId, Workload> = new Map(
  SPECS.map((s) => [s.id, build(s)]),
);

/** Ordered list of workload ids (cheap → expensive). */
export const WORKLOAD_IDS: readonly WorkloadId[] = SPECS.map((s) => s.id);

/** Look up one workload; throws on unknown id (caller validates input). */
export function getWorkload(id: WorkloadId): Workload {
  const w = WORKLOADS.get(id);
  if (!w) throw new Error(`unknown workload: ${id}`);
  return w;
}
