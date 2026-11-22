// Pure grid geometry for dashboards: clamping, overlap, placement, resizing and
// pixel conversion. No React and no DOM, so it is tested with node:test
// (see layout.test.ts and the test:layout script).
//
// Rules, shared by every caller:
// * Positions are 0-based grid units; sizes are at least 1 x 1.
// * An item must fit inside `columns` (x + w <= columns) and must not overlap another.
// * A move or resize is clamped to the grid first (a drag past the edge stops at the
//   edge). If the clamped target overlaps another item, the change is rejected and
//   the item keeps its last valid rectangle.

import type { DashboardItem } from "./types.ts";

/** A rectangle on the dashboard grid, in grid units. */
export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Cell {
  x: number;
  y: number;
}

export interface Size {
  w: number;
  h: number;
}

/** Limits and defaults of a dashboard, the same as dv-core (DASHBOARD_*). */
export const GRID_LIMITS = {
  minColumns: 1,
  maxColumns: 24,
  defaultColumns: 12,
  minRowHeight: 20,
  maxRowHeight: 400,
  defaultRowHeight: 80,
} as const;

/** Gap between cells in pixels. The CSS grid and the pixel maths both use it. */
export const GRID_GAP = 8;

/** Narrowest column the editor draws. Below this the canvas scrolls sideways. */
export const MIN_COLUMN_PX = 64;

/** Size of a visual added with "Add visual", before it is clamped to the columns. */
export const NEW_ITEM_SIZE: Size = { w: 6, h: 4 };

/** What the pixel conversions need to know about the grid on screen. */
export interface GridMetrics {
  columns: number;
  rowHeight: number;
  gap: number;
  columnWidth: number;
}

/** A rectangle in pixels, relative to the top-left of the grid. */
export interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

function clamp(n: number, low: number, high: number): number {
  return Math.min(high, Math.max(low, n));
}

/** Rounds to the nearest whole number, halves away from zero. Non-finite values become 0. */
export function roundHalfAway(n: number): number {
  if (!Number.isFinite(n)) return 0;
  const rounded = Math.sign(n) * Math.round(Math.abs(n));
  return rounded === 0 ? 0 : rounded;
}

/** Puts a rectangle inside the grid: size at least 1 x 1 and no wider than the grid, position inside the grid. */
export function clampRect(rect: Rect, columns: number): Rect {
  const cols = Math.max(1, roundHalfAway(columns));
  const w = clamp(roundHalfAway(rect.w), 1, cols);
  const h = Math.max(1, roundHalfAway(rect.h));
  const x = clamp(roundHalfAway(rect.x), 0, cols - w);
  const y = Math.max(0, roundHalfAway(rect.y));
  return { x, y, w, h };
}

/** True when the rectangle is a real size and lies inside `columns`. Rows are unbounded. */
export function fitsGrid(rect: Rect, columns: number): boolean {
  return rect.w >= 1 && rect.h >= 1 && rect.x >= 0 && rect.y >= 0 && rect.x + rect.w <= columns;
}

/** True when two rectangles share at least one cell. Touching edges do not overlap. */
export function overlaps(a: Rect, b: Rect): boolean {
  return a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
}

/** Moves a rectangle so its top-left is `cell`, keeping its size. The result is clamped to the grid. */
export function moveTo(rect: Rect, cell: Cell, columns: number): Rect {
  return clampRect({ x: cell.x, y: cell.y, w: rect.w, h: rect.h }, columns);
}

/**
 * Resizes a rectangle from its top-left corner. The width is limited so the item
 * stays inside the grid from where it starts; it never moves.
 */
export function resizeTo(rect: Rect, size: Size, columns: number): Rect {
  const cols = Math.max(1, roundHalfAway(columns));
  const x = clamp(roundHalfAway(rect.x), 0, cols - 1);
  const y = Math.max(0, roundHalfAway(rect.y));
  const w = clamp(roundHalfAway(size.w), 1, cols - x);
  const h = Math.max(1, roundHalfAway(size.h));
  return { x, y, w, h };
}

/** One cell step in each direction (keyboard moves). Clamped to the grid. */
export function moveBy(rect: Rect, dx: number, dy: number, columns: number): Rect {
  return moveTo(rect, { x: rect.x + dx, y: rect.y + dy }, columns);
}

/** Grows or shrinks by whole cells (keyboard resizes). Clamped to the grid and to 1 x 1. */
export function resizeBy(rect: Rect, dw: number, dh: number, columns: number): Rect {
  return resizeTo(rect, { w: rect.w + dw, h: rect.h + dh }, columns);
}

/** Items that share a cell with `rect`. `ignoreVisual` names the item being moved, which never blocks itself. */
export function overlappingItems(rect: Rect, items: readonly DashboardItem[], ignoreVisual?: string): DashboardItem[] {
  return items.filter((item) => item.visual !== ignoreVisual && overlaps(rect, item));
}

/**
 * The first position, reading left to right and then down, where a `size` rectangle
 * fits without overlapping `items`. The size is clamped to the grid. Always finds one:
 * the row below the lowest item is free.
 */
