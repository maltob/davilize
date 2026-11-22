import { useState, type DragEvent } from "react";
import type { ChartType, ColumnView, Filter, Visual } from "../types";
import {
  AGGREGATES,
  CHART_TYPES,
  FILTER_OPS,
  SLOTS,
  familyOf,
  formatFilterValues,
  outputNames,
  parseFilterValues,
  type Pill,
  type PillAgg,
  type Slot,
  type Slots,
} from "../model";
import { FIELD_MIME, type FieldTarget } from "./DataPanel";

interface Props {
  visual: Visual;
  slots: Slots;
  columns: ColumnView[];
  onSlot: (slot: Slot, pill: Pill | null) => void;
  onFieldDrop: (column: ColumnView, target: FieldTarget) => void;
  onChange: (patch: Partial<Visual>) => void;
}

const control =
  "rounded-md border border-border bg-surface px-2 py-1 text-sm text-text focus-visible:outline-2 focus-visible:outline-accent";

function readDrop(event: DragEvent): ColumnView | null {
  const raw = event.dataTransfer.getData(FIELD_MIME);
  if (!raw) return null;
  try {
    return JSON.parse(raw) as ColumnView;
  } catch {
    return null;
  }
}

/** Drop-target behaviour shared by the slots and the filter shelf. */
function dropProps(onDrop: (column: ColumnView) => void, setOver: (over: boolean) => void) {
  return {
    onDragOver: (event: DragEvent) => {
      if (event.dataTransfer.types.includes(FIELD_MIME)) {
        event.preventDefault();
        event.dataTransfer.dropEffect = "copy";
        setOver(true);
      }
    },
    onDragLeave: () => setOver(false),
    onDrop: (event: DragEvent) => {
      event.preventDefault();
      setOver(false);
      const column = readDrop(event);
      if (column) onDrop(column);
    },
  };
}

export function Shelves({ visual, slots, columns, onSlot, onFieldDrop, onChange }: Props) {
  return (
    <div className="flex flex-col gap-3">
      <div className="grid grid-cols-1 gap-2 md:grid-cols-2">
        {SLOTS.map((slot) => (
          <ShelfSlot
            key={slot.key}
            label={slot.label}
            hint={slot.hint}
            pill={slots[slot.key]}
            onDrop={(column) => onFieldDrop(column, slot.key)}
            onChange={(pill) => onSlot(slot.key, pill)}
          />
        ))}
      </div>

      <div className="grid grid-cols-1 gap-3 xl:grid-cols-[1fr_20rem]">
        <FiltersShelf
          visual={visual}
          columns={columns}
          onChange={onChange}
          onDrop={(column) => onFieldDrop(column, "filter")}
        />
        <div className="flex flex-col gap-3">
          <fieldset className="flex flex-col gap-1">
            <legend className="text-xs font-semibold uppercase tracking-wide text-muted">Chart type</legend>
            <div role="radiogroup" aria-label="Chart type" className="grid grid-cols-5 gap-1">
              {CHART_TYPES.map((chart) => {
                const checked = visual.chart === chart.value;
                return (
                  <label
                    key={chart.value}
                    className={`cursor-pointer rounded-md border px-1 py-1 text-center text-xs font-medium focus-within:outline-2 focus-within:outline-accent ${
                      checked ? "border-accent bg-accent/15" : "border-border bg-surface hover:bg-surface-2"
                    }`}
                  >
                    <input
                      type="radio"
                      name="chart-type"
                      value={chart.value}
                      checked={checked}
                      onChange={() => onChange({ chart: chart.value as ChartType })}
                      className="sr-only"
                    />
                    {chart.label}
                  </label>
                );
              })}
            </div>
          </fieldset>
          <div className="grid grid-cols-2 gap-2">
            <label className="flex flex-col gap-1 text-xs font-semibold uppercase tracking-wide text-muted">
              Sort by
              <select
                className={`${control} normal-case tracking-normal`}
                value={visual.sort?.by ?? ""}
                onChange={(event) => {
                  const by = event.target.value;
                  onChange({ sort: by ? { by, direction: visual.sort?.direction ?? "desc" } : null });
                }}
              >
                <option value="">None</option>
                {outputNames(visual).map((name) => (
                  <option key={name} value={name}>
                    {name}
                  </option>
                ))}
              </select>
            </label>
            <label className="flex flex-col gap-1 text-xs font-semibold uppercase tracking-wide text-muted">
              Direction
              <select
                className={`${control} normal-case tracking-normal`}
                value={visual.sort?.direction ?? "desc"}
                disabled={!visual.sort}
                onChange={(event) =>
                  visual.sort &&
                  onChange({ sort: { by: visual.sort.by, direction: event.target.value as "asc" | "desc" } })
                }
              >
                <option value="desc">Descending</option>
                <option value="asc">Ascending</option>
              </select>
            </label>
            <label className="col-span-2 flex flex-col gap-1 text-xs font-semibold uppercase tracking-wide text-muted">
              Row limit
              <input
                type="number"
                min={1}
                placeholder="No limit"
                className={`${control} normal-case tracking-normal`}
                value={visual.limit ?? ""}
                onChange={(event) => {
                  const value = event.target.value;
                  const limit = value === "" ? null : Math.max(1, Math.floor(Number(value)));
                  onChange({ limit: limit !== null && Number.isFinite(limit) ? limit : null });
                }}
              />
            </label>
          </div>
        </div>
      </div>
    </div>
  );
}

