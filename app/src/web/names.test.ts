import assert from "node:assert/strict";
import { test } from "node:test";

import { family, outputStem, relationBase, uniqueRelation } from "./names.ts";

test("family groups DuckDB types", () => {
  assert.equal(family("BIGINT"), "number");
  assert.equal(family("DECIMAL(18,3)"), "number");
  assert.equal(family("VARCHAR"), "text");
  assert.equal(family("BOOLEAN"), "bool");
  assert.equal(family("TIMESTAMP WITH TIME ZONE"), "date");
  assert.equal(family("STRUCT(a INTEGER)"), "other");
});

test("relation names are safe identifiers", () => {
  assert.equal(relationBase("my data-2026"), "my_data_2026");
  assert.equal(relationBase("---"), "data");
});

test("unique relations avoid the dataset and its source", () => {
  assert.equal(uniqueRelation("sales", new Set()), "sales");
  assert.equal(uniqueRelation("sales", new Set(["sales"])), "sales_2");
  assert.equal(uniqueRelation("sales", new Set(["sales_source"])), "sales_2");
  assert.equal(uniqueRelation("sales", new Set(["sales", "sales_2_source"])), "sales_3");
});

test("output stems are file-name safe", () => {
  assert.equal(outputStem("Sample sales (2026)"), "Sample_sales__2026");
  assert.equal(outputStem("***"), "report");
});
