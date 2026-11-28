import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent } from "react";
import {
  api,
  pickDataFiles,
  pickFileToImport,
  pickProjectFolderToImport,
  pickFolder,
  pickReportToOpen,
  pickReportToSave,
  toApiError,
} from "./api";
import { ChartView, ShowTableToggle, isTableSpec } from "./components/ChartView";
import { DashboardsPanel } from "./components/Dashboards";
import { DataGrid } from "./components/DataGrid";
import { DataPanel, type FieldTarget } from "./components/DataPanel";
import { HelpDialog } from "./components/HelpDialog";
import { Welcome } from "./components/Welcome";
import { ParameterValuesBar } from "./components/Parameters";
import { Shelves } from "./components/Shelves";
import { canRedo, canUndo, createHistory, push, redo, redoLabel, replace, undo, undoLabel, type History } from "./history";
import { useDatasetTable, useRender } from "./hooks";
import {
  addJoinedDataset,
  applySlots,
  autoTitle,
  blankDashboard,
  blankVisual,
  dashboardsWithVisual,
  datasetDependents,
  datasetOf,
  datasetsToRefresh,
  emptySlots,
  hasFields,
  newVisualId,
  paramText,
  parameterIssues,
  pillFor,
  removeDataset,
  replaceDashboard,
  replaceDataset,
  requestValues,
  slotsFromVisual,
  withCalculatedField,
  withoutDashboard,
  withoutVisualPlacements,
  withoutCalculatedField,
  withoutParameter,
  withParameter,
  type Pill,
  type Slot,
} from "./model";
import { isWeb, WEB_EXPORT_FORMATS } from "./web/platform";
import { onEngineStarting } from "./web/status";
import type { ApiError, CalculatedField, ColumnView, Dashboard, Dataset, Parameter, Report, Visual } from "./types";

const ALL_EXPORT_FORMATS = [
  { value: "html", label: "HTML page" },
  { value: "xlsx", label: "Excel workbook" },
  { value: "csv", label: "CSV per chart" },
  { value: "png", label: "PNG image" },
  { value: "pdf", label: "PDF document" },
  { value: "twb", label: "Tableau workbook (.twb)" },
  { value: "twbx", label: "Tableau packaged (.twbx)" },
  { value: "pbit", label: "Power BI template (.pbit)" },
  { value: "pbip", label: "Power BI project folder" },
  { value: "pbix", label: "Power BI file (.pbix, data snapshot)" },
];

const EXPORT_FORMATS = isWeb
  ? ALL_EXPORT_FORMATS.filter((f) => WEB_EXPORT_FORMATS.includes(f.value))
  : ALL_EXPORT_FORMATS;

type ToastTone = "info" | "success" | "error";
interface Toast {
  id: number;
  tone: ToastTone;
  text: string;
}
type View = "builder" | "report";
const VIEWS: readonly View[] = ["builder", "report"];

const TOAST_MS = 6000;
const TOAST_LIMIT = 4;
const PANEL_MIN = 220;
const PANEL_MAX = 480;
const PANEL_DEFAULT = 300;
const PANEL_STEP = 4;
const LAYOUT_KEY = "davilize.layout";

interface LayoutPrefs {
  panelWidth: number;
  panelCollapsed: boolean;
}
const DEFAULT_LAYOUT: LayoutPrefs = { panelWidth: PANEL_DEFAULT, panelCollapsed: false };

function clampPanel(width: number): number {
  return Math.min(PANEL_MAX, Math.max(PANEL_MIN, Math.round(width)));
}

/** Reads the saved data panel layout. Storage may be unavailable or hold junk, so failures fall back to the defaults. */
function loadLayout(): LayoutPrefs {
  try {
    const raw = window.localStorage.getItem(LAYOUT_KEY);
    if (!raw) return firstLayout();
    const saved = JSON.parse(raw) as Partial<LayoutPrefs> | null;
    return {
      panelWidth:
        typeof saved?.panelWidth === "number" && Number.isFinite(saved.panelWidth) ? clampPanel(saved.panelWidth) : PANEL_DEFAULT,
      panelCollapsed: saved?.panelCollapsed === true,
    };
  } catch {
    return firstLayout();
  }
}

/** Layout before the user has chosen one: on a narrow window the data panel starts hidden, so the chart gets the room. */
function firstLayout(): LayoutPrefs {
  return { ...DEFAULT_LAYOUT, panelCollapsed: window.innerWidth < 900 };
}

function saveLayout(prefs: LayoutPrefs) {
  try {
    window.localStorage.setItem(LAYOUT_KEY, JSON.stringify(prefs));
  } catch {
    // Storage may be unavailable. The layout is then simply not remembered.
  }
}

/** Adds a toast. When the stack is full the oldest non-error toast leaves first, so errors stay until dismissed. */
function addToast(list: Toast[], next: Toast): Toast[] {
  const out = [...list, next];
  while (out.length > TOAST_LIMIT) {
    const index = out.findIndex((t) => t.tone !== "error");
    if (index === -1) break;
    out.splice(index, 1);
  }
  return out;
}

function newReport(): Report {
  return { version: 1, name: "Untitled report", sources: [], datasets: [], visuals: [], parameters: [] };
}

