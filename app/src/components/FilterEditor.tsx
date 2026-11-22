import { useEffect, useState } from "react";
import type { ColumnView, Filter, FilterOp, FilterScope, Parameter, Visual } from "../types";
import {
  FILTER_OPS,
  FILTER_SCOPES,
  filterFamily,
  filterFieldOptions,
  filterScope,
  valueDraft,
  valueDrafts,
  valueFromDraft,
  withScope,
  type ValueDraft,
} from "../model";

/** Controls are at least 24px tall and text is at least 12px. */
const control =
  "min-h-6 rounded-md border border-border bg-surface px-2 py-0.5 text-sm text-text focus-visible:outline-2 focus-visible:outline-accent";

interface Props {
  /** Heading shown above the list, for example "Filters" or "Shared filters". */
  title: string;
  filters: Filter[];
  columns: ColumnView[];
  parameters: Parameter[];
  /**
   * The visual the filters belong to: totals can then be filtered too. Omit it for
   * shared dashboard filters, which are row filters only.
   */
  visual?: Visual;
  onChange: (filters: Filter[]) => void;
}

/** A list of filters with add and remove, used by the chart builder and by dashboards. */
export function FilterEditor({ title, filters, columns, parameters, visual, onChange }: Props) {
  const replace = (index: number, next: Filter) => onChange(filters.map((f, i) => (i === index ? next : f)));
  const first = columns[0];

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-muted">{title}</h3>
        <button
          type="button"
          onClick={() => first && onChange([...filters, { field: first.name, op: "is_not_null", values: [] }])}
          disabled={!first}
          className="min-h-6 rounded-md border border-border px-2 py-0.5 text-xs hover:bg-surface-2 disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-accent"
        >
          Add filter
        </button>
      </div>
      {filters.length === 0 && <p className="text-sm text-muted">No filters.</p>}
      {filters.map((filter, index) => (
        <FilterRow
          key={index}
          index={index}
          filter={filter}
          visual={visual}
          columns={columns}
          parameters={parameters}
          onChange={(next) => replace(index, next)}
          onRemove={() => onChange(filters.filter((_, i) => i !== index))}
        />
      ))}
      {parameters.length === 0 && (
        <p className="text-xs text-muted">Declare a parameter in the Parameters panel to filter by a runtime value.</p>
      )}
    </div>
  );
}

