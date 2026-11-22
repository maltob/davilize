// Unit tests for history.ts. Run with: npm run test:layout (in app/) or
// node --experimental-strip-types --test src/history.test.ts
/// <reference types="node" />
import { describe, test } from "node:test";
import assert from "node:assert/strict";
import {
  COALESCE_MS,
  HISTORY_LIMIT,
  canRedo,
  canUndo,
  createHistory,
  push,
  redo,
  redoLabel,
  replace,
  undo,
  undoLabel,
} from "./history.ts";

describe("history", () => {
  test("starts with nothing to undo or redo", () => {
    const h = createHistory("a");
    assert.equal(h.present, "a");
    assert.equal(canUndo(h), false);
    assert.equal(canRedo(h), false);
    assert.equal(undoLabel(h), null);
    assert.equal(redoLabel(h), null);
  });

  test("push, undo and redo move through states", () => {
    let h = createHistory("a");
    h = push(h, "b", "Step one", null, 0);
    h = push(h, "c", "Step two", null, 10_000);
    assert.equal(h.present, "c");
    assert.equal(undoLabel(h), "Step two");

    h = undo(h);
    assert.equal(h.present, "b");
    assert.equal(redoLabel(h), "Step two");
    h = undo(h);
    assert.equal(h.present, "a");
    assert.equal(canUndo(h), false);
    assert.equal(canRedo(h), true);

    h = redo(h);
    assert.equal(h.present, "b");
    h = redo(h);
    assert.equal(h.present, "c");
    assert.equal(canRedo(h), false);
  });

  test("undo and redo on an empty side return the same history", () => {
    const h = createHistory(1);
    assert.equal(undo(h), h);
    assert.equal(redo(h), h);
  });

  test("a new push clears the redo steps", () => {
    let h = createHistory("a");
    h = push(h, "b", "One", null, 0);
    h = undo(h);
    h = push(h, "c", "Two", null, 10_000);
    assert.equal(canRedo(h), false);
    assert.equal(h.present, "c");
  });

  test("pushing the identical value records nothing", () => {
    const value = { name: "x" };
    const h = createHistory(value);
    assert.equal(push(h, value, "Same", null, 0), h);
  });

  test("pushes with the same key within COALESCE_MS form one step", () => {
    let h = createHistory("");
    h = push(h, "T", "Rename report", "report-name", 1_000);
    h = push(h, "Ti", "Rename report", "report-name", 1_200);
    h = push(h, "Tit", "Rename report", "report-name", 1_000 + COALESCE_MS + 0);
    assert.equal(h.present, "Tit");
    assert.equal(h.past.length, 1);
    h = undo(h);
    assert.equal(h.present, "");
    assert.equal(canUndo(h), false);
  });

  test("a pause longer than COALESCE_MS starts a new step", () => {
    let h = createHistory("");
    h = push(h, "T", "Rename report", "report-name", 1_000);
    h = push(h, "Ti", "Rename report", "report-name", 1_000 + COALESCE_MS + 1);
    assert.equal(h.past.length, 2);
    h = undo(h);
    assert.equal(h.present, "T");
  });

  test("different keys, or no key, never coalesce", () => {
    let h = createHistory(0);
    h = push(h, 1, "A", "a", 0);
    h = push(h, 2, "B", "b", 10);
    h = push(h, 3, "C", null, 20);
    h = push(h, 4, "D", null, 30);
    assert.equal(h.past.length, 4);
  });

  test("undoing breaks the coalescing run", () => {
    let h = createHistory(0);
    h = push(h, 1, "A", "k", 0);
    h = undo(h);
    h = push(h, 2, "A", "k", 10);
    assert.equal(h.past.length, 1);
    assert.equal(h.present, 2);
  });

  test("replace changes the present without recording a step", () => {
    let h = createHistory("a");
    h = push(h, "b", "One", null, 0);
    const saved = replace(h, "saved");
    assert.equal(saved.present, "saved");
    assert.equal(saved.past.length, 1);
    assert.equal(canRedo(saved), false);
  });

  test("history is bounded to HISTORY_LIMIT steps", () => {
    let h = createHistory(0);
    for (let i = 1; i <= HISTORY_LIMIT + 50; i += 1) {
      h = push(h, i, `Change ${i}`, null, i);
    }
    assert.equal(h.past.length, HISTORY_LIMIT);
    assert.equal(h.present, HISTORY_LIMIT + 50);
    let steps = 0;
    while (canUndo(h)) {
      h = undo(h);
      steps += 1;
    }
    assert.equal(steps, HISTORY_LIMIT);
    assert.equal(h.present, 50);
  });

  test("labels follow the steps in order", () => {
    let h = createHistory(0);
    h = push(h, 1, "Add chart", null, 0);
    h = push(h, 2, "Delete chart", null, 10_000);
    assert.equal(undoLabel(h), "Delete chart");
    h = undo(h);
    assert.equal(undoLabel(h), "Add chart");
    assert.equal(redoLabel(h), "Delete chart");
  });
});
