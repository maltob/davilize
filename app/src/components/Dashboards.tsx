import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
  type RefObject,
} from "react";
import { useRender } from "../hooks";
import {
  GRID_GAP,
  GRID_LIMITS,
  MIN_COLUMN_PX,
  cellBox,
  cellsMoved,
  checkChange,
  checkColumns,
  gridHeight,
  gridMetrics,
  moveBy,
  moveTo,
  parseBounded,
  placeNewItem,
  resizeBy,
  resizeTo,
  type Rect,
} from "../layout";
import { dashboardColumns, withDashboardFilters } from "../model";
import type {
  ColumnView,
  Dashboard,
  DashboardItem,
  Parameter,
  Report,
  Visual,
  Filter,
} from "../types";
import { ChartView } from "./ChartView";
import { FilterEditor } from "./FilterEditor";

const control =
  "rounded-md border border-border bg-surface px-2 py-1 text-sm text-text focus-visible:outline-2 focus-visible:outline-accent";
const button =
  "rounded-md border border-border bg-surface px-3 py-1.5 text-sm font-medium hover:bg-surface-2 disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-accent";
const smallButton =
  "rounded-md border border-border px-2 py-0.5 text-xs hover:bg-surface-2 disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-accent";
const label = "text-xs font-semibold uppercase tracking-wide text-muted";

/** Height of a card's title bar and the padding around its chart, in pixels. */
const CARD_CHROME_PX = 52;
const KEY_HELP =
  "Drag a card by its title bar, or select it and use the arrow keys to move it. Shift with an arrow key resizes it. Delete removes it from the dashboard. Escape cancels a drag.";

interface PanelProps {
  report: Report;
  schemas: Record<string, ColumnView[]>;
  /** The dashboard shown, or null for "All visuals". */
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  onNew: () => void;
  /** Replaces the dashboard with the same id (or adds it). */
  onSave: (dashboard: Dashboard) => void;
  onDelete: (id: string) => void;
  values: Record<string, string>;
  blocked: boolean;
  onOpen: (visual: Visual) => void;
  /** The "All visuals" view, shown when no dashboard is selected. */
  children: ReactNode;
}