/** One filter: its scope, field, condition and values. */
function FilterRow({
  index,
  filter,
  visual,
  columns,
  parameters,
  onChange,
  onRemove,
}: {
  index: number;
  filter: Filter;
  visual: Visual | undefined;
  columns: ColumnView[];
  parameters: Parameter[];
  onChange: (next: Filter) => void;
  onRemove: () => void;
}) {
  const scope = filterScope(filter);
  const options = filterFieldOptions(scope, visual, columns);
  // A field that is not offered here (for example from a hand-written file) stays visible.
  const fieldChoices = options.includes(filter.field) ? options : [filter.field, ...options];
  const family = filterFamily(filter, visual, columns);
  const slots = FILTER_OPS.find((o) => o.value === filter.op)?.slots ?? "list";
  const drafts = valueDrafts(filter);
  const writeValues = (next: ValueDraft[]) => onChange({ ...filter, values: next.map((d) => valueFromDraft(d, family)) });

  const changeScope = (next: FilterScope) => {
    const nextOptions = filterFieldOptions(next, visual, columns);
    const field = nextOptions.includes(filter.field) ? filter.field : (nextOptions[0] ?? filter.field);
    onChange({ ...withScope(filter, next), field });
  };

  const changeOp = (op: FilterOp) => {
    const nextSlots = FILTER_OPS.find((o) => o.value === op)?.slots ?? "list";
    const keep = nextSlots === "none" ? 0 : nextSlots === "one" ? 1 : nextSlots === "two" ? 2 : undefined;
    const values = filter.values.map(valueDraft).slice(0, keep);
    onChange({ ...filter, op, values: values.map((d) => valueFromDraft(d, family)) });
  };

  return (
    <div className="flex flex-col gap-2 rounded-md bg-surface-2 p-2">
      <div className="flex flex-wrap items-center gap-2">
        {visual ? (
          <>
            <label className="sr-only" htmlFor={`filter-scope-${index}`}>
              Filter {index + 1} applies to
            </label>
            <select
              id={`filter-scope-${index}`}
              className={`${control} py-0.5`}
              value={scope}
              onChange={(e) => changeScope(e.target.value as FilterScope)}
            >
              {FILTER_SCOPES.map((s) => (
                <option key={s.value} value={s.value}>
                  {s.label}
                </option>
              ))}
            </select>
          </>
        ) : (
          <span className="text-xs text-muted">Rows</span>
        )}
        <label className="sr-only" htmlFor={`filter-field-${index}`}>
          Filter field
        </label>
        <select
          id={`filter-field-${index}`}
          className={`${control} py-0.5`}
          value={filter.field}
          onChange={(e) => onChange({ ...filter, field: e.target.value })}
        >
          {fieldChoices.map((name) => (
            <option key={name} value={name}>
              {name}
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
          onChange={(e) => changeOp(e.target.value as FilterOp)}
        >
          {FILTER_OPS.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
        {scope === "aggregate" && options.length === 0 && (
          <span className="text-xs text-muted">Add a measure to filter totals.</span>
        )}
        {!visual && scope === "aggregate" && (
          <span role="alert" className="text-xs text-danger">
            Dashboards filter rows only. Remove this filter.
          </span>
        )}
        <button
          type="button"
          onClick={onRemove}
          aria-label={`Remove filter on ${filter.field}`}
          className="ml-auto min-h-6 rounded px-2 py-0.5 text-sm text-muted hover:bg-surface hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
        >
          Remove
        </button>
      </div>

      {slots !== "none" &&
        drafts.map((draft, i) => (
          <ValueRow
            key={i}
            id={`filter-${index}-value-${i}`}
            label={slots === "two" ? (i === 0 ? "Lower" : "Upper") : slots === "list" ? `Value ${i + 1}` : "Value"}
            draft={draft}
            parameters={parameters}
            numeric={family === "number"}
            onChange={(next) => writeValues(drafts.map((d, j) => (j === i ? next : d)))}
            onRemove={slots === "list" && drafts.length > 1 ? () => writeValues(drafts.filter((_, j) => j !== i)) : undefined}
          />
        ))}
      {slots === "list" && (
        <button
          type="button"
          onClick={() => writeValues([...drafts, { kind: "text", text: "" }])}
          className="min-h-6 self-start rounded-md border border-border px-2 py-0.5 text-xs hover:bg-surface focus-visible:outline-2 focus-visible:outline-accent"
        >
          Add value
        </button>
      )}
    </div>
  );
}

/** One value of a filter: typed text, empty (NULL), or a declared parameter. */
function ValueRow({
  id,
  label,
  draft,
  parameters,
  numeric,
  onChange,
  onRemove,
}: {
  id: string;
  label: string;
  draft: ValueDraft;
  parameters: Parameter[];
  numeric: boolean;
  onChange: (next: ValueDraft) => void;
  onRemove?: () => void;
}) {
  const storedText = draft.kind === "text" ? draft.text : null;
  const [text, setText] = useState(storedText ?? "");
  // Follow edits made elsewhere (a removed filter shifts rows), but keep typing in progress.
  useEffect(() => {
    if (storedText !== null) setText(storedText);
  }, [storedText]);

  const sourceValue = draft.kind === "param" ? `param:${draft.name}` : draft.kind === "null" ? "null" : "text";
  const declared = draft.kind === "param" && parameters.some((p) => p.name === draft.name);
  const commit = () => {
    if (draft.kind === "text" && text !== draft.text) onChange({ kind: "text", text });
  };

  return (
    <div className="flex flex-wrap items-center gap-2 pl-2">
      <label htmlFor={`${id}-source`} className="text-xs text-muted">
        {label}
      </label>
      <select
        id={`${id}-source`}
        className={`${control} py-0.5`}
        value={sourceValue}
        onChange={(e) => {
          const value = e.target.value;
          if (value === "text") onChange({ kind: "text", text: "" });
          else if (value === "null") onChange({ kind: "null" });
          else onChange({ kind: "param", name: value.slice("param:".length) });
        }}
      >
        <option value="text">Typed value</option>
        <option value="null">Empty (NULL)</option>
        {parameters.map((p) => (
          <option key={p.name} value={`param:${p.name}`}>
            Parameter: {p.name}
          </option>
        ))}
        {draft.kind === "param" && !declared && (
          <option value={sourceValue}>Parameter: {draft.name} (not declared)</option>
        )}
      </select>
      {draft.kind === "text" && (
        <input
          id={id}
          type="text"
          inputMode={numeric ? "decimal" : undefined}
          aria-label={`${label} value`}
          placeholder={numeric ? "number" : "value"}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit();
          }}
          className={`${control} w-36 py-0.5`}
        />
      )}
      {onRemove && (
        <button
          type="button"
          onClick={onRemove}
          aria-label={`Remove ${label.toLowerCase()}`}
          className="min-h-6 rounded px-2 py-0.5 text-xs text-muted hover:bg-surface hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
        >
          Remove value
        </button>
      )}
    </div>
  );
}
