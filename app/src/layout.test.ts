// Unit tests for layout.ts. Run with: npm run test:layout (in app/).
// Uses only node:test and node:assert, and runs without a build step.
/// <reference types="node" />
import { describe, test } from "node:test";
import assert from "node:assert/strict";
import {
  GRID_GAP,
  cellAtPixel,
  cellBox,
  cellsMoved,
  checkChange,
  checkColumns,
  clampRect,
  columnWidth,
  firstFreeSpot,
  fitsGrid,
  gridHeight,
  gridMetrics,
  moveBy,
  moveTo,
  overlappingItems,
  overlaps,
  parseBounded,
  placeNewItem,
  resizeBy,
  resizeTo,
  roundHalfAway,
  type GridMetrics,
} from "./layout.ts";
import type { DashboardItem } from "./types.ts";

function item(visual: string, x: number, y: number, w: number, h: number): DashboardItem {
  return { visual, x, y, w, h };
}

const metrics: GridMetrics = { columns: 12, rowHeight: 80, gap: 8, columnWidth: 100 };

describe("clamping and size", () => {
  test("keeps an item inside the grid at the left, top and right edges", () => {
    assert.deepEqual(clampRect({ x: -3, y: -1, w: 2, h: 2 }, 12), { x: 0, y: 0, w: 2, h: 2 });
    assert.deepEqual(clampRect({ x: 11, y: 0, w: 4, h: 1 }, 12), { x: 8, y: 0, w: 4, h: 1 });
  });

  test("a size below 1 x 1 becomes 1 x 1", () => {
    assert.deepEqual(clampRect({ x: 0, y: 0, w: 0, h: -4 }, 12), { x: 0, y: 0, w: 1, h: 1 });
    assert.deepEqual(clampRect({ x: 3, y: 2, w: 0.2, h: 0.4 }, 12), { x: 3, y: 2, w: 1, h: 1 });
  });

  test("a width larger than the grid is cut to the grid", () => {
    assert.deepEqual(clampRect({ x: 5, y: 0, w: 40, h: 2 }, 12), { x: 0, y: 0, w: 12, h: 2 });
  });

  test("non-finite numbers are treated as 0 and then clamped", () => {
    assert.deepEqual(clampRect({ x: Number.NaN, y: Number.POSITIVE_INFINITY, w: 3, h: 1 }, 12), {
      x: 0,
      y: 0,
      w: 3,
      h: 1,
    });
  });

  test("rounding halves away from zero and never produces -0", () => {
    assert.equal(roundHalfAway(2.5), 3);
    assert.equal(roundHalfAway(-2.5), -3);
    assert.equal(Object.is(roundHalfAway(-0.2), 0), true);
  });

  test("fitsGrid rejects sizes below 1, negative positions and items past the last column", () => {
    assert.equal(fitsGrid({ x: 0, y: 0, w: 12, h: 1 }, 12), true);
    assert.equal(fitsGrid({ x: 1, y: 0, w: 12, h: 1 }, 12), false);
    assert.equal(fitsGrid({ x: 0, y: 0, w: 0, h: 1 }, 12), false);
    assert.equal(fitsGrid({ x: -1, y: 0, w: 1, h: 1 }, 12), false);
  });
});

describe("moving and resizing", () => {
  test("moving past an edge stops at the edge and keeps the size", () => {
    const rect = { x: 0, y: 0, w: 4, h: 2 };
    assert.deepEqual(moveTo(rect, { x: 20, y: 3 }, 12), { x: 8, y: 3, w: 4, h: 2 });
    assert.deepEqual(moveTo(rect, { x: -5, y: -5 }, 12), { x: 0, y: 0, w: 4, h: 2 });
  });

  test("keyboard moves are one cell and stop at the edges", () => {
    const rect = { x: 0, y: 0, w: 4, h: 2 };
    assert.deepEqual(moveBy(rect, 1, 0, 12), { x: 1, y: 0, w: 4, h: 2 });
    assert.deepEqual(moveBy(rect, -1, 0, 12), { x: 0, y: 0, w: 4, h: 2 });
    assert.deepEqual(moveBy({ x: 8, y: 0, w: 4, h: 2 }, 1, 0, 12), { x: 8, y: 0, w: 4, h: 2 });
  });

  test("resizing keeps the top-left corner and is limited by the right edge", () => {
    const rect = { x: 9, y: 1, w: 2, h: 2 };
    assert.deepEqual(resizeTo(rect, { w: 10, h: 3 }, 12), { x: 9, y: 1, w: 3, h: 3 });
    assert.deepEqual(resizeTo(rect, { w: 0, h: 0 }, 12), { x: 9, y: 1, w: 1, h: 1 });
  });

  test("keyboard resizes are one cell, never below 1 x 1", () => {
    assert.deepEqual(resizeBy({ x: 0, y: 0, w: 1, h: 1 }, -1, -1, 12), { x: 0, y: 0, w: 1, h: 1 });
    assert.deepEqual(resizeBy({ x: 0, y: 0, w: 3, h: 1 }, 1, 2, 12), { x: 0, y: 0, w: 4, h: 3 });
  });
});