function ShelfSlot({
  label,
  hint,
  pill,
  onDrop,
  onChange,
}: {
  label: string;
  hint: string;
  pill: Pill | null;
  onDrop: (column: ColumnView) => void;
  onChange: (pill: Pill | null) => void;
}) {
  const [over, setOver] = useState(false);
  return (
    <section
      aria-label={`${label} shelf`}
      {...dropProps(onDrop, setOver)}
      className={`flex min-h-16 flex-col gap-1 rounded-md border-2 border-dashed p-2 ${
        over ? "border-accent bg-accent/10" : "border-border bg-surface"
      }`}
    >
      <div className="flex items-baseline justify-between gap-2">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-muted">{label}</h3>
        <span className="text-xs text-muted">{hint}</span>
      </div>
      {pill ? <PillView label={label} pill={pill} onChange={onChange} /> : <p className="text-sm text-muted">Drop a field here, or use Add to on the field list.</p>}
    </section>
  );
}

function PillView({ label, pill, onChange }: { label: string; pill: Pill; onChange: (pill: Pill | null) => void }) {
  const id = `${label.replace(/\s+/g, "-").toLowerCase()}-${pill.field}`.replace(/[^A-Za-z0-9_-]/g, "_");
  return (
    <div className="flex flex-wrap items-center gap-2 rounded-md bg-surface-2 px-2 py-1">
      <span className="min-w-0 truncate font-medium" title={pill.field}>
        {pill.field}
      </span>
      <label className="sr-only" htmlFor={`${id}-kind`}>
        Field kind for {pill.field} on {label}
      </label>
      <select
        id={`${id}-kind`}
        className={`${control} py-0.5 text-xs`}
        value={pill.kind}
        onChange={(event) => {
          const kind = event.target.value as Pill["kind"];
          onChange({ field: pill.field, kind, agg: kind === "measure" ? "sum" : pill.agg });
        }}
      >
        <option value="dimension">Dimension</option>
        <option value="measure">Measure</option>
      </select>
      {pill.kind === "measure" && (
        <>
          <label className="sr-only" htmlFor={`${id}-agg`}>
            Aggregation for {pill.field} on {label}
          </label>
          <select
            id={`${id}-agg`}
            className={`${control} py-0.5 text-xs`}
            value={pill.agg}
            onChange={(event) => onChange({ ...pill, agg: event.target.value as PillAgg })}
          >
            {AGGREGATES.map((agg) => (
              <option key={agg.value} value={agg.value}>
                {agg.label}
              </option>
            ))}
          </select>
        </>
      )}
      <button
        type="button"
        onClick={() => onChange(null)}
        aria-label={`Remove ${pill.field} from ${label}`}
        className="ml-auto rounded px-2 py-0.5 text-sm text-muted hover:bg-surface hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
      >
        Remove
      </button>
    </div>
  );
}

