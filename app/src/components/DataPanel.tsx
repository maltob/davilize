import { useState } from "react";
import type { CalculatedField, ColumnView, Dataset, Parameter, Report, Source } from "../types";
import { SLOTS, describeJoin, type Slot } from "../model";
import { CalculatedFieldsSection } from "./CalculatedFields";
import { JoinsSection } from "./Joins";
import { ParametersSection } from "./Parameters";
import { SectionToggle, TypeBadge } from "./icons";

/** MIME type for field drags. The payload is the field's ColumnView as JSON. */
export const FIELD_MIME = "application/x-davilize-field";
export type FieldTarget = Slot | "filter";

interface Props {
  report: Report;
  reportPath: string | null;
  datasets: Dataset[];
  sources: Source[];
  schemas: Record<string, ColumnView[]>;
  active: string | null;
  busy: boolean;
  onSelect: (dataset: string) => void;
  onAddFiles: () => void;
  onAddField: (column: ColumnView, target: FieldTarget) => void;
  /** Each resolves to an error message, or null when the change was applied. */
  onSaveCalculated: (dataset: string, field: CalculatedField, original: string | null) => Promise<string | null>;
  onDeleteCalculated: (dataset: string, name: string) => Promise<string | null>;
  /** Each resolves to an error message, or null when the change was applied. */
  onSaveJoined: (dataset: Dataset, original: string | null) => Promise<string | null>;
  onDeleteJoined: (name: string) => Promise<string | null>;
  /** The joined dataset's form was closed without saving. */
  onCancelJoined: (name: string) => void;
  onSaveParameter: (param: Parameter, original: string | null) => void;
  onDeleteParameter: (name: string) => void;
}

const button =
  "min-h-7 whitespace-nowrap rounded-md border border-border bg-surface px-2 text-xs font-medium hover:bg-surface-2 disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-accent";

