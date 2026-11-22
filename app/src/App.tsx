import { useEffect, useMemo, useState } from "react";
import { api, pickDataFiles, pickFolder, pickReportToOpen, pickReportToSave, toApiError } from "./api";
import { ChartView } from "./components/ChartView";
import { DataGrid } from "./components/DataGrid";
import { DataPanel, type FieldTarget } from "./components/DataPanel";
import { Shelves } from "./components/Shelves";
import { useDatasetTable, useRender } from "./hooks";
import {
  applySlots,
  autoTitle,
  blankVisual,
  emptySlots,
  hasFields,
  newVisualId,
  pillFor,
  slotsFromVisual,
  type Pill,
  type Slot,
} from "./model";
import type { ApiError, ColumnView, Report, Visual } from "./types";

const EXPORT_FORMATS = [
  { value: "html", label: "HTML page" },
  { value: "xlsx", label: "Excel workbook" },
  { value: "csv", label: "CSV per visual" },
  { value: "png", label: "PNG image" },
  { value: "pdf", label: "PDF document" },
];

type Notice = { tone: "error" | "info"; text: string };
type View = "builder" | "report";

function newReport(): Report {
  return { version: 1, name: "Untitled report", sources: [], datasets: [], visuals: [], parameters: [] };
}

const button =
  "rounded-md border border-border bg-surface px-3 py-1.5 text-sm font-medium hover:bg-surface-2 disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-accent";
const primary =
  "rounded-md bg-accent px-3 py-1.5 text-sm font-semibold text-accent-text hover:opacity-90 disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-accent";

