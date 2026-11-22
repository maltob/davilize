// Pure logic that maps the shelf-based chart builder onto a dv-core Visual, and back.
// The visual is the only state: shelves are derived from it, so a saved visual reopens exactly.
import type { Aggregate, ColumnView, Family, Measure, Report, Visual } from "./types";

export type Slot = "x" | "y" | "color" | "size";
export type PillAgg = Aggregate | "raw";
export interface Pill {
  field: string;
  kind: "dimension" | "measure";
  agg: PillAgg;
}
export type Slots = Record<Slot, Pill | null>;

export const SLOTS: { key: Slot; label: string; hint: string }[] = [
  { key: "x", label: "Columns", hint: "x axis" },
  { key: "y", label: "Rows", hint: "y axis" },
  { key: "color", label: "Color", hint: "colour encoding" },
  { key: "size", label: "Size", hint: "size encoding (scatter)" },
];

export const AGGREGATES: { value: PillAgg; label: string }[] = [
  { value: "sum", label: "Sum" },
  { value: "avg", label: "Average" },
  { value: "min", label: "Minimum" },
  { value: "max", label: "Maximum" },
  { value: "count", label: "Count" },
  { value: "count_distinct", label: "Count distinct" },
  { value: "raw", label: "None (row values)" },
];

export const CHART_TYPES: { value: Visual["chart"]; label: string }[] = [
  { value: "bar", label: "Bar" },
  { value: "line", label: "Line" },
  { value: "scatter", label: "Scatter" },
  { value: "table", label: "Table" },
  { value: "heatmap", label: "Heatmap" },
];

export const FILTER_OPS: { value: Visual["filters"][number]["op"]; label: string; needsValue: boolean }[] = [
  { value: "eq", label: "equals", needsValue: true },
  { value: "ne", label: "does not equal", needsValue: true },
  { value: "lt", label: "less than", needsValue: true },
  { value: "le", label: "at most", needsValue: true },
  { value: "gt", label: "greater than", needsValue: true },
  { value: "ge", label: "at least", needsValue: true },
  { value: "in", label: "is one of", needsValue: true },
  { value: "is_null", label: "is empty", needsValue: false },
  { value: "is_not_null", label: "is not empty", needsValue: false },
];

export function outputName(measure: Measure): string {
  return measure.alias ?? `${measure.agg}_${measure.field}`;
}

export function emptySlots(): Slots {
  return { x: null, y: null, color: null, size: null };
}

export function blankVisual(dataset: string, id: string): Visual {
  return {
    id,
    title: "",
    dataset,
    chart: "bar",
    encodings: {},
    dimensions: [],
    measures: [],
    filters: [],
    sort: null,
    limit: null,
  };
}

/** A visual id that is not used in the report. Ids become file names, so only [A-Za-z0-9_-]. */
export function newVisualId(report: Report): string {
  const used = new Set(report.visuals.map((v) => v.id));
  let n = report.visuals.length + 1;
  while (used.has(`visual_${n}`)) n += 1;
  return `visual_${n}`;
}

export function slotsFromVisual(visual: Visual): Slots {
  const slots = emptySlots();
  for (const { key } of SLOTS) {
    const name = visual.encodings[key];
    if (!name) continue;
    const measure = visual.measures.find((m) => outputName(m) === name);
    if (measure) {
      slots[key] = { field: measure.field, kind: "measure", agg: measure.agg };
    } else if (visual.dimensions.includes(name)) {
      slots[key] = { field: name, kind: "dimension", agg: "sum" };
    } else {
      // Not grouped: a raw row value (scatter points, or table columns).
      slots[key] = { field: name, kind: "measure", agg: "raw" };
    }
  }
  return slots;
}

/** Writes the shelves into a visual's encodings, dimensions and measures. Other fields are kept. */
export function applySlots(visual: Visual, slots: Slots): Visual {
  const encodings: Visual["encodings"] = {};
  const dimensions: string[] = [];
  const measures: Measure[] = [];
  for (const { key } of SLOTS) {
    const pill = slots[key];
    if (!pill) {
      encodings[key] = null;
    } else if (pill.kind === "dimension") {
      if (!dimensions.includes(pill.field)) dimensions.push(pill.field);
      encodings[key] = pill.field;
    } else if (pill.agg === "raw") {
      encodings[key] = pill.field;
    } else {
      const measure: Measure = { field: pill.field, agg: pill.agg };
      const name = outputName(measure);
      if (!measures.some((m) => outputName(m) === name)) measures.push(measure);
      encodings[key] = name;
    }
  }
  return { ...visual, encodings, dimensions, measures };
}

export function hasFields(visual: Visual): boolean {
  // Hand-written reports (e.g. table visuals) may define columns only through
  // dimensions/measures, with no encodings.
  return (
    Object.values(visual.encodings).some((v) => Boolean(v)) ||
    visual.dimensions.length > 0 ||
    visual.measures.length > 0
  );
}

/** Field names a sort or a shelf can refer to. */
export function outputNames(visual: Visual): string[] {
  const names = [
    ...visual.dimensions,
    ...visual.measures.map(outputName),
    ...Object.values(visual.encodings).filter((v): v is string => Boolean(v)),
  ];
  return [...new Set(names)];
}

/** Default pill for a field dropped on a shelf. */
export function pillFor(column: ColumnView): Pill {
  return column.role === "measure"
    ? { field: column.name, kind: "measure", agg: "sum" }
    : { field: column.name, kind: "dimension", agg: "sum" };
}

export function autoTitle(visual: Visual): string {
  const names = outputNames(visual);
  return names.length ? names.join(" by ") : "Untitled visual";
}

/** Converts text typed for a filter into the JSON values dv-core expects. */
export function parseFilterValues(text: string, family: Family | undefined): unknown[] {
  return text
    .split(",")
    .map((part) => part.trim())
    .filter((part) => part.length > 0)
    .map((part) => {
      if (family === "number") {
        const n = Number(part);
        return Number.isFinite(n) ? n : part;
      }
      if (family === "bool") return part.toLowerCase() === "true";
      return part;
    });
}

export function formatFilterValues(values: unknown[]): string {
  return values.map((v) => String(v)).join(", ");
}

export function familyOf(columns: ColumnView[] | undefined, name: string): Family | undefined {
  return columns?.find((c) => c.name === name)?.family;
}