describe("overlaps", () => {
  test("cells that share an edge do not overlap", () => {
    const a = { x: 0, y: 0, w: 2, h: 2 };
    assert.equal(overlaps(a, { x: 2, y: 0, w: 2, h: 2 }), false);
    assert.equal(overlaps(a, { x: 0, y: 2, w: 2, h: 2 }), false);
  });

  test("any shared cell overlaps, in either order", () => {
    const a = { x: 0, y: 0, w: 3, h: 3 };
    const b = { x: 2, y: 2, w: 2, h: 2 };
    assert.equal(overlaps(a, b), true);
    assert.equal(overlaps(b, a), true);
  });

  test("overlappingItems can ignore the item being moved", () => {
    const items = [item("a", 0, 0, 4, 2), item("b", 4, 0, 4, 2)];
    assert.deepEqual(
      overlappingItems({ x: 2, y: 0, w: 4, h: 2 }, items).map((i) => i.visual),
      ["a", "b"],
    );
    assert.deepEqual(
      overlappingItems({ x: 2, y: 0, w: 4, h: 2 }, items, "a").map((i) => i.visual),
      ["b"],
    );
  });
});

describe("placement", () => {
  test("the first free spot on an empty grid is the top-left corner", () => {
    assert.deepEqual(firstFreeSpot([], { w: 6, h: 4 }, 12), { x: 0, y: 0 });
  });

  test("the next spot is to the right of an item in the same rows", () => {
    assert.deepEqual(firstFreeSpot([item("a", 0, 0, 6, 4)], { w: 6, h: 4 }, 12), { x: 6, y: 0 });
  });

  test("when a row is full, the spot is in the next row down", () => {
    const items = [item("a", 0, 0, 6, 4), item("b", 6, 0, 6, 4)];
    assert.deepEqual(firstFreeSpot(items, { w: 6, h: 4 }, 12), { x: 0, y: 4 });
  });

  test("a gap that is too narrow is skipped", () => {
    // A 1-wide gap at column 5 cannot hold a 2-wide item.
    const items = [item("a", 0, 0, 5, 2), item("b", 6, 0, 6, 2)];
    assert.deepEqual(firstFreeSpot(items, { w: 2, h: 2 }, 12), { x: 0, y: 2 });
  });

  test("a size wider than the grid is clamped before the search", () => {
    assert.deepEqual(firstFreeSpot([item("a", 0, 0, 4, 1)], { w: 20, h: 1 }, 6), { x: 0, y: 1 });
  });

  test("placeNewItem uses the default size, clamped to the columns", () => {
    assert.deepEqual(placeNewItem([], 12), { x: 0, y: 0, w: 6, h: 4 });
    assert.deepEqual(placeNewItem([], 4), { x: 0, y: 0, w: 4, h: 4 });
    assert.deepEqual(placeNewItem([item("a", 0, 0, 12, 4)], 12), { x: 0, y: 4, w: 6, h: 4 });
  });

  test("gridHeight is the bottom of the lowest item", () => {
    assert.equal(gridHeight([]), 0);
    assert.equal(gridHeight([item("a", 0, 0, 1, 1), item("b", 0, 3, 1, 2)]), 5);
  });
});

describe("a move or resize into another item is rejected", () => {
  const items = [item("a", 0, 0, 4, 2), item("b", 6, 0, 4, 2)];

  test("a valid move is applied", () => {
    const check = checkChange(items, "a", { x: 0, y: 2, w: 4, h: 2 }, 12);
    assert.equal(check.valid, true);
    assert.deepEqual(check.next, { x: 0, y: 2, w: 4, h: 2 });
    assert.deepEqual(check.blockers, []);
  });

  test("a move onto another item keeps the last valid position and names the blocker", () => {
    const check = checkChange(items, "a", { x: 5, y: 0, w: 4, h: 2 }, 12);
    assert.equal(check.valid, false);
    assert.deepEqual(check.target, { x: 5, y: 0, w: 4, h: 2 });
    assert.deepEqual(check.next, { x: 0, y: 0, w: 4, h: 2 });
    assert.deepEqual(check.blockers.map((i) => i.visual), ["b"]);
  });

  test("a resize into another item is rejected", () => {
    const check = checkChange(items, "a", { x: 0, y: 0, w: 7, h: 2 }, 12);
    assert.equal(check.valid, false);
    assert.deepEqual(check.next, { x: 0, y: 0, w: 4, h: 2 });
  });

  test("an item never blocks itself", () => {
    const check = checkChange(items, "a", { x: 1, y: 0, w: 4, h: 2 }, 12);
    assert.equal(check.valid, true);
  });

  test("the target is clamped to the grid before it is checked", () => {
    const check = checkChange(items, "a", { x: 30, y: 9, w: 4, h: 2 }, 12);
    assert.deepEqual(check.target, { x: 8, y: 9, w: 4, h: 2 });
    assert.equal(check.valid, true);
  });
});