export function firstFreeSpot(items: readonly Rect[], size: Size, columns: number): Cell {
  const cols = Math.max(1, roundHalfAway(columns));
  const w = clamp(roundHalfAway(size.w), 1, cols);
  const h = Math.max(1, roundHalfAway(size.h));
  for (let y = 0; ; y += 1) {
    for (let x = 0; x + w <= cols; x += 1) {
      const candidate = { x, y, w, h };
      if (!items.some((item) => overlaps(candidate, item))) return { x, y };
    }
  }
}

/** Where a new visual goes: the default size, clamped, at the first free spot. */
export function placeNewItem(items: readonly Rect[], columns: number): Rect {
  const size = { w: Math.min(NEW_ITEM_SIZE.w, Math.max(1, columns)), h: NEW_ITEM_SIZE.h };
  const cell = firstFreeSpot(items, size, columns);
  return { x: cell.x, y: cell.y, w: size.w, h: size.h };
}

/** Rows used by the items: the bottom edge of the lowest item. */
export function gridHeight(items: readonly Rect[]): number {
  return items.reduce((bottom, item) => Math.max(bottom, item.y + item.h), 0);
}

export interface ChangeCheck {
  /** The clamped rectangle the user asked for. Shown as the ghost outline. */
  target: Rect;
  valid: boolean;
  /** Items the target overlaps (empty when valid). */
  blockers: DashboardItem[];
  /** What the item should be after the change: the target when valid, else its last valid rectangle. */
  next: Rect;
}

/**
 * Decides a move or a resize of `visual`. A target that overlaps another item is
 * rejected and the item keeps its position, so no change ever leaves two items on
 * the same cell.
 */
export function checkChange(items: readonly DashboardItem[], visual: string, proposed: Rect, columns: number): ChangeCheck {
  const target = clampRect(proposed, columns);
  const current = items.find((item) => item.visual === visual);
  const blockers = overlappingItems(target, items, visual);
  const valid = blockers.length === 0 && fitsGrid(target, columns);
  const origin: Rect = current ? { x: current.x, y: current.y, w: current.w, h: current.h } : target;
  return { target, valid, blockers, next: valid ? target : origin };
}

export type ColumnsCheck = { ok: true } | { ok: false; visuals: string[] };

/**
 * Whether a dashboard can have `columns` columns. Shrinking never moves items, so
 * the change is refused when an item would stick out past the new last column;
 * the result names those items so the user can move or resize them first.
 */
export function checkColumns(items: readonly DashboardItem[], columns: number): ColumnsCheck {
  const visuals = items.filter((item) => item.x + item.w > columns).map((item) => item.visual);
  return visuals.length === 0 ? { ok: true } : { ok: false, visuals };
}

/** Width of one column for a grid `containerWidth` pixels wide. Columns never get narrower than `minWidth`. */
export function columnWidth(containerWidth: number, columns: number, gap: number = GRID_GAP, minWidth: number = MIN_COLUMN_PX): number {
  const share = (containerWidth - gap * (columns - 1)) / columns;
  return Math.max(minWidth, share);
}

/** Grid metrics for a dashboard drawn in `containerWidth` pixels. */
export function gridMetrics(containerWidth: number, dashboard: { columns: number; row_height: number }): GridMetrics {
  return {
    columns: dashboard.columns,
    rowHeight: dashboard.row_height,
    gap: GRID_GAP,
    columnWidth: columnWidth(containerWidth, dashboard.columns),
  };
}

/** The pixel box of a rectangle, as the CSS grid draws it (gaps between cells, none around). */
export function cellBox(rect: Rect, metrics: GridMetrics): Box {
  return {
    left: rect.x * (metrics.columnWidth + metrics.gap),
    top: rect.y * (metrics.rowHeight + metrics.gap),
    width: rect.w * metrics.columnWidth + (rect.w - 1) * metrics.gap,
    height: rect.h * metrics.rowHeight + (rect.h - 1) * metrics.gap,
  };
}

/** The cell that contains a pixel offset from the grid's top-left corner. */
export function cellAtPixel(px: { x: number; y: number }, metrics: GridMetrics): Cell {
  return {
    x: Math.max(0, Math.floor(px.x / (metrics.columnWidth + metrics.gap))),
    y: Math.max(0, Math.floor(px.y / (metrics.rowHeight + metrics.gap))),
  };
}

/** How many cells a pointer movement of (dx, dy) pixels is: the nearest whole cell in each direction. */
export function cellsMoved(dx: number, dy: number, metrics: GridMetrics): Cell {
  return {
    x: roundHalfAway(dx / (metrics.columnWidth + metrics.gap)),
    y: roundHalfAway(dy / (metrics.rowHeight + metrics.gap)),
  };
}

export type ParseResult = { value: number } | { error: string };

/** A whole number from `min` to `max`, typed by the user. Blank, fractional or out-of-range text is an error. */
export function parseBounded(text: string, min: number, max: number, label: string): ParseResult {
  const trimmed = text.trim();
  if (!/^\d+$/.test(trimmed)) return { error: `${label} must be a whole number from ${min} to ${max}.` };
  const value = Number(trimmed);
  if (value < min || value > max) return { error: `${label} must be from ${min} to ${max}.` };
  return { value };
}