export function DataPanel({
  report,
  reportPath,
  datasets,
  sources,
  schemas,
  active,
  busy,
  onSelect,
  onAddFiles,
  onAddField,
  onSaveCalculated,
  onDeleteCalculated,
  onSaveJoined,
  onDeleteJoined,
  onCancelJoined,
  onSaveParameter,
  onDeleteParameter,
}: Props) {
  const [dataOpen, setDataOpen] = useState(true);
  const columns = active ? schemas[active] ?? [] : [];
  const dimensions = columns.filter((c) => c.role === "dimension");
  const measures = columns.filter((c) => c.role === "measure");

  return (
    <div className="flex h-full flex-col gap-4 overflow-y-auto p-3">
      <section aria-labelledby="sources-heading" className="flex flex-col gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <SectionToggle
            title="Data"
            open={dataOpen}
            controls="data-section"
            headingId="sources-heading"
            onToggle={() => setDataOpen((open) => !open)}
          />
          <button type="button" onClick={onAddFiles} disabled={busy} className={button}>
            Add data files
          </button>
        </div>
        <div id="data-section" hidden={!dataOpen}>
          {datasets.length === 0 ? (
            <p className="px-2 text-xs text-muted">Example: sales.csv, or open a report you saved earlier.</p>
          ) : (
            <ul className="flex flex-col gap-1" aria-label="Datasets">
              {datasets.map((dataset) => {
                const source = sources.find((s) => s.name === dataset.source);
                const pressed = dataset.name === active;
                const joined = (dataset.joins ?? []).length > 0;
                const joinText = joined ? describeJoin(dataset) : "";
                return (
                  <li key={dataset.name}>
                    <button
                      type="button"
                      aria-pressed={pressed}
                      onClick={() => onSelect(dataset.name)}
                      className={`flex min-h-9 w-full flex-col justify-center rounded-md px-2 py-1 text-left focus-visible:outline-2 focus-visible:outline-accent ${
                        pressed ? "bg-accent/15 text-text" : "hover:bg-surface-2"
                      }`}
                    >
                      <span className="flex min-w-0 items-center gap-1.5">
                        <span className="truncate text-sm font-medium">{dataset.name}</span>
                        {joined && (
                          <span className="shrink-0 rounded bg-accent/15 px-1 text-xs font-semibold text-accent">
                            joined
                          </span>
                        )}
                      </span>
                      <span className="truncate text-xs text-muted" title={joined ? joinText : undefined}>
                        {joined ? joinText : source?.type === "file" ? source.path.split(/[\/]/).pop() : dataset.source}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      </section>

      <JoinsSection
        report={report}
        schemas={schemas}
        onSave={onSaveJoined}
        onDelete={onDeleteJoined}
        onCancelEdit={onCancelJoined}
      />

      {active && (
        <>
          <FieldGroup title="Dimensions" hint="Example: region or month" columns={dimensions} onAddField={onAddField} />
          <FieldGroup title="Measures" hint="Example: revenue or units" columns={measures} onAddField={onAddField} />
          <p className="text-xs text-muted">
            Drag a field onto a shelf, or use the Add to menu. Numbers start as measures; change the kind on the shelf.
          </p>
          <CalculatedFieldsSection
            report={report}
            reportPath={reportPath}
            dataset={active}
            columns={columns}
            onSave={(field, original) => onSaveCalculated(active, field, original)}
            onDelete={(name) => onDeleteCalculated(active, name)}
          />
        </>
      )}
      <ParametersSection report={report} onSave={onSaveParameter} onDelete={onDeleteParameter} />
    </div>
  );
}

function FieldGroup({
  title,
  hint,
  columns,
  onAddField,
}: {
  title: string;
  hint: string;
  columns: ColumnView[];
  onAddField: (column: ColumnView, target: FieldTarget) => void;
}) {
  const [open, setOpen] = useState(true);
  const panelId = `fields-${title.toLowerCase()}`;
  return (
    <section aria-label={title} className="flex flex-col gap-1">
      <div className="flex items-center">
        <SectionToggle
          title={title}
          count={columns.length}
          open={open}
          controls={panelId}
          onToggle={() => setOpen((value) => !value)}
        />
      </div>
      <div id={panelId} hidden={!open}>
        {columns.length === 0 ? (
          <p className="px-2 text-xs text-muted">{hint}</p>
        ) : (
          <ul className="flex flex-col">
            {columns.map((column) => (
              <li key={column.name} className="flex items-center gap-2 rounded px-1 py-0.5 hover:bg-surface-2">
                <span
                  draggable
                  onDragStart={(event) => {
                    event.dataTransfer.setData(FIELD_MIME, JSON.stringify(column));
                    event.dataTransfer.effectAllowed = "copy";
                  }}
                  className="flex min-h-7 min-w-0 flex-1 cursor-grab items-center gap-2"
                  title={`${column.name} (${column.data_type})`}
                >
                  <TypeBadge family={column.family} role={column.role} />
                  <span className="truncate text-sm">{column.name}</span>
                  {column.calculated && (
                    <>
                      <span
                        aria-hidden="true"
                        title="Calculated field"
                        className="shrink-0 rounded bg-accent/15 px-1 font-mono text-xs font-semibold text-accent"
                      >
                        fx
                      </span>
                      <span className="sr-only">(calculated field)</span>
                    </>
                  )}
                </span>
                <select
                  aria-label={`Add ${column.name} to a shelf`}
                  title={`Add ${column.name} to a shelf`}
                  value=""
                  onChange={(event) => {
                    const target = event.target.value as FieldTarget | "";
                    if (target) onAddField(column, target);
                  }}
                  className="min-h-7 max-w-28 rounded border border-border bg-surface px-1 text-xs focus-visible:outline-2 focus-visible:outline-accent"
                >
                  <option value="">Add to...</option>
                  {SLOTS.map((slot) => (
                    <option key={slot.key} value={slot.key}>
                      {slot.label}
                    </option>
                  ))}
                  <option value="filter">Filters</option>
                </select>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}