/** The Report tab's view picker: All visuals, or one dashboard with its editor. */
export function DashboardsPanel({
  report,
  schemas,
  selectedId,
  onSelect,
  onNew,
  onSave,
  onDelete,
  values,
  blocked,
  onOpen,
  children,
}: PanelProps) {
  const dashboards = report.dashboards ?? [];
  const selected = dashboards.find((d) => d.id === selectedId) ?? null;
  const titleRef = useRef<HTMLInputElement>(null);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex flex-wrap items-end gap-2 border-b border-border px-3 py-2">
        <div className="flex flex-col gap-1">
          <label htmlFor="report-view" className={label}>
            Report view
          </label>
          <select
            id="report-view"
            value={selected ? selected.id : ""}
            onChange={(e) =>
              onSelect(e.target.value === "" ? null : e.target.value)
            }
            className={control}
          >
            <option value="">All visuals</option>
            {dashboards.map((d) => (
              <option key={d.id} value={d.id}>
                {d.title || d.id}
              </option>
            ))}
          </select>
        </div>
        <button type="button" className={button} onClick={onNew}>
          New dashboard
        </button>
        {selected && (
          <>
            <div className="flex min-w-0 flex-1 flex-col gap-1">
              <label htmlFor="dashboard-title" className={label}>
                Dashboard title
              </label>
              <input
                id="dashboard-title"
                ref={titleRef}
                value={selected.title}
                onChange={(e) => onSave({ ...selected, title: e.target.value })}
                className={`${control} w-full max-w-xs`}
              />
            </div>
            <button
              type="button"
              className={button}
              onClick={() => {
                titleRef.current?.focus();
                titleRef.current?.select();
              }}
            >
              Rename
            </button>
            <button
              type="button"
              className={button}
              onClick={() => onDelete(selected.id)}
              aria-label={`Delete dashboard ${selected.title || selected.id}`}
            >
              Delete dashboard
            </button>
          </>
        )}
      </div>
      {selected ? (
        <DashboardEditor
          key={selected.id}
          report={report}
          dashboard={selected}
          schemas={schemas}
          values={values}
          blocked={blocked}
          onSave={onSave}
          onOpen={onOpen}
        />
      ) : (
        <div className="flex min-h-0 flex-1 flex-col">{children}</div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Editor: settings, canvas, shared filters
// ---------------------------------------------------------------------------

interface Gesture {
  kind: "move" | "resize";
  visual: string;
  origin: Rect;
  start: { x: number; y: number };
  target: Rect;
}

interface EditorProps {
  report: Report;
  dashboard: Dashboard;
  schemas: Record<string, ColumnView[]>;
  values: Record<string, string>;
  blocked: boolean;
  onSave: (dashboard: Dashboard) => void;
  onOpen: (visual: Visual) => void;
}

function rectOf(item: Rect): Rect {
  return { x: item.x, y: item.y, w: item.w, h: item.h };
}

function sameRect(a: Rect, b: Rect): boolean {
  return a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h;
}

/** Width of an element, kept up to date while it resizes. */
function useElementWidth(ref: RefObject<HTMLElement | null>): number {
  const [width, setWidth] = useState(0);
  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    const update = () => setWidth(element.clientWidth);
    update();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref]);
  return width;
}

/** Fits a Vega-Lite spec to its card: numeric size becomes the card's size, as the HTML export does. */
function fitToCard(
  spec: Record<string, unknown>,
  height: number,
): Record<string, unknown> {
  const next: Record<string, unknown> = { ...spec };
  let fills = false;
  if (typeof next.width === "number") {
    next.width = "container";
    fills = true;
  }
  if (typeof next.height === "number") {
    next.height = height;
    fills = true;
  }
  if (fills) next.autosize = { type: "fit", contains: "padding", resize: true };
  return next;
}

function DashboardEditor({
  report,
  dashboard,
  schemas,
  values,
  blocked,
  onSave,
  onOpen,
}: EditorProps) {
  const items = dashboard.items;
  const columns = dashboard.columns;
  const surface = useRef<HTMLDivElement>(null);
  const width = useElementWidth(surface);
  const metrics = gridMetrics(width, dashboard);
  const [gesture, setGestureState] = useState<Gesture | null>(null);
  const gestureRef = useRef<Gesture | null>(null);
  const [announcement, setAnnouncement] = useState("");
  const [addChoice, setAddChoice] = useState<string | null>(null);
  const addRef = useRef<HTMLSelectElement>(null);

  const setGesture = (next: Gesture | null) => {
    gestureRef.current = next;
    setGestureState(next);
  };
  const announce = (text: string) => setAnnouncement(text);
  const titleOf = (id: string) =>
    report.visuals.find((v) => v.id === id)?.title || id;
  const namesOf = (list: readonly DashboardItem[]) =>
    list.map((i) => titleOf(i.visual)).join(", ");
  const visualsOnDashboard = new Set(items.map((i) => i.visual));
  const available = report.visuals.filter((v) => !visualsOnDashboard.has(v.id));
  const chosen =
    addChoice !== null && available.some((v) => v.id === addChoice)
      ? addChoice
      : (available[0]?.id ?? "");
  const dragging = gesture !== null;

  // Escape cancels a pointer drag.
  useEffect(() => {
    if (!dragging) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      setGesture(null);
      announce("Drag cancelled. The visual did not move.");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // setGesture only wraps a state setter, so the listener needs to be set once per drag.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dragging]);

  // No text selection while a card is dragged.
  useEffect(() => {
    if (!dragging) return;
    const previous = document.body.style.userSelect;
    document.body.style.userSelect = "none";
    return () => {
      document.body.style.userSelect = previous;
    };
  }, [dragging]);

  /** Applies a move or resize if it is valid. Otherwise the item keeps its last valid position. */
  function commitChange(visual: string, proposed: Rect, verb: string): boolean {
    const check = checkChange(items, visual, proposed, columns);
    const title = titleOf(visual);
    if (!check.valid) {
      const overlap =
        check.blockers.length > 0
          ? `It would overlap ${namesOf(check.blockers)}. `
          : "";
      announce(`Cannot ${verb} ${title}. ${overlap}It stays where it was.`);
      return false;
    }
    const next = check.next;
    const current = items.find((i) => i.visual === visual);
    if (current && sameRect(current, next)) {
      announce(`${title} did not move.`);
      return false;
    }
    onSave({
      ...dashboard,
      items: items.map((i) =>
        i.visual === visual
          ? { ...i, x: next.x, y: next.y, w: next.w, h: next.h }
          : i,
      ),
    });
    announce(
      `${title} is now at column ${next.x + 1}, row ${next.y + 1}, ${next.w} by ${next.h} cells.`,
    );
    return true;
  }

  function removeVisual(visual: string) {
    const title = titleOf(visual);
    onSave({ ...dashboard, items: items.filter((i) => i.visual !== visual) });
    announce(`Removed ${title} from ${dashboard.title || dashboard.id}.`);
    addRef.current?.focus();
  }

  function addVisual() {
    if (!chosen) return;
    const rect = placeNewItem(items, columns);
    onSave({ ...dashboard, items: [...items, { visual: chosen, ...rect }] });
    announce(
      `Added ${titleOf(chosen)} at column ${rect.x + 1}, row ${rect.y + 1}, ${rect.w} by ${rect.h} cells.`,
    );
  }

  // Settings. Each returns an error to show, or null when applied.
  function applyColumns(next: number): string | null {
    const check = checkColumns(items, next);
    if (!check.ok) {
      return `Cannot use ${next} columns: ${check.visuals.map(titleOf).join(", ")} would not fit. Move or resize them first.`;
    }
    onSave({ ...dashboard, columns: next });
    return null;
  }

  function applyRowHeight(next: number): string | null {
    onSave({ ...dashboard, row_height: next });
    return null;
  }

  // Pointer drag: the title bar moves a card, the corner grip resizes it.
  function beginGesture(
    event: ReactPointerEvent<HTMLElement>,
    kind: "move" | "resize",
    item: DashboardItem,
  ) {
    if (event.button !== 0) return;
    if ((event.target as HTMLElement).closest("button")) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    const origin = rectOf(item);
    setGesture({
      kind,
      visual: item.visual,
      origin,
      start: { x: event.clientX, y: event.clientY },
      target: origin,
    });
  }

  function trackGesture(event: ReactPointerEvent<HTMLElement>) {
    const g = gestureRef.current;
    if (!g) return;
    const delta = cellsMoved(
      event.clientX - g.start.x,
      event.clientY - g.start.y,
      metrics,
    );
    const target =
      g.kind === "move"
        ? moveTo(
            g.origin,
            { x: g.origin.x + delta.x, y: g.origin.y + delta.y },
            columns,
          )
        : resizeTo(
            g.origin,
            { w: g.origin.w + delta.x, h: g.origin.h + delta.y },
            columns,
          );
    if (!sameRect(target, g.target)) setGesture({ ...g, target });
  }

  function finishGesture() {
    const g = gestureRef.current;
    if (!g) return;
    setGesture(null);
    if (sameRect(g.target, g.origin)) return;
    commitChange(g.visual, g.target, g.kind === "move" ? "move" : "resize");
  }

  function cancelGesture() {
    if (!gestureRef.current) return;
    setGesture(null);
    announce("Drag cancelled. The visual did not move.");
  }

  // Keyboard: arrows move, Shift with arrows resizes, Delete removes.
  function onCardKey(
    event: ReactKeyboardEvent<HTMLElement>,
    item: DashboardItem,
  ) {
    if (event.target !== event.currentTarget) return;
    if (event.key === "Delete") {
      event.preventDefault();
      removeVisual(item.visual);
      return;
    }
    const step: Record<string, [number, number]> = {
      ArrowLeft: [-1, 0],
      ArrowRight: [1, 0],
      ArrowUp: [0, -1],
      ArrowDown: [0, 1],
    };
    const delta = step[event.key];
    if (!delta) return;
    event.preventDefault();
    const [dx, dy] = delta;
    const origin = rectOf(item);
    if (event.shiftKey) {
      commitChange(item.visual, resizeBy(origin, dx, dy, columns), "resize");
    } else {
      commitChange(item.visual, moveBy(origin, dx, dy, columns), "move");
    }
  }

  const lineRows = Math.max(4, gridHeight(items)) + 1;
  const minCanvas = cellBox(
    { x: 0, y: 0, w: 1, h: Math.max(4, gridHeight(items)) },
    metrics,
  ).height;
  const gridStyle = {
    gridTemplateColumns: `repeat(${columns}, minmax(${MIN_COLUMN_PX}px, 1fr))`,
    gridAutoRows: `${dashboard.row_height}px`,
    gap: `${GRID_GAP}px`,
  };
  const ghost = gesture
    ? (() => {
        const check = checkChange(
          items,
          gesture.visual,
          gesture.target,
          columns,
        );
        return { check, box: cellBox(check.target, metrics) };
      })()
    : null;

  const filters: Filter[] = dashboard.filters ?? [];
  const fieldColumns = dashboardColumns(report, dashboard, schemas);

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-3">
      <details
        open
        className="shrink-0 rounded-md border border-border bg-surface px-3 py-2"
      >
        <summary className="cursor-pointer text-sm font-medium">
          Grid settings and visuals
        </summary>
        <div className="mt-2 flex flex-col gap-2">
          <div className="flex flex-wrap items-end gap-4">
            <GridSetting
              id="dashboard-columns"
              label="Columns"
              min={GRID_LIMITS.minColumns}
              max={GRID_LIMITS.maxColumns}
              value={columns}
              unitName="columns"
              onApply={applyColumns}
            />
            <GridSetting
              id="dashboard-row-height"
              label="Row height (px)"
              min={GRID_LIMITS.minRowHeight}
              max={GRID_LIMITS.maxRowHeight}
              value={dashboard.row_height}
              unitName="row height"
              onApply={applyRowHeight}
            />
            <div className="flex flex-wrap items-end gap-2">
              <div className="flex flex-col gap-1">
                <label htmlFor="dashboard-add-visual" className={label}>
                  Add visual
                </label>
                <select
                  id="dashboard-add-visual"
                  ref={addRef}
                  value={chosen}
                  onChange={(e) => setAddChoice(e.target.value)}
                  disabled={available.length === 0}
                  className={`${control} max-w-56`}
                >
                  {available.length === 0 && (
                    <option value="">Every visual is on this dashboard</option>
                  )}
                  {available.map((v) => (
                    <option key={v.id} value={v.id}>
                      {v.title || v.id}
                    </option>
                  ))}
                </select>
              </div>
              <button
                type="button"
                className={button}
                onClick={addVisual}
                disabled={!chosen}
              >
                Add to dashboard
              </button>
            </div>
          </div>
          <p id="dashboard-keys" className="text-xs text-muted">
            {KEY_HELP}
          </p>
        </div>
      </details>
      <p role="status" aria-live="polite" className="sr-only">
        {announcement}
      </p>

      <div
        role="region"
        aria-label="Dashboard canvas"
        className="min-h-96 min-w-0 shrink-0 overflow-x-auto rounded-md border border-border bg-bg p-2"
      >
        <div
          ref={surface}
          className="relative"
          style={{ minHeight: minCanvas }}
        >
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-0 grid"
            style={gridStyle}
          >
            {Array.from({ length: columns * lineRows }, (_, k) => {
              const x = k % columns;
              const y = Math.floor(k / columns);
              return (
                <div
                  key={k}
                  className="border border-border/50"
                  style={{
                    gridColumn: `${x + 1} / span 1`,
                    gridRow: `${y + 1} / span 1`,
                  }}
                />
              );
            })}
          </div>
          <div className="relative grid" style={gridStyle}>
            {items.map((item) => {
              const visual = report.visuals.find((v) => v.id === item.visual);
              if (!visual) return null;
              const title = visual.title || visual.id;
              const box = cellBox(item, metrics);
              return (
                <article
                  key={item.visual}
                  role="group"
                  aria-roledescription="dashboard item"
                  aria-label={`${title}, column ${item.x + 1}, row ${item.y + 1}, ${item.w} by ${item.h} cells`}
                  aria-describedby="dashboard-keys"
                  tabIndex={0}
                  onKeyDown={(e) => onCardKey(e, item)}
                  style={{
                    gridColumn: `${item.x + 1} / span ${item.w}`,
                    gridRow: `${item.y + 1} / span ${item.h}`,
                  }}
                  className={`relative flex min-h-0 min-w-0 flex-col overflow-hidden rounded-md border border-border bg-surface focus-visible:outline-2 focus-visible:outline-accent motion-safe:transition-opacity ${
                    gesture?.visual === item.visual ? "opacity-50" : ""
                  }`}
                >
                  <header
                    onPointerDown={(e) => beginGesture(e, "move", item)}
                    onPointerMove={trackGesture}
                    onPointerUp={finishGesture}
                    onPointerCancel={cancelGesture}
                    className="flex cursor-grab touch-none select-none items-center gap-2 border-b border-border px-2 py-1 active:cursor-grabbing"
                  >
                    <h3
                      className="min-w-0 flex-1 truncate text-sm font-semibold"
                      title={title}
                    >
                      {title}
                    </h3>
                    <button
                      type="button"
                      onClick={() => onOpen(visual)}
                      className={smallButton}
                    >
                      Edit
                    </button>
                    <button
                      type="button"
                      onClick={() => removeVisual(item.visual)}
                      aria-label={`Remove ${title} from this dashboard`}
                      className={smallButton}
                    >
                      Remove
                    </button>
                  </header>
                  <div className="flex min-h-0 flex-1 flex-col gap-2 overflow-auto p-2">
                    <CardPreview
                      visual={visual}
                      parameters={report.parameters}
                      values={values}
                      blocked={blocked}
                      filters={filters}
                      height={Math.max(
                        72,
                        Math.floor(box.height - CARD_CHROME_PX),
                      )}
                    />
                  </div>
                  <div
                    aria-hidden="true"
                    title="Drag to resize"
                    onPointerDown={(e) => beginGesture(e, "resize", item)}
                    onPointerMove={trackGesture}
                    onPointerUp={finishGesture}
                    onPointerCancel={cancelGesture}
                    className="absolute bottom-0 right-0 h-4 w-4 cursor-nwse-resize touch-none rounded-tl border-l-2 border-t-2 border-border"
                  />
                </article>
              );
            })}
            {items.length === 0 && (
              <p className="pointer-events-none col-span-full p-4 text-center text-sm text-muted">
                No visuals on this dashboard yet. Choose one under Add visual.
              </p>
            )}
          </div>
          {ghost && (
            <div
              aria-hidden="true"
              className={`pointer-events-none absolute z-10 flex flex-col gap-0.5 overflow-hidden rounded-md border-2 border-dashed p-1 text-xs motion-safe:transition-[left,top,width,height] motion-safe:duration-100 ${
                ghost.check.valid
                  ? "border-accent bg-accent/10"
                  : "border-danger bg-danger-bg/70 text-danger"
              }`}
              style={{
                left: ghost.box.left,
                top: ghost.box.top,
                width: ghost.box.width,
                height: ghost.box.height,
              }}
            >
              {!ghost.check.valid && (
                <span className="font-medium">
                  Overlaps {namesOf(ghost.check.blockers)}
                </span>
              )}
            </div>
          )}
        </div>
      </div>

      <details
        open
        className="shrink-0 rounded-md border border-border bg-surface p-3"
      >
        <summary className="cursor-pointer text-sm font-medium">
          Shared filters ({filters.length})
        </summary>
        <div className="mt-3 flex flex-col gap-3">
          <p className="text-xs text-muted">
            A shared filter applies to every visual on this dashboard whose
            dataset has a column with that name. Values can be parameters, which
            are set in the parameter values bar above. Shared filters are rows
            only.
          </p>
          <FilterEditor
            title="Filters"
            filters={filters}
            columns={fieldColumns}
            parameters={report.parameters}
            onChange={(next) => onSave(withDashboardFilters(dashboard, next))}
          />
          <FilterNotes
            report={report}
            dashboard={dashboard}
            schemas={schemas}
          />
        </div>
      </details>
    </div>
  );
}

/** Which visuals each shared filter reaches, and which it is skipped for. */
function FilterNotes({
  report,
  dashboard,
  schemas,
}: {
  report: Report;
  dashboard: Dashboard;
  schemas: Record<string, ColumnView[]>;
}) {
  const filters = dashboard.filters ?? [];
  if (filters.length === 0) return null;
  const placed = dashboard.items
    .map((i) => report.visuals.find((v) => v.id === i.visual))
    .filter((v): v is Visual => v !== undefined);
  const titles = (list: Visual[]) =>
    list.map((v) => v.title || v.id).join(", ");
  return (
    <ul
      className="flex flex-col gap-1 text-xs text-muted"
      aria-label="Where each shared filter applies"
    >
      {filters.map((filter, index) => {
        const has = (v: Visual) =>
          (schemas[v.dataset] ?? []).some((c) => c.name === filter.field);
        const applies = placed.filter(has);
        const skipped = placed.filter((v) => !has(v));
        return (
          <li key={index}>
            <span className="font-medium text-text">
              {filter.field || "(no field)"}
            </span>{" "}
            applies to {applies.length > 0 ? titles(applies) : "no visual"}
            {skipped.length > 0 && (
              <>
                ; skipped for {titles(skipped)} (no column named &quot;
                {filter.field}&quot;)
              </>
            )}
            .
          </li>
        );
      })}
    </ul>
  );
}

/** A numeric grid setting. Checked as the user types, applied when the box is left or Enter is pressed. */
function GridSetting({
  id,
  label: text,
  min,
  max,
  value,
  unitName,
  onApply,
}: {
  id: string;
  label: string;
  min: number;
  max: number;
  value: number;
  unitName: string;
  onApply: (next: number) => string | null;
}) {
  const [draft, setDraft] = useState(String(value));
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setDraft(String(value));
    setError(null);
  }, [value]);

  const check = (raw: string) => parseBounded(raw, min, max, text);
  const commit = () => {
    const result = check(draft);
    if ("error" in result) {
      setError(result.error);
      return;
    }
    if (result.value === value) {
      setError(null);
      return;
    }
    setError(onApply(result.value));
  };
  const hintId = `${id}-hint`;

  return (
    <div className="flex flex-col gap-1">
      <label htmlFor={id} className={label}>
        {text}
      </label>
      <input
        id={id}
        type="number"
        inputMode="numeric"
        min={min}
        max={max}
        step={1}
        aria-label={`${unitName} (${min} to ${max})`}
        value={draft}
        onChange={(e) => {
          setDraft(e.target.value);
          const result = check(e.target.value);
          setError("error" in result ? result.error : null);
        }}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") commit();
        }}
        aria-invalid={error !== null}
        aria-describedby={hintId}
        className={`${control} w-24 ${error ? "border-danger" : ""}`}
      />
      <span
        id={hintId}
        className={`max-w-64 text-xs ${error ? "text-danger" : "text-muted"}`}
      >
        {error ?? `From ${min} to ${max}. Applied when you leave the box.`}
      </span>
    </div>
  );
}

/** A card's chart: the visual run with the dashboard's shared filters, sized to the card. */
function CardPreview({
  visual,
  parameters,
  values,
  blocked,
  filters,
  height,
}: {
  visual: Visual;
  parameters: Parameter[];
  values: Record<string, string>;
  blocked: boolean;
  filters: Filter[];
  height: number;
}) {
  const state = useRender(visual, 0, parameters, values, blocked, filters);
  const spec = useMemo(
    () => (state.result ? fitToCard(state.result.spec, height) : null),
    [state.result, height],
  );
  if (spec && state.result)
    return <ChartView spec={spec} label={state.result.title} />;
  if (state.error && state.error.kind !== "cancelled") {
    return (
      <p role="alert" className="break-words text-sm text-danger">
        {state.error.message}
      </p>
    );
  }
  if (state.loading) {
    return (
      <p className="flex items-center gap-2 text-sm text-muted" role="status">
        <span className="spinner" aria-hidden="true" /> Running query
      </p>
    );
  }
  return (
    <p className="text-sm text-muted">
      Give this visual a field on its shelves to draw it.
    </p>
  );
}