export default function App() {
  const [report, setReport] = useState<Report>(newReport);
  const [reportPath, setReportPath] = useState<string | null>(null);
  const [reportDirty, setReportDirty] = useState(false);
  const [schemas, setSchemas] = useState<Record<string, ColumnView[]>>({});
  const [activeDataset, setActiveDataset] = useState<string | null>(null);
  const [draft, setDraft] = useState<Visual | null>(null);
  const [view, setView] = useState<View>("builder");
  const [bottomTab, setBottomTab] = useState<"preview" | "profile">("preview");
  const [notice, setNotice] = useState<Notice | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [exportFormat, setExportFormat] = useState("html");

  const columns = useMemo(() => (draft ? schemas[draft.dataset] ?? [] : []), [draft, schemas]);
  const slots = useMemo(() => (draft ? slotsFromVisual(draft) : emptySlots()), [draft]);
  const render = useRender(draft, 250);
  const table = useDatasetTable(activeDataset, bottomTab);

  const savedVisual = draft ? report.visuals.find((v) => v.id === draft.id) : undefined;
  const draftChanged = Boolean(draft) && (!savedVisual || JSON.stringify(savedVisual) !== JSON.stringify(draft));

  /** Runs an async action with a busy label and reports failures inline. */
  async function run(label: string, action: () => Promise<void>) {
    setBusy(label);
    setNotice(null);
    try {
      await action();
    } catch (err) {
      const e: ApiError = toApiError(err);
      if (e.kind !== "cancelled") setNotice({ tone: "error", text: e.message });
    } finally {
      setBusy(null);
    }
  }

  function selectDataset(name: string) {
    setActiveDataset(name);
    if (!draft || draft.dataset !== name) {
      setDraft(blankVisual(name, newVisualId(report)));
    }
  }

  function updateDraft(change: (visual: Visual) => Visual) {
    setDraft((current) => (current ? change(current) : current));
  }

  function setSlot(slot: Slot, pill: Pill | null) {
    updateDraft((visual) => applySlots(visual, { ...slotsFromVisual(visual), [slot]: pill }));
  }

  /** A field dropped on a shelf or picked from the Add to menu. */
  function addField(column: ColumnView, target: FieldTarget) {
    if (!draft) return;
    if (target === "filter") {
      updateDraft((visual) => ({
        ...visual,
        filters: [...visual.filters, { field: column.name, op: "is_not_null", values: [] }],
      }));
    } else {
      setSlot(target, pillFor(column));
    }
  }

  function saveVisual() {
    if (!draft) return;
    const visual: Visual = { ...draft, title: draft.title.trim() || autoTitle(draft) };
    setReport((current) => ({
      ...current,
      visuals: current.visuals.some((v) => v.id === visual.id)
        ? current.visuals.map((v) => (v.id === visual.id ? visual : v))
        : [...current.visuals, visual],
    }));
    setDraft(visual);
    setReportDirty(true);
    setNotice({ tone: "info", text: `Saved visual "${visual.title}" to the report.` });
  }

  function deleteVisual(id: string) {
    setReport((current) => ({ ...current, visuals: current.visuals.filter((v) => v.id !== id) }));
    setReportDirty(true);
    if (draft?.id === id) {
      setDraft(activeDataset ? blankVisual(activeDataset, newVisualId(report)) : null);
    }
  }

  function openVisual(visual: Visual) {
    setDraft(visual);
    setActiveDataset(visual.dataset);
    setView("builder");
  }

  async function addFiles() {
    const paths = await pickDataFiles();
    if (!paths || paths.length === 0) return;
    await run("Reading data", async () => {
      const existing = [...report.sources.map((s) => s.name), ...report.datasets.map((d) => d.name)];
      const added = await api.addDataFiles(existing, paths);
      setReport((current) => ({
        ...current,
        sources: [...current.sources, ...added.map((a) => a.source)],
        datasets: [...current.datasets, ...added.map((a) => a.dataset)],
      }));
      setSchemas((current) => {
        const next = { ...current };
        for (const a of added) next[a.dataset.name] = a.columns;
        return next;
      });
      setReportDirty(true);
      if (added.length > 0 && !activeDataset) selectDataset(added[0]!.dataset.name);
    });
  }

  async function openReport() {
    const path = await pickReportToOpen();
    if (!path) return;
    await run("Opening report", async () => {
      const opened = await api.openReport(path);
      setReport(opened.report);
      setReportPath(opened.path);
      setReportDirty(false);
      setSchemas(Object.fromEntries(opened.datasets.map((d) => [d.name, d.columns])));
      const first = opened.report.visuals[0];
      const dataset = first?.dataset ?? opened.report.datasets[0]?.name ?? null;
      setActiveDataset(dataset);
      setDraft(first ?? (dataset ? blankVisual(dataset, newVisualId(opened.report)) : null));
      setView("builder");
      setNotice({ tone: "info", text: `Opened ${opened.path}` });
    });
  }

  async function saveReport(asNew: boolean) {
    let path = reportPath;
    if (!path || asNew) {
      const chosen = await pickReportToSave(reportPath);
      if (!chosen) return;
      path = chosen.endsWith(".dvz") ? chosen : `${chosen}.dvz`;
    }
    const target = path;
    await run("Saving report", async () => {
      const stored = await api.saveReport(target, report);
      setReport(stored);
      setReportPath(target);
      setReportDirty(false);
      setNotice({ tone: "info", text: `Saved ${target}. Unsaved visual edits are not part of the report.` });
    });
  }

  async function exportReport() {
    const dir = await pickFolder();
    if (!dir) return;
    await run("Exporting", async () => {
      const summary = await api.exportReport(report, exportFormat, dir);
      const warnings = summary.warnings.length ? ` Skipped: ${summary.warnings.join("; ")}` : "";
      setNotice({
        tone: "info",
        text: `Wrote ${summary.files.length} file(s) to ${dir}.${warnings}`,
      });
    });
  }

  // Keep the window title in step with the report.
  useEffect(() => {
    document.title = `${reportDirty ? "* " : ""}${report.name} - Davilize`;
  }, [report.name, reportDirty]);

  return (
    <div className="flex h-full min-h-0 flex-col bg-bg text-text">
      <header className="flex flex-wrap items-center gap-2 border-b border-border bg-surface px-4 py-2">
        <h1 className="mr-2 text-base font-semibold">Davilize</h1>
        <label className="sr-only" htmlFor="report-name">
          Report name
        </label>
        <input
          id="report-name"
          value={report.name}
          onChange={(e) => {
            setReport((current) => ({ ...current, name: e.target.value }));
            setReportDirty(true);
          }}
          className="w-56 rounded-md border border-transparent bg-transparent px-2 py-1 text-sm hover:border-border focus-visible:border-accent focus-visible:outline-none"
        />
        {reportDirty && <span className="text-xs text-muted">unsaved</span>}
        <div className="ml-auto flex flex-wrap items-center gap-2">
          <button type="button" className={button} onClick={openReport} disabled={busy !== null}>
            Open report
          </button>
          <button type="button" className={button} onClick={() => saveReport(false)} disabled={busy !== null}>
            Save
          </button>
          <button type="button" className={button} onClick={() => saveReport(true)} disabled={busy !== null}>
            Save as
          </button>
          <span className="mx-1 h-6 w-px bg-border" aria-hidden="true" />
          <label className="sr-only" htmlFor="export-format">
            Export format
          </label>
          <select
            id="export-format"
            value={exportFormat}
            onChange={(e) => setExportFormat(e.target.value)}
            className="rounded-md border border-border bg-surface px-2 py-1.5 text-sm"
          >
            {EXPORT_FORMATS.map((f) => (
              <option key={f.value} value={f.value}>
                {f.label}
              </option>
            ))}
          </select>
          <button type="button" className={primary} onClick={exportReport} disabled={busy !== null || report.visuals.length === 0}>
            Export...
          </button>
        </div>
      </header>

      {(busy || notice) && (
        <div className="flex items-center gap-3 border-b border-border px-4 py-2 text-sm">
          {busy && (
            <>
              <span className="spinner" aria-hidden="true" />
              <span role="status">{busy}...</span>
            </>
          )}
          {notice && (
            <p role={notice.tone === "error" ? "alert" : "status"} className={`min-w-0 flex-1 truncate ${notice.tone === "error" ? "text-danger" : "text-muted"}`} title={notice.text}>
              {notice.text}
            </p>
          )}
          {notice && (
            <button type="button" className="text-muted hover:text-text focus-visible:outline-2 focus-visible:outline-accent" onClick={() => setNotice(null)} aria-label="Dismiss message">
              Dismiss
            </button>
          )}
        </div>
      )}

      <div className="grid min-h-0 flex-1 grid-cols-1 md:grid-cols-[18rem_1fr]">
        <aside aria-label="Data panel" className="min-h-0 overflow-hidden border-b border-border bg-surface md:border-b-0 md:border-r">
          <DataPanel
            datasets={report.datasets}
            sources={report.sources}
            schemas={schemas}
            active={activeDataset}
            busy={busy !== null}
            onSelect={selectDataset}
            onAddFiles={addFiles}
            onAddField={addField}
          />
        </aside>

        <main className="flex min-h-0 min-w-0 flex-col">
          <div role="tablist" aria-label="Views" className="flex gap-1 border-b border-border px-3 pt-2">
            {(["builder", "report"] as const).map((name) => (
              <button
                key={name}
                role="tab"
                type="button"
                aria-selected={view === name}
                onClick={() => setView(name)}
                className={`rounded-t-md px-3 py-1.5 text-sm font-medium focus-visible:outline-2 focus-visible:outline-accent ${
                  view === name ? "bg-bg text-text ring-1 ring-border" : "text-muted hover:text-text"
                }`}
              >
                {name === "builder" ? "Chart builder" : `Report (${report.visuals.length})`}
              </button>
            ))}
          </div>

          {view === "builder" ? (
            <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-3">
              {!draft ? (
                <EmptyState hasData={report.datasets.length > 0} onAddFiles={addFiles} />
              ) : (
                <>
                  <div className="flex flex-wrap items-end gap-2">
                    <div className="min-w-0 flex-1">
                      <label htmlFor="visual-title" className="text-xs font-semibold uppercase tracking-wide text-muted">
                        Visual title
                      </label>
                      <input
                        id="visual-title"
                        value={draft.title}
                        placeholder={autoTitle(draft)}
                        onChange={(e) => updateDraft((v) => ({ ...v, title: e.target.value }))}
                        className="mt-1 w-full rounded-md border border-border bg-surface px-2 py-1.5 text-sm"
                      />
                    </div>
                    <button type="button" className={primary} onClick={saveVisual} disabled={!hasFields(draft)}>
                      {savedVisual ? (draftChanged ? "Update visual" : "Saved") : "Add to report"}
                    </button>
                    {savedVisual && (
                      <button type="button" className={button} onClick={() => deleteVisual(draft.id)}>
                        Delete visual
                      </button>
                    )}
                    <button
                      type="button"
                      className={button}
                      onClick={() => setDraft(blankVisual(draft.dataset, newVisualId(report)))}
                    >
                      New visual
                    </button>
                  </div>

                  {report.visuals.length > 0 && (
                    <div role="tablist" aria-label="Visuals in report" className="flex flex-wrap gap-1">
                      {report.visuals.map((v) => (
                        <button
                          key={v.id}
                          role="tab"
                          type="button"
                          aria-selected={v.id === draft.id}
                          onClick={() => openVisual(v)}
                          className={`rounded-md border px-2 py-1 text-xs focus-visible:outline-2 focus-visible:outline-accent ${
                            v.id === draft.id ? "border-accent bg-accent/15" : "border-border bg-surface hover:bg-surface-2"
                          }`}
                        >
                          {v.title || v.id}
                        </button>
                      ))}
                    </div>
                  )}

                  <Shelves
                    visual={draft}
                    slots={slots}
                    columns={columns}
                    onSlot={setSlot}
                    onFieldDrop={addField}
                    onChange={(patch) => updateDraft((v) => ({ ...v, ...patch }))}
                  />

                  <section aria-label="Chart" className="flex min-h-64 flex-col gap-2 rounded-md border border-border bg-surface p-3">
                    <div className="flex items-center gap-2 text-sm text-muted">
                      <span className="font-medium text-text">{draft.title || autoTitle(draft)}</span>
                      {render.loading && (
                        <>
                          <span className="spinner" aria-hidden="true" />
                          <span role="status">Running query</span>
                          <button type="button" className="rounded px-2 py-0.5 underline hover:text-text focus-visible:outline-2 focus-visible:outline-accent" onClick={() => api.cancelQuery().catch(() => undefined)}>
                            Cancel
                          </button>
                        </>
                      )}
                    </div>
                    {render.error && render.error.kind !== "cancelled" && (
                      <div role="alert" className="rounded-md bg-danger-bg px-3 py-2 text-sm text-danger">
                        <p className="font-medium">Query error ({render.error.kind})</p>
                        <p className="break-words font-mono text-xs">{render.error.message}</p>
                      </div>
                    )}
                    {render.error?.kind === "cancelled" && <p className="text-sm text-muted">The query was cancelled.</p>}
                    {!render.result && !render.error && !render.loading && (
                      <p className="text-sm text-muted">Drop fields on the shelves to draw the chart.</p>
                    )}
                    {render.result && (
                      <>
                        <ChartView spec={render.result.spec} label={render.result.title} />
                        {render.result.notes.map((note) => (
                          <p key={note} className="text-xs text-muted">
                            {note}
                          </p>
                        ))}
                        <details className="text-xs text-muted">
                          <summary className="cursor-pointer hover:text-text">Query ({render.result.row_count} rows)</summary>
                          <pre className="mt-1 overflow-x-auto whitespace-pre-wrap rounded bg-surface-2 p-2 font-mono">{render.result.sql}</pre>
                        </details>
                      </>
                    )}
                  </section>

                  <section aria-label="Data" className="flex h-72 min-h-0 flex-col gap-2">
                    <div role="tablist" aria-label="Data views" className="flex gap-1">
                      {(["preview", "profile"] as const).map((mode) => (
                        <button
                          key={mode}
                          role="tab"
                          type="button"
                          aria-selected={bottomTab === mode}
                          onClick={() => setBottomTab(mode)}
                          className={`rounded-md px-2.5 py-1 text-xs font-medium focus-visible:outline-2 focus-visible:outline-accent ${
                            bottomTab === mode ? "bg-accent/15" : "text-muted hover:text-text"
                          }`}
                        >
                          {mode === "preview" ? "Preview (first 500 rows)" : "Column profile"}
                        </button>
                      ))}
                    </div>
                    <div className="min-h-0 flex-1">
                      {table.loading && (
                        <p className="flex items-center gap-2 text-sm text-muted" role="status">
                          <span className="spinner" aria-hidden="true" /> Loading
                        </p>
                      )}
                      {table.error && (
                        <p role="alert" className="text-sm text-danger">
                          {table.error.message}
                        </p>
                      )}
                      {table.table && (
                        <DataGrid table={table.table} label={bottomTab === "preview" ? "Data preview" : "Column profile"} />
                      )}
                    </div>
                  </section>
                </>
              )}
            </div>
          ) : (
            <ReportView visuals={report.visuals} onOpen={openVisual} />
          )}
        </main>
      </div>
    </div>
  );
}