describe("pixels and grid cells", () => {
  test("a cell box matches the CSS grid: gaps between cells, none around", () => {
    assert.deepEqual(cellBox({ x: 0, y: 0, w: 1, h: 1 }, metrics), { left: 0, top: 0, width: 100, height: 80 });
    assert.deepEqual(cellBox({ x: 2, y: 1, w: 3, h: 2 }, metrics), {
      left: 2 * 108,
      top: 1 * 88,
      width: 3 * 100 + 2 * GRID_GAP,
      height: 2 * 80 + GRID_GAP,
    });
  });

  test("a pixel maps to the cell that contains it", () => {
    assert.deepEqual(cellAtPixel({ x: 0, y: 0 }, metrics), { x: 0, y: 0 });
    assert.deepEqual(cellAtPixel({ x: 107, y: 87 }, metrics), { x: 0, y: 0 });
    assert.deepEqual(cellAtPixel({ x: 108, y: 88 }, metrics), { x: 1, y: 1 });
    assert.deepEqual(cellAtPixel({ x: -20, y: -20 }, metrics), { x: 0, y: 0 });
  });

  test("a pointer movement rounds to the nearest whole cell", () => {
    assert.deepEqual(cellsMoved(53, 0, metrics), { x: 0, y: 0 });
    assert.deepEqual(cellsMoved(54, 0, metrics), { x: 1, y: 0 });
    assert.deepEqual(cellsMoved(-54, 44, metrics), { x: -1, y: 1 });
    assert.deepEqual(cellsMoved(-162, -176, metrics), { x: -2, y: -2 });
  });

  test("a cell box and a movement of one cell agree", () => {
    const one = cellBox({ x: 0, y: 0, w: 1, h: 1 }, metrics);
    const next = cellBox({ x: 1, y: 0, w: 1, h: 1 }, metrics);
    assert.equal(next.left - one.left, 108);
    assert.deepEqual(cellsMoved(108, 88, metrics), { x: 1, y: 1 });
  });

  test("columns are never narrower than the minimum, so a narrow canvas scrolls", () => {
    assert.equal(columnWidth(1200, 12), (1200 - 8 * 11) / 12);
    assert.equal(columnWidth(200, 12), 64);
    const m = gridMetrics(1200, { columns: 12, row_height: 80 });
    assert.equal(m.columns, 12);
    assert.equal(m.rowHeight, 80);
    assert.equal(m.gap, GRID_GAP);
  });
});

describe("changing the number of columns", () => {
  const items = [item("a", 0, 0, 6, 4), item("b", 6, 0, 6, 4), item("c", 2, 4, 3, 1)];

  test("growing the grid never blocks", () => {
    assert.deepEqual(checkColumns(items, 24), { ok: true });
  });

  test("shrinking to a size that every item fits in is allowed", () => {
    assert.deepEqual(checkColumns(items, 12), { ok: true });
  });

  test("shrinking past an item's right edge is refused and names every item that no longer fits", () => {
    assert.deepEqual(checkColumns(items, 8), { ok: false, visuals: ["b"] });
    assert.deepEqual(checkColumns(items, 5), { ok: false, visuals: ["a", "b"] });
    assert.deepEqual(checkColumns(items, 4), { ok: false, visuals: ["a", "b", "c"] });
  });

  test("the items themselves are not moved by the check", () => {
    const before = JSON.stringify(items);
    checkColumns(items, 5);
    assert.equal(JSON.stringify(items), before);
  });
});

describe("typed grid numbers", () => {
  test("whole numbers in range are accepted", () => {
    assert.deepEqual(parseBounded("12", 1, 24, "Columns"), { value: 12 });
    assert.deepEqual(parseBounded(" 7 ", 1, 24, "Columns"), { value: 7 });
  });

  test("blank, fractional, signed and out-of-range text is an error that names the field", () => {
    for (const text of ["", "1.5", "-1", "0", "25", "abc", "1e2"]) {
      const result = parseBounded(text, 1, 24, "Columns");
      assert.ok("error" in result, text);
      assert.match(result.error, /^Columns must be/);
    }
  });
});
