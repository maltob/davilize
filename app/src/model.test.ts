import assert from "node:assert/strict";
import { test } from "node:test";

import { aggregatesFor, defaultAgg, expectedKind, pillFor, smartSlot } from "./model.ts";
import type { ColumnView } from "./types.ts";

const col = (name: string, family: ColumnView["family"], role: ColumnView["role"]): ColumnView => ({
  name,
  data_type: family === "number" ? "BIGINT" : "VARCHAR",
  family,
  role,
});

const revenue = col("revenue", "number", "measure");
const orderId = col("order_id", "number", "dimension");
const region = col("region", "text", "dimension");
const when = col("order_date", "date", "dimension");
const flag = col("churned", "bool", "dimension");

test("text, dates and booleans are never offered Sum or Average", () => {
  for (const family of ["text", "date", "bool", "other"] as const) {
    const aggs = aggregatesFor(family);
    assert.ok(!aggs.includes("sum"), family);
    assert.ok(!aggs.includes("avg"), family);
    assert.ok(aggs.includes("count"), family);
  }
  assert.deepEqual(aggregatesFor("number").slice(0, 2), ["sum", "avg"]);
});

test("the default aggregate fits the field", () => {
  assert.equal(defaultAgg(revenue), "sum");
  assert.equal(defaultAgg(orderId), "count_distinct");
  assert.equal(defaultAgg(region), "count_distinct");
  assert.equal(defaultAgg(when), "count");
  assert.equal(defaultAgg(flag), "count");
  // The default is always one of the offered aggregates.
  for (const c of [revenue, orderId, region, when, flag]) {
    assert.ok(aggregatesFor(c.family).includes(defaultAgg(c)), c.name);
  }
});

test("each chart type says what its shelves hold", () => {
  assert.equal(expectedKind("bar", "x"), "dimension");
  assert.equal(expectedKind("bar", "y"), "measure");
  assert.equal(expectedKind("scatter", "x"), "measure");
  assert.equal(expectedKind("scatter", "color"), "dimension");
  assert.equal(expectedKind("heatmap", "color"), "measure");
  assert.equal(expectedKind("heatmap", "y"), "dimension");
  assert.equal(expectedKind("table", "x"), null);
});

test("a text field dropped on a measure shelf becomes a count, not a sum", () => {
  assert.deepEqual(pillFor(region, "y", "bar"), { field: "region", kind: "measure", agg: "count_distinct" });
  assert.deepEqual(pillFor(orderId, "y", "bar"), { field: "order_id", kind: "measure", agg: "count_distinct" });
  assert.deepEqual(pillFor(when, "y", "line"), { field: "order_date", kind: "measure", agg: "count" });
});

test("a number dropped on a grouping shelf groups by its values", () => {
  assert.deepEqual(pillFor(revenue, "x", "bar"), { field: "revenue", kind: "dimension", agg: "sum" });
});

test("a number on a measure shelf is summed", () => {
  assert.deepEqual(pillFor(revenue, "y", "bar"), { field: "revenue", kind: "measure", agg: "sum" });
  assert.deepEqual(pillFor(revenue, "x", "scatter"), { field: "revenue", kind: "measure", agg: "sum" });
});

test("without a shelf or on a table, numbers are measures and the rest group", () => {
  assert.equal(pillFor(revenue).kind, "measure");
  assert.equal(pillFor(region).kind, "dimension");
  assert.equal(pillFor(region, "x", "table").kind, "dimension");
  assert.equal(pillFor(revenue, "y", "table").kind, "measure");
});

test("a field dropped on the chart goes to the first shelf that suits it", () => {
  const empty = { x: null, y: null, color: null, size: null };
  const full = (field: string, kind: "dimension" | "measure") => ({ field, kind, agg: "sum" as const });
  // Numbers go to Rows, grouping fields to Columns, on an empty bar chart.
  assert.equal(smartSlot("bar", revenue, empty), "y");
  assert.equal(smartSlot("bar", region, empty), "x");
  // With Columns taken, a second grouping field becomes the series (Color).
  assert.equal(smartSlot("bar", when, { ...empty, x: full("region", "dimension") }), "color");
  // A scatter chart takes two numbers before it uses Size.
  assert.equal(smartSlot("scatter", revenue, { ...empty, y: full("units", "measure") }), "x");
  // Size is only offered on a scatter chart.
  const bar = { ...empty, x: full("region", "dimension"), y: full("units", "measure"), color: full("product", "dimension") };
  assert.notEqual(smartSlot("bar", revenue, bar), "size");
  // When every shelf is taken, the field replaces the main shelf of its kind.
  assert.equal(smartSlot("bar", revenue, bar), "y");
});