const button =
  "rounded-md border border-border bg-surface px-3 py-1.5 text-sm font-medium hover:bg-surface-2 disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-accent";
const primary =
  "rounded-md bg-accent px-3 py-1.5 text-sm font-semibold text-accent-text hover:opacity-90 disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-accent";

export default function App() {
  const [hist, setHist] = useState<History<Report>>(() => createHistory(newReport()));
  const report = hist.present;
  const [reportPath, setReportPath] = useState<string | null>(null);
  const [reportDirty, setReportDirty] = useState(false);
  const [schemas, setSchemas] = useState<Record<string, ColumnView[]>>({});
  const [activeDataset, setActiveDataset] = useState<string | null>(null);
  const [draft, setDraft] = useState<Visual | null>(null);
  const [view, setView] = useState<View>("builder");
  const [bottomTab, setBottomTab] = useState<"preview" | "profile">("preview");
  // The builder's chart shows as a table instead of a chart.
  const [builderTable, setBuilderTable] = useState(false);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [helpOpen, setHelpOpen] = useState(false);
  const nextToastId = useRef(1);
  const [busy, setBusy] = useState<string | null>(null);
  const [engineStarting, setEngineStartingState] = useState(false);
  useEffect(() => onEngineStarting(setEngineStartingState), []);
  const [exportFormat, setExportFormat] = useState("html");
  // Runtime values for the report's parameters. They are not saved in the .dvz.
  const [paramValues, setParamValues] = useState<Record<string, string>>({});
  // The dashboard shown in the Report tab, or null for "All charts".
  const [dashboardId, setDashboardId] = useState<string | null>(null);
  const [layout, setLayout] = useState<LayoutPrefs>(loadLayout);
  const tabRefs = useRef<Record<View, HTMLButtonElement | null>>({ builder: null, report: null });
  const dragStart = useRef<{ x: number; width: number } | null>(null);
  const shortcutRef = useRef<(e: KeyboardEvent) => void>(() => undefined);

  const columns = useMemo(() => (draft ? schemas[draft.dataset] ?? [] : []), [draft, schemas]);
  const slots = useMemo(() => (draft ? slotsFromVisual(draft) : emptySlots()), [draft]);
  const paramIssues = useMemo(() => parameterIssues(report.parameters, paramValues), [report.parameters, paramValues]);
  const paramsBlocked = Object.keys(paramIssues).length > 0;
  const render = useRender(draft, 250, report.parameters, paramValues, paramsBlocked);
  const table = useDatasetTable(activeDataset, bottomTab);

  const savedVisual = draft ? report.visuals.find((v) => v.id === draft.id) : undefined;
  const draftChanged = Boolean(draft) && (!savedVisual || JSON.stringify(savedVisual) !== JSON.stringify(draft));

  useEffect(() => {
    saveLayout(layout);
  }, [layout]);

  const dismissToast = useCallback((id: number) => setToasts((list) => list.filter((t) => t.id !== id)), []);

  function notify(text: string, tone: ToastTone = "info") {
    const id = nextToastId.current++;
    setToasts((list) => addToast(list, { id, tone, text }));
  }

  /** Runs an async action with a busy label and reports failures in a toast. */
  async function run(label: string, action: () => Promise<void>) {
    setBusy(label);
    try {
      await action();
    } catch (err) {
      const e: ApiError = toApiError(err);
      if (e.kind !== "cancelled") notify(e.message, "error");
    } finally {
      setBusy(null);
    }
  }

  /**
   * Applies a change to the report and records it for undo. Changes that share a key
   * within the coalescing window (for example keystrokes in the report name) form one step.
   */
  function changeReport(label: string, update: (current: Report) => Report, key: string | null = null) {
    const now = Date.now();
    setHist((h) => push(h, update(h.present), label, key, now));
    setReportDirty(true);
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
    changeReport("Save chart", (current) => ({
      ...current,
      visuals: current.visuals.some((v) => v.id === visual.id)
        ? current.visuals.map((v) => (v.id === visual.id ? visual : v))
        : [...current.visuals, visual],
    }));
    setDraft(visual);
    notify(`Saved chart "${visual.title}" to the report.`, "success");
  }

  function deleteVisual(id: string) {
    const placedOn = dashboardsWithVisual(report, id).map((d) => d.title || d.id);
    const title = report.visuals.find((v) => v.id === id)?.title || id;
    changeReport(`Delete chart "${title}"`, (current) =>
      withoutVisualPlacements({ ...current, visuals: current.visuals.filter((v) => v.id !== id) }, id),
    );
    if (placedOn.length > 0) {
      notify(`Deleted chart "${title}". It was also removed from ${placedOn.join(", ")}.`);
    }
    if (draft?.id === id) {
      setDraft(activeDataset ? blankVisual(activeDataset, newVisualId(report)) : null);
    }
  }

  function createDashboard() {
    const dashboard = blankDashboard(report);
    changeReport("Add dashboard", (current) => replaceDashboard(current, dashboard));
    setDashboardId(dashboard.id);
  }

  function saveDashboard(dashboard: Dashboard) {
    changeReport("Edit dashboard", (current) => replaceDashboard(current, dashboard), `dashboard:${dashboard.id}`);
  }

  function deleteDashboard(id: string) {
    const title = report.dashboards?.find((d) => d.id === id)?.title || id;
    changeReport(`Delete dashboard "${title}"`, (current) => withoutDashboard(current, id));
    if (dashboardId === id) setDashboardId(null);
  }

  function openVisual(visual: Visual) {
    setDraft(visual);
    setActiveDataset(visual.dataset);
    setView("builder");
  }

  /**
   * Re-registers the edited dataset in the engine, then applies the change to the
   * report. A refused change (for example a bad expression) leaves everything as it was.
   */
  async function applyCalculated(dataset: string, candidate: Report, label: string): Promise<string | null> {
    if (!datasetOf(candidate, dataset)) return `Unknown dataset "${dataset}".`;
    return applyDatasets(report, candidate, label);
  }

  /**
   * Registers the datasets that change between `current` and `candidate`, and those built on them, then
   * records `candidate` for undo. When the engine refuses one, the changed datasets are registered back
   * to their current definitions and nothing is recorded.
   */
  async function applyDatasets(current: Report, candidate: Report, label: string): Promise<string | null> {
    const failure = await syncDatasets(current, candidate);
    if (failure) {
      await syncDatasets(candidate, current);
      return failure;
    }
    changeReport(label, () => candidate);
    return null;
  }

  /** Saves a joined dataset (a new one, or the one named `original`) and selects it. */
  async function saveJoinedDataset(dataset: Dataset, original: string | null): Promise<string | null> {
    const candidate = original === null ? addJoinedDataset(report, dataset) : replaceDataset(report, original, dataset);
    const label = original === null ? `Add joined dataset "${dataset.name}"` : `Edit joined dataset "${dataset.name}"`;
    const failure = await applyDatasets(report, candidate, label);
    if (failure === null) selectDataset(dataset.name);
    return failure;
  }

  async function deleteJoinedDataset(name: string): Promise<string | null> {
    const used = datasetDependents(report, name);
    if (used.length > 0) return `"${name}" is used by ${used.join(", ")}. Remove it from those first.`;
    const wasActive = activeDataset === name;
    const other = report.datasets.find((d) => d.name !== name)?.name;
    changeReport(`Delete joined dataset "${name}"`, (current) => removeDataset(current, name));
    setSchemas((current) => {
      const next = { ...current };
      delete next[name];
      return next;
    });
    if (wasActive) {
      if (other) selectDataset(other);
      else {
        setActiveDataset(null);
        setDraft(null);
      }
    }
    return null;
  }

  /** The joined dataset's form was closed without saving. The engine may hold the version checked in the form, so the saved one is registered again. */
  function cancelJoinedEdit(name: string) {
    const saved = report.datasets.find((d) => d.name === name);
    if (!saved) return;
    void api
      .registerDataset(saved)
      .then((columns) => setSchemas((current) => ({ ...current, [name]: columns })))
      .catch(() => undefined);
  }

  function saveCalculatedField(dataset: string, field: CalculatedField, original: string | null) {
    return applyCalculated(dataset, withCalculatedField(report, dataset, field, original), "Save calculated field");
  }

  function deleteCalculatedField(dataset: string, name: string) {
    return applyCalculated(dataset, withoutCalculatedField(report, dataset, name), "Delete calculated field");
  }

  function saveParameter(param: Parameter, original: string | null) {
    changeReport(`Save parameter "${param.name}"`, (current) => withParameter(current, param, original));
    setParamValues((current) => {
      const next = { ...current };
      if (original !== null) delete next[original];
      return next;
    });
  }

  function deleteParameter(name: string) {
    changeReport(`Delete parameter "${name}"`, (current) => withoutParameter(current, name));
    setParamValues((current) => {
      const next = { ...current };
      delete next[name];
      return next;
    });
  }

  /**
   * Brings the engine and the schemas in line with `after`: datasets that were added or whose definition
   * changed (calculated fields, joins, source) are registered again, in dependency order, along with the
   * datasets built on them. Removed datasets lose their schema. Returns the first failure, or null.
   */
  async function syncDatasets(before: Report, after: Report): Promise<string | null> {
    const removed = before.datasets.map((d) => d.name).filter((name) => !after.datasets.some((d) => d.name === name));
    if (removed.length > 0) {
      setSchemas((current) => {
        const next = { ...current };
        for (const name of removed) delete next[name];
        return next;
      });
    }
    let failure: string | null = null;
    const failed = new Set<string>();
    for (const dataset of datasetsToRefresh(before, after)) {
      const inputs = [dataset.source, ...(dataset.joins ?? []).map((j) => j.with)];
      if (inputs.some((input) => failed.has(input))) {
        failed.add(dataset.name);
        continue;
      }
      try {
        const refreshed = await api.registerDataset(dataset);
        setSchemas((current) => ({ ...current, [dataset.name]: refreshed }));
      } catch (err) {
        failed.add(dataset.name);
        failure ??= toApiError(err).message;
      }
    }
    return failure;
  }

  function stepHistory(direction: "undo" | "redo") {
    const label = direction === "undo" ? undoLabel(hist) : redoLabel(hist);
    if (label === null) return;
    const before = hist.present;
    const next = direction === "undo" ? undo(hist) : redo(hist);
    setHist(next);
    setReportDirty(true);
    notify(`${direction === "undo" ? "Undid" : "Redid"}: ${label}`);
    if (dashboardId !== null && !next.present.dashboards?.some((d) => d.id === dashboardId)) {
      setDashboardId(null);
    }
    void syncDatasets(before, next.present).then((failure) => {
      if (failure) notify(failure, "error");
    });
  }

  function togglePanel() {
    setLayout((current) => ({ ...current, panelCollapsed: !current.panelCollapsed }));
  }

  /** Keyboard shortcuts: Ctrl+B toggles the data panel; Ctrl+Z and Ctrl+Shift+Z / Ctrl+Y undo and redo. */
  function onShortcut(e: KeyboardEvent) {
    if (e.key === "?" && !e.ctrlKey && !e.metaKey && !e.altKey) {
      const t = e.target instanceof HTMLElement ? e.target : null;
      if (!(t && (t.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(t.tagName)))) {
        e.preventDefault();
        setHelpOpen(true);
        return;
      }
    }
    if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
    const key = e.key.toLowerCase();
    if (key === "b" && !e.shiftKey) {
      e.preventDefault();
      togglePanel();
      return;
    }
    // Text fields keep their own undo.
    const target = e.target instanceof HTMLElement ? e.target : null;
    if (target && (target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName))) return;
    if (key === "z" && !e.shiftKey) {
      e.preventDefault();
      stepHistory("undo");
    } else if ((key === "z" && e.shiftKey) || (key === "y" && !e.shiftKey)) {
      e.preventDefault();
      stepHistory("redo");
    }
  }
  shortcutRef.current = onShortcut;

  useEffect(() => {
    const listener = (e: KeyboardEvent) => shortcutRef.current(e);
    window.addEventListener("keydown", listener);
    return () => window.removeEventListener("keydown", listener);
  }, []);

  function onSeparatorPointerDown(e: ReactPointerEvent<HTMLDivElement>) {
    if (e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    dragStart.current = { x: e.clientX, width: layout.panelWidth };
  }

  function onSeparatorPointerMove(e: ReactPointerEvent<HTMLDivElement>) {
    const start = dragStart.current;
    if (!start) return;
    const width = clampPanel(start.width + e.clientX - start.x);
    setLayout((current) => (current.panelWidth === width ? current : { ...current, panelWidth: width }));
  }

  function onSeparatorPointerUp(e: ReactPointerEvent<HTMLDivElement>) {
    dragStart.current = null;
    if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
  }

  function onSeparatorKey(e: ReactKeyboardEvent<HTMLDivElement>) {
    let width: number;
    if (e.key === "ArrowLeft") width = layout.panelWidth - PANEL_STEP;
    else if (e.key === "ArrowRight") width = layout.panelWidth + PANEL_STEP;
    else if (e.key === "Home") width = PANEL_MIN;
    else if (e.key === "End") width = PANEL_MAX;
    else return;
    e.preventDefault();
    const next = clampPanel(width);
    setLayout((current) => ({ ...current, panelWidth: next }));
  }

  function onTabKey(e: ReactKeyboardEvent<HTMLButtonElement>, current: View) {
    const index = VIEWS.indexOf(current);
    let next: View | undefined;
    if (e.key === "ArrowRight") next = VIEWS[(index + 1) % VIEWS.length];
    else if (e.key === "ArrowLeft") next = VIEWS[(index - 1 + VIEWS.length) % VIEWS.length];
    else if (e.key === "Home") next = VIEWS[0];
    else if (e.key === "End") next = VIEWS[VIEWS.length - 1];
    if (next === undefined) return;
    e.preventDefault();
    setView(next);
    tabRefs.current[next]?.focus();
  }

  async function addFiles() {
    const paths = await pickDataFiles();
    if (!paths || paths.length === 0) return;
    await run("Reading data", async () => {
      const existing = [...report.sources.map((s) => s.name), ...report.datasets.map((d) => d.name)];
      const added = await api.addDataFiles(existing, paths);
      changeReport("Add data", (current) => ({
        ...current,
        sources: [...current.sources, ...added.map((a) => a.source)],
        datasets: [...current.datasets, ...added.map((a) => a.dataset)],
      }));
      setSchemas((current) => {
        const next = { ...current };
        for (const a of added) next[a.dataset.name] = a.columns;
        return next;
      });
      if (added.length > 0 && !activeDataset) selectDataset(added[0]!.dataset.name);
    });
  }

  async function openReport() {
    const path = await pickReportToOpen();
    if (!path) return;
    await openReportAt(path);
  }

  async function openSample() {
    try {
      await openReportAt(await api.createSampleReport());
    } catch (e) {
      notify(toApiError(e).message, "error");
    }
  }

  async function importReport(project = false) {
    const path = project ? await pickProjectFolderToImport() : await pickFileToImport();
    if (!path) return;
    await run("Importing", async () => {
      const imported = await api.importReport(path);
      const opened = imported.opened;
      setHist(createHistory(opened.report));
      setReportPath(opened.path);
      setReportDirty(false);
      setParamValues({});
      setDashboardId(null);
      setSchemas(Object.fromEntries(opened.datasets.map((d) => [d.name, d.columns])));
      const first = opened.report.visuals[0];
      const dataset = first?.dataset ?? opened.report.datasets[0]?.name ?? null;
      setActiveDataset(dataset);
      setDraft(first ?? (dataset ? blankVisual(dataset, newVisualId(opened.report)) : null));
      setView("builder");
      const skipped = imported.warnings.length;
      const detail = skipped
        ? ` ${skipped} item(s) could not be converted: ${imported.warnings.slice(0, 3).join("; ")}${skipped > 3 ? "; ..." : ""}`
        : "";
      notify(`Imported to ${opened.path}.${detail}`, skipped ? "info" : "success");
    });
  }

  async function openReportAt(path: string) {
    await run("Opening report", async () => {
      const opened = await api.openReport(path);
      setHist(createHistory(opened.report));
      setReportPath(opened.path);
      setReportDirty(false);
      setParamValues({});
      setDashboardId(null);
      setSchemas(Object.fromEntries(opened.datasets.map((d) => [d.name, d.columns])));
      const first = opened.report.visuals[0];
      const dataset = first?.dataset ?? opened.report.datasets[0]?.name ?? null;
      setActiveDataset(dataset);
      setDraft(first ?? (dataset ? blankVisual(dataset, newVisualId(opened.report)) : null));
      setView("builder");
      notify(`Opened ${opened.path}`);
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
      setHist((h) => replace(h, stored));
      setReportPath(target);
      setReportDirty(false);
      notify(`Saved ${target}. Unsaved chart edits are not part of the report.`, "success");
    });
  }

  async function exportReport() {
    const dir = await pickFolder();
    if (!dir) return;
    await run("Exporting", async () => {
      const summary = await api.exportReport(report, exportFormat, dir, requestValues(report.parameters, paramValues));
      const warnings = summary.warnings.length ? ` Skipped: ${summary.warnings.join("; ")}` : "";
      notify(`Wrote ${summary.files.length} file(s) to ${dir}.${warnings}`, summary.warnings.length ? "info" : "success");
    });
  }

  // Keep the window title in step with the report.
  useEffect(() => {
    document.title = `${reportDirty ? "* " : ""}${report.name} - Davilize`;
  }, [report.name, reportDirty]);

  const panelCollapsed = layout.panelCollapsed;

  return (
    <div className="flex h-full min-h-0 flex-col bg-bg text-text">
      <a
        href="#main-content"
        className="sr-only focus:not-sr-only focus:fixed focus:left-2 focus:top-2 focus:z-50 focus:rounded-md focus:bg-surface focus:px-3 focus:py-2 focus:text-sm focus:font-medium focus:outline-2 focus:outline-accent"
      >
        Skip to main content
      </a>

      <header className="flex flex-wrap items-center gap-x-2 gap-y-1 border-b border-border bg-surface px-4 py-2">
        <h1 className="mr-1 text-base font-semibold">Davilize</h1>
        <label className="sr-only" htmlFor="report-name">
          Report name
        </label>
        <input
          id="report-name"
          value={report.name}
          onChange={(e) => {
            const name = e.target.value;
            changeReport("Rename report", (current) => ({ ...current, name }), "report-name");
          }}
          className="w-44 rounded-md border border-transparent bg-transparent px-2 py-1 text-sm hover:border-border focus-visible:border-accent focus-visible:outline-none"
        />
        {reportDirty && <span className="text-xs text-muted">unsaved</span>}
        {busy && (
          <span role="status" className="flex items-center gap-2 text-xs text-muted">
            <span className="spinner" aria-hidden="true" />
            {busy}...
            {engineStarting && " Loading the data engine (about 34 MB on the first visit only)"}
          </span>
        )}
        {report.parameters.length > 0 && (
          <ParametersMenu
            parameters={report.parameters}
            values={paramValues}
            issues={paramIssues}
            onChange={(name, text) => setParamValues((current) => ({ ...current, [name]: text }))}
            onReset={() => setParamValues({})}
          />
        )}
        <div className="ml-auto flex flex-wrap items-center gap-2">
          <button
            type="button"
            className={button}
            onClick={() => stepHistory("undo")}
            disabled={!canUndo(hist)}
            title={`Undo${undoLabel(hist) ? `: ${undoLabel(hist)}` : ""} (Ctrl+Z)`}
            aria-keyshortcuts="Control+Z"
          >
            Undo
          </button>
          <button
            type="button"
            className={button}
            onClick={() => stepHistory("redo")}
            disabled={!canRedo(hist)}
            title={`Redo${redoLabel(hist) ? `: ${redoLabel(hist)}` : ""} (Ctrl+Shift+Z or Ctrl+Y)`}
            aria-keyshortcuts="Control+Shift+Z Control+Y"
          >
            Redo
          </button>
          <span className="mx-1 h-6 w-px bg-border" aria-hidden="true" />
          <button
            type="button"
            className={button}
            onClick={() => setHelpOpen(true)}
            aria-label="Help and keyboard shortcuts"
            title="Help and keyboard shortcuts (?)"
            aria-keyshortcuts="Shift+/"
          >
            Help
          </button>
          <button type="button" className={button} onClick={openReport} disabled={busy !== null}>
            Open report
          </button>
          <button
            type="button"
            className={button}
            onClick={() => importReport()}
            disabled={busy !== null}
            title="Import a Tableau (.twb, .twbx) or Power BI (.pbit, .pbix) file"
          >
            Import Tableau / Power BI...
          </button>
          {isWeb && (
            <button
              type="button"
              className={button}
              onClick={() => importReport(true)}
              disabled={busy !== null}
              title="Import a Power BI Project folder"
            >
              Import Power BI folder...
            </button>
          )}
          <button type="button" className={button} onClick={() => saveReport(false)} disabled={busy !== null}>
            Save
          </button>
          <button type="button" className={button} onClick={() => saveReport(true)} disabled={busy !== null}>
            Save as
          </button>
          <span className="mx-1 h-6 w-px bg-border" aria-hidden="true" />
          {/* One group, so the format and its button never wrap onto separate lines. */}
          <div className="flex items-center gap-2" role="group" aria-label="Export">
            <label className="whitespace-nowrap text-sm text-muted" htmlFor="export-format">
              Export as
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
            <button
              type="button"
              className={primary}
              onClick={exportReport}
              disabled={busy !== null || report.visuals.length === 0 || paramsBlocked}
              title={
                paramsBlocked
                  ? "Fix the parameter values to export"
                  : report.visuals.length === 0
                    ? "Build a chart first, then export"
                    : undefined
              }
            >
              Export...
            </button>
          </div>
        </div>
      </header>

      <div className="flex min-h-0 flex-1">
        <aside
          id="data-panel"
          aria-label="Data panel"
          hidden={panelCollapsed}
          style={{ width: layout.panelWidth }}
          className="min-h-0 shrink-0 overflow-hidden border-r border-border bg-surface"
        >
          <DataPanel
            report={report}
            reportPath={reportPath}
            datasets={report.datasets}
            sources={report.sources}
            schemas={schemas}
            active={activeDataset}
            busy={busy !== null}
            onSelect={selectDataset}
            onAddFiles={addFiles}
            onAddField={addField}
            onSaveCalculated={saveCalculatedField}
            onDeleteCalculated={deleteCalculatedField}
            onSaveJoined={saveJoinedDataset}
            onDeleteJoined={deleteJoinedDataset}
            onCancelJoined={cancelJoinedEdit}
            onSaveParameter={saveParameter}
            onDeleteParameter={deleteParameter}
          />
        </aside>

        {!panelCollapsed && (
          <div
            role="separator"
            aria-orientation="vertical"
            aria-label="Resize data panel"
            aria-valuemin={PANEL_MIN}
            aria-valuemax={PANEL_MAX}
            aria-valuenow={layout.panelWidth}
            tabIndex={0}
            onKeyDown={onSeparatorKey}
            onPointerDown={onSeparatorPointerDown}
            onPointerMove={onSeparatorPointerMove}
            onPointerUp={onSeparatorPointerUp}
            onPointerCancel={onSeparatorPointerUp}
            className="group flex w-2 shrink-0 cursor-col-resize justify-center focus-visible:outline-2 focus-visible:outline-accent"
          >
            <span className="w-px bg-border group-hover:bg-accent group-focus-visible:bg-accent" aria-hidden="true" />
          </div>
        )}

        <main id="main-content" tabIndex={-1} className="flex min-h-0 min-w-0 flex-1 flex-col focus:outline-none">
          <div className="flex items-end gap-2 border-b border-border px-3 pt-2">
            <div role="tablist" aria-label="Views" className="flex gap-1">
              {VIEWS.map((name) => (
                <button
                  key={name}
                  id={`tab-${name}`}
                  ref={(el) => {
                    tabRefs.current[name] = el;
                  }}
                  role="tab"
                  type="button"
                  aria-selected={view === name}
                  aria-controls={`panel-${name}`}
                  tabIndex={view === name ? 0 : -1}
                  onClick={() => setView(name)}
                  onKeyDown={(e) => onTabKey(e, name)}
                  className={`rounded-t-md px-3 py-1.5 text-sm font-medium focus-visible:outline-2 focus-visible:outline-accent ${
                    view === name ? "bg-bg text-text ring-1 ring-border" : "text-muted hover:text-text"
                  }`}
                >
                  {name === "builder" ? "Chart builder" : `Report (${report.visuals.length})`}
                </button>
              ))}
            </div>
            <button
              type="button"
              aria-expanded={!panelCollapsed}
              aria-controls="data-panel"
              aria-keyshortcuts="Control+B"
              title="Show or hide the data panel (Ctrl+B)"
              onClick={togglePanel}
              className="mb-1.5 ml-auto inline-flex items-center gap-1.5 rounded-md border border-border bg-surface px-2.5 py-1 text-sm font-medium text-muted hover:bg-surface-2 hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
            >
              <svg aria-hidden="true" viewBox="0 0 24 24" className="h-4 w-4" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <rect x="3" y="4" width="18" height="16" rx="2" />
                <path d="M9 4v16" />
              </svg>
              Data panel
            </button>
          </div>

          {view === "builder" ? (
            <div
              role="tabpanel"
              id="panel-builder"
              aria-labelledby="tab-builder"
              className="flex min-h-0 flex-1 flex-col"
            >
              {!draft ? (
                <div className="flex min-h-0 flex-1 overflow-y-auto">
                  {report.datasets.length > 0 ? (
                    <EmptyState hasData onAddFiles={addFiles} />
                  ) : (
                    <Welcome
                      onOpenData={addFiles}
                      onOpenReport={openReport}
                      onOpenSample={openSample}
                      onOpenStored={openReportAt}
                      busy={busy !== null}
                    />
                  )}
                </div>
              ) : (
                <div className="grid min-h-0 flex-1 grid-cols-1 gap-3 overflow-y-auto p-3 min-[1000px]:grid-cols-[minmax(0,45fr)_minmax(0,55fr)] min-[1000px]:grid-rows-[minmax(0,1fr)] min-[1000px]:overflow-hidden">
                  <div className="flex min-h-0 flex-col gap-3 min-[1000px]:overflow-y-auto min-[1000px]:pr-1">
                    <div className="flex flex-wrap items-end gap-2">
                      <div className="min-w-0 flex-1">
                        <label htmlFor="visual-title" className="text-xs font-semibold uppercase tracking-wide text-muted">
                          Chart title
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
                        {savedVisual ? (draftChanged ? "Update chart" : "Saved") : "Add to report"}
                      </button>
                      {savedVisual && (
                        <button
                          type="button"
                          className={button}
                          onClick={() => deleteVisual(draft.id)}
                          title={
                            dashboardsWithVisual(report, draft.id).length > 0
                              ? `Also removes it from ${dashboardsWithVisual(report, draft.id).map((d) => d.title || d.id).join(", ")}`
                              : undefined
                          }
                        >
                          Delete chart
                        </button>
                      )}
                      <button
                        type="button"
                        className={button}
                        onClick={() => setDraft(blankVisual(draft.dataset, newVisualId(report)))}
                      >
                        New chart
                      </button>
                    </div>

                    {report.visuals.length > 0 && (
                      <div role="group" aria-label="Charts in report" className="flex flex-wrap gap-1">
                        {report.visuals.map((v) => (
                          <button
                            key={v.id}
                            type="button"
                            aria-pressed={v.id === draft.id}
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
                      parameters={report.parameters}
                      onSlot={setSlot}
                      onFieldDrop={addField}
                      onChange={(patch) => updateDraft((v) => ({ ...v, ...patch }))}
                    />
                  </div>

                  <div className="flex min-h-0 flex-col gap-3 min-[1000px]:overflow-y-auto">
                    <section
                      aria-label="Chart"
                      className="flex min-h-[300px] shrink-0 flex-col gap-2 rounded-md border border-border bg-surface p-3"
                    >
                      <div className="flex items-center gap-2 text-sm text-muted">
                        <span className="font-medium text-text">{draft.title || autoTitle(draft)}</span>
                        {render.result && !isTableSpec(render.result.spec) && (
                          <ShowTableToggle
                            className="ml-auto"
                            pressed={builderTable}
                            onToggle={() => setBuilderTable((on) => !on)}
                          />
                        )}
                        {render.loading && (
                          <>
                            <span className="spinner" aria-hidden="true" />
                            <span role="status">Running query</span>
                            <button
                              type="button"
                              className="rounded px-2 py-0.5 underline hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
                              onClick={() => api.cancelQuery().catch(() => undefined)}
                            >
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
                          <ChartView spec={render.result.spec} label={render.result.title} showTable={builderTable} />
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

                    <section aria-label="Data" className="flex h-72 shrink-0 flex-col gap-2">
                      <div role="group" aria-label="Data views" className="flex gap-1">
                        {(["preview", "profile"] as const).map((mode) => (
                          <button
                            key={mode}
                            type="button"
                            aria-pressed={bottomTab === mode}
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
                  </div>
                </div>
              )}
            </div>
          ) : (
            <div role="tabpanel" id="panel-report" aria-labelledby="tab-report" className="flex min-h-0 flex-1 flex-col">
              <DashboardsPanel
                report={report}
                schemas={schemas}
                selectedId={dashboardId}
                onSelect={setDashboardId}
                onNew={createDashboard}
                onSave={saveDashboard}
                onDelete={deleteDashboard}
                values={paramValues}
                blocked={paramsBlocked}
                onOpen={openVisual}
              >
                <ReportView
                  visuals={report.visuals}
                  parameters={report.parameters}
                  values={paramValues}
                  blocked={paramsBlocked}
                  onOpen={openVisual}
                />
              </DashboardsPanel>
            </div>
          )}
        </main>
      </div>

      <ToastStack toasts={toasts} onDismiss={dismissToast} />
      <HelpDialog open={helpOpen} onClose={() => setHelpOpen(false)} />
    </div>
  );
}

/**
 * The report's parameters as one header control. The button opens a popover that holds
 * the same inputs as the parameter values bar. Escape or an outside click closes it.
 */
function ParametersMenu({
  parameters,
  values,
  issues,
  onChange,
  onReset,
}: {
  parameters: Parameter[];
  values: Record<string, string>;
  issues: Record<string, string>;
  onChange: (name: string, text: string) => void;
  onReset: () => void;
}) {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const blocked = Object.keys(issues).length > 0;
  const summary = parameters.map((p) => `${p.name}: ${paramText(p, values)}`).join(", ");

  useEffect(() => {
    if (!open) return;
    function onPointerDown(e: PointerEvent) {
      if (!wrapRef.current?.contains(e.target as Node)) setOpen(false);
    }
    function onKeyDown(e: KeyboardEvent) {
      if (e.key !== "Escape") return;
      e.preventDefault();
      setOpen(false);
      buttonRef.current?.focus();
    }
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    dialogRef.current?.focus();
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  return (
    <div ref={wrapRef} className="relative flex min-w-0 items-center gap-2">
      <button
        ref={buttonRef}
        type="button"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls="params-popover"
        onClick={() => setOpen((o) => !o)}
        className={`${button} ${blocked ? "border-danger" : ""}`}
      >
        Parameters ({parameters.length})
      </button>
      {blocked && <span className="text-xs font-medium text-danger">Fix values</span>}
      <span className="min-w-0 max-w-64 truncate text-xs text-muted" title={summary}>
        {summary}
      </span>
      {open && (
        <div
          id="params-popover"
          ref={dialogRef}
          role="dialog"
          aria-label="Parameter values"
          tabIndex={-1}
          className="absolute left-0 top-full z-40 mt-2 max-h-[70vh] w-[min(28rem,calc(100vw-2rem))] overflow-y-auto rounded-md border border-border bg-surface p-3 shadow-lg focus:outline-none [&>section]:border-0 [&>section]:bg-transparent [&>section]:p-0"
        >
          <ParameterValuesBar parameters={parameters} values={values} issues={issues} onChange={onChange} onReset={onReset} />
        </div>
      )}
    </div>
  );
}

function ToastStack({ toasts, onDismiss }: { toasts: Toast[]; onDismiss: (id: number) => void }) {
  return (
    <div className="pointer-events-none fixed bottom-4 right-4 z-50 flex w-96 max-w-[calc(100vw-2rem)] flex-col gap-2">
      {toasts.map((toast) => (
        <ToastItem key={toast.id} toast={toast} onDismiss={onDismiss} />
      ))}
    </div>
  );
}

/**
 * One notice. Info and success toasts go away after TOAST_MS; errors stay until dismissed.
 * The timer pauses while the pointer is over the toast or focus is inside it.
 */
function ToastItem({ toast, onDismiss }: { toast: Toast; onDismiss: (id: number) => void }) {
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const paused = hovered || focused;
  const autoDismiss = toast.tone !== "error";
  const remaining = useRef(TOAST_MS);
  const startedAt = useRef(0);

  useEffect(() => {
    if (!autoDismiss || paused) return;
    startedAt.current = Date.now();
    const timer = window.setTimeout(() => onDismiss(toast.id), remaining.current);
    return () => {
      window.clearTimeout(timer);
      remaining.current = Math.max(0, remaining.current - (Date.now() - startedAt.current));
    };
  }, [autoDismiss, paused, toast.id, onDismiss]);

  const isError = toast.tone === "error";
  return (
    <div
      role={isError ? "alert" : "status"}
      aria-live={isError ? undefined : "polite"}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onFocus={() => setFocused(true)}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setFocused(false);
      }}
      className={`toast-enter pointer-events-auto flex items-start gap-3 rounded-md border bg-surface px-3 py-2 text-sm shadow-lg ${
        isError ? "border-danger" : "border-border"
      }`}
    >
      <p className={`min-w-0 flex-1 break-words ${isError ? "text-danger" : "text-text"}`}>{toast.text}</p>
      <button
        type="button"
        onClick={() => onDismiss(toast.id)}
        className="min-h-7 shrink-0 rounded-md px-2 text-sm font-medium text-muted hover:bg-surface-2 hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
      >
        Dismiss
      </button>
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

function ReportView({
  visuals,
  parameters,
  values,
  blocked,
  onOpen,
}: {
  visuals: Visual[];
  parameters: Parameter[];
  values: Record<string, string>;
  blocked: boolean;
  onOpen: (visual: Visual) => void;
}) {
  if (visuals.length === 0) {
    return <p className="p-6 text-sm text-muted">The report has no charts yet. Build one in the chart builder and add it to the report.</p>;
  }
  return (
    <div className="grid grid-cols-1 gap-3 overflow-y-auto p-3 xl:grid-cols-2">
      {visuals.map((visual) => (
        <VisualCard
          key={visual.id}
          visual={visual}
          parameters={parameters}
          values={values}
          blocked={blocked}
          onOpen={() => onOpen(visual)}
        />
      ))}
    </div>
  );
}

function VisualCard({
  visual,
  parameters,
  values,
  blocked,
  onOpen,
}: {
  visual: Visual;
  parameters: Parameter[];
  values: Record<string, string>;
  blocked: boolean;
  onOpen: () => void;
}) {
  const state = useRender(visual, 0, parameters, values, blocked);
  const [showTable, setShowTable] = useState(false);
  return (
    <article aria-label={visual.title || visual.id} className="flex min-w-0 flex-col gap-2 rounded-md border border-border bg-surface p-3">
      <div className="flex items-center justify-between gap-2">
        <h2 className="truncate text-sm font-semibold">{visual.title || visual.id}</h2>
        <div className="flex shrink-0 items-center gap-2">
          {visual.chart !== "table" && (
            <ShowTableToggle pressed={showTable} onToggle={() => setShowTable((on) => !on)} />
          )}
          <button type="button" onClick={onOpen} className="text-xs text-accent underline focus-visible:outline-2 focus-visible:outline-accent">
            Edit
          </button>
        </div>
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
      {state.result && <ChartView spec={state.result.spec} showTable={showTable} />}
    </article>
  );
}