function EmptyState({ hasData, onAddFiles }: { hasData: boolean; onAddFiles: () => void }) {
  return (
    <div className="m-auto flex max-w-md flex-col items-center gap-3 p-6 text-center">
      <h2 className="text-lg font-semibold">{hasData ? "Pick a dataset" : "Start with some data"}</h2>
      <p className="text-sm text-muted">
        {hasData
          ? "Select a dataset in the data panel to build a chart from its fields."
          : "Add a CSV, Parquet, JSON or Arrow file. The data stays on this machine."}
      </p>
      {!hasData && (
        <button type="button" className={primary} onClick={onAddFiles}>
          Add data files
        </button>
      )}
    </div>
  );
}

function ReportView({ visuals, onOpen }: { visuals: Visual[]; onOpen: (visual: Visual) => void }) {
  if (visuals.length === 0) {
    return <p className="p-6 text-sm text-muted">The report has no visuals yet. Build one in the chart builder and add it to the report.</p>;
  }
  return (
    <div className="grid grid-cols-1 gap-3 overflow-y-auto p-3 xl:grid-cols-2">
      {visuals.map((visual) => (
        <VisualCard key={visual.id} visual={visual} onOpen={() => onOpen(visual)} />
      ))}
    </div>
  );
}

function VisualCard({ visual, onOpen }: { visual: Visual; onOpen: () => void }) {
  const state = useRender(visual, 0);
  return (
    <article aria-label={visual.title || visual.id} className="flex min-w-0 flex-col gap-2 rounded-md border border-border bg-surface p-3">
      <div className="flex items-center justify-between gap-2">
        <h2 className="truncate text-sm font-semibold">{visual.title || visual.id}</h2>
        <button type="button" onClick={onOpen} className="text-xs text-accent underline focus-visible:outline-2 focus-visible:outline-accent">
          Edit
        </button>
      </div>
      {state.loading && (
        <p className="flex items-center gap-2 text-sm text-muted" role="status">
          <span className="spinner" aria-hidden="true" /> Running query
        </p>
      )}
      {state.error && (
        <p role="alert" className="break-words text-sm text-danger">
          {state.error.message}
        </p>
      )}
      {state.result && <ChartView spec={state.result.spec} label={state.result.title} />}
    </article>
  );
}