function FiltersShelf({
  visual,
  columns,
  onChange,
  onDrop,
}: {
  visual: Visual;
  columns: ColumnView[];
  onChange: (patch: Partial<Visual>) => void;
  onDrop: (column: ColumnView) => void;
}) {
  const [over, setOver] = useState(false);
  const setFilters = (filters: Filter[]) => onChange({ filters });
  const update = (index: number, patch: Partial<Filter>) =>
    setFilters(visual.filters.map((f, i) => (i === index ? { ...f, ...patch } : f)));
  const first = columns[0];

  return (
    <section
      aria-label="Filters shelf"
      {...dropProps(onDrop, setOver)}
      className={`flex flex-col gap-2 rounded-md border-2 border-dashed p-2 ${
        over ? "border-accent bg-accent/10" : "border-border bg-surface"
      }`}
    >
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-muted">Filters</h3>
        <button
          type="button"
          onClick={() => first && setFilters([...visual.filters, { field: first.name, op: "is_not_null", values: [] }])}
          disabled={!first}
          className="rounded-md border border-border px-2 py-0.5 text-xs hover:bg-surface-2 disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-accent"
        >
          Add filter
        </button>
      </div>
      {visual.filters.length === 0 && <p className="text-sm text-muted">No filters.</p>}
      {visual.filters.map((filter, index) => {
        const op = FILTER_OPS.find((o) => o.value === filter.op);
        const family = familyOf(columns, filter.field);
        return (
          <div key={index} className="flex flex-wrap items-center gap-2">
            <label className="sr-only" htmlFor={`filter-field-${index}`}>
              Filter field
            </label>
            <select
              id={`filter-field-${index}`}
              className={`${control} py-0.5`}
              value={filter.field}
              onChange={(e) => update(index, { field: e.target.value })}
            >
              {columns.map((c) => (
                <option key={c.name} value={c.name}>
                  {c.name}
                </option>
              ))}
            </select>
            <label className="sr-only" htmlFor={`filter-op-${index}`}>
              Filter condition
            </label>
            <select
              id={`filter-op-${index}`}
              className={`${control} py-0.5`}
              value={filter.op}
              onChange={(e) => update(index, { op: e.target.value as Filter["op"] })}
            >
              {FILTER_OPS.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
            {op?.needsValue && (
              <>
                <label className="sr-only" htmlFor={`filter-value-${index}`}>
                  Filter value, comma separated
                </label>
                <input
                  id={`filter-value-${index}`}
                  type="text"
                  placeholder={filter.op === "in" ? "a, b, c" : "value"}
                  className={`${control} w-40 py-0.5`}
                  defaultValue={formatFilterValues(filter.values)}
                  key={`${index}-${filter.op}-${filter.field}`}
                  onBlur={(e) => update(index, { values: parseFilterValues(e.target.value, family) })}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") update(index, { values: parseFilterValues(e.currentTarget.value, family) });
                  }}
                />
              </>
            )}
            <button
              type="button"
              onClick={() => setFilters(visual.filters.filter((_, i) => i !== index))}
              aria-label={`Remove filter on ${filter.field}`}
              className="rounded px-2 py-0.5 text-sm text-muted hover:bg-surface-2 hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
            >
              Remove
            </button>
          </div>
        );
      })}
    </section>
  );
}
