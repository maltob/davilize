// Unit tests for chartText.ts. Run with: node --experimental-strip-types --test src/chartText.test.ts (in app/).
// Uses only node:test and node:assert, and runs without a build step.
/// <reference types="node" />
import { describe, test } from "node:test";
import assert from "node:assert/strict";
import {
  TABLE_MAX_ROWS,
  capText,
  chartTable,
  chartSummary,
  chartThemeConfig,
  formatNumber,
  mergeConfig,
  readSpec,
  themedSpec,
  type ChartSpec,
} from "./chartText.ts";

function bar(rows: Record<string, unknown>[]): ChartSpec {
  return {
    mark: { type: "bar", tooltip: true },
    encoding: {
      x: { field: "region", type: "nominal", sort: null },
      y: { field: "total_revenue", type: "quantitative" },
    },
    data: { values: rows },
  };
}

describe("summaries", () => {
  test("bar: names the measure, the category, the count and the extremes", () => {
    const spec = bar([
      { region: "North", total_revenue: 735 },
      { region: "South", total_revenue: 410 },
      { region: "East", total_revenue: 260 },
      { region: "West", total_revenue: 120 },
    ]);
    assert.equal(
      chartSummary(spec),
      "Bar chart: total_revenue by region. 4 bars. Highest: North 735, lowest: West 120.",
    );
  });

  test("bar with one bar uses the singular and has no contrast to report", () => {
    assert.equal(
      chartSummary(bar([{ region: "North", total_revenue: 5 }])),
      "Bar chart: total_revenue by region. 1 bar. Highest: North 5, lowest: North 5.",
    );
  });

  test("line: counts points and labels a colour series", () => {
    const spec = {
      mark: { type: "line", point: true },
      encoding: {
        x: { field: "day", type: "nominal" },
        y: { field: "v", type: "quantitative" },
        color: { field: "g", type: "nominal" },
      },
      data: {
        values: [
          { day: "Mon", v: 1.5, g: "a" },
          { day: "Tue", v: 3, g: "b" },
        ],
      },
    };
    assert.equal(
      chartSummary(spec),
      "Line chart: v by day. 2 points. Highest: Tue (b) 3, lowest: Mon (a) 1.5.",
    );
  });

  test("scatter: gives the count and the range of each axis", () => {
    const spec = {
      mark: { type: "point", filled: true },
      encoding: { x: { field: "x", type: "quantitative" }, y: { field: "y", type: "quantitative" } },
      data: {
        values: [
          { x: 1, y: 10 },
          { x: 4, y: 5 },
          { x: 2.5, y: 7.5 },
        ],
      },
    };
    assert.equal(chartSummary(spec), "Scatter plot: y against x. 3 points. x from 1 to 4, y from 5 to 10.");
  });

  test("heatmap: labels extremes by both axes", () => {
    const spec = {
      mark: { type: "rect" },
      encoding: {
        x: { field: "month", type: "nominal" },
        y: { field: "region", type: "nominal" },
        color: { field: "sales", type: "quantitative" },
      },
      data: {
        values: [
          { month: "Jan", region: "North", sales: 3 },
          { month: "Feb", region: "South", sales: 9 },
        ],
      },
    };
    assert.equal(
      chartSummary(spec),
      "Heatmap: sales by month and region. 2 cells. Highest: Feb, South 9, lowest: Jan, North 3.",
    );
  });

  test("table: counts rows and lists columns", () => {
    const spec = {
      repeat: { column: ["region", "total_revenue"] },
      data: { values: [{ __row: 0, region: "North", total_revenue: 735 }] },
    };
    assert.equal(chartSummary(spec), "Table: 1 row. Columns: region, total_revenue.");
  });

  test("summaries are capped at 250 characters and end with an ellipsis", () => {
    const long = "x".repeat(400);
    const summary = chartSummary(bar([{ region: long, total_revenue: 1 }]));
    assert.ok([...summary].length <= 250);
    assert.ok(summary.endsWith("..."));
  });

  test("capText leaves short text alone and counts characters, not bytes", () => {
    assert.equal(capText("short"), "short");
    const emoji = "\u{1F600}".repeat(260);
    assert.equal([...capText(emoji)].length, 250);
  });

  test("number formatting drops trailing zeros", () => {
    assert.equal(formatNumber(735), "735");
    assert.equal(formatNumber(2.5), "2.5");
    assert.equal(formatNumber(100.004), "100");
    assert.equal(formatNumber(1.236), "1.24");
  });
});

describe("tables", () => {
  test("bar values come out in the spec's order with numeric columns marked", () => {
    const table = chartTable(bar([{ region: "West", total_revenue: 120 }, { region: "North", total_revenue: 735 }]));
    assert.deepEqual(table.columns, ["region", "total_revenue"]);
    assert.deepEqual(table.rows, [
      ["West", 120],
      ["North", 735],
    ]);
    assert.deepEqual(table.numeric, [false, true]);
    assert.equal(table.truncated, false);
  });

  test("table charts keep the row order from __row and hide the index column", () => {
    const spec = {
      repeat: { column: ["name"] },
      data: { values: [{ __row: 1, name: "second" }, { __row: 0, name: "first" }] },
    };
    const table = chartTable(spec);
    assert.deepEqual(table.columns, ["name"]);
    assert.deepEqual(table.rows, [["first"], ["second"]]);
  });

  test("more than 200 rows are cut and flagged", () => {
    const rows = Array.from({ length: TABLE_MAX_ROWS + 5 }, (_, i) => ({ region: `r${i}`, total_revenue: i }));
    const table = chartTable(bar(rows));
    assert.equal(table.rows.length, TABLE_MAX_ROWS);
    assert.equal(table.totalRows, TABLE_MAX_ROWS + 5);
    assert.equal(table.truncated, true);
  });

  test("blank and non-finite values are shown as null", () => {
    const table = chartTable(bar([{ region: null, total_revenue: Number.NaN }]));
    assert.deepEqual(table.rows, [[null, null]]);
    assert.deepEqual(table.numeric, [false, false]);
  });
});

describe("theme", () => {
  const colours = { text: "#e6ebf1", muted: "#9aa6b4", line: "#5d6b7a", grid: "#2e3a48" };

  test("theme config carries the app's colours for text, axes and legends", () => {
    const theme = chartThemeConfig(colours);
    assert.deepEqual(theme.axis, {
      labelColor: "#e6ebf1",
      titleColor: "#e6ebf1",
      domainColor: "#5d6b7a",
      tickColor: "#5d6b7a",
      gridColor: "#2e3a48",
    });
  });

  test("merging keeps the spec's own config and lets it win on shared keys", () => {
    const merged = mergeConfig(chartThemeConfig(colours), {
      background: "transparent",
      axis: { labelFontSize: 12, labelColor: "#000000" },
    });
    assert.equal(merged.background, "transparent");
    assert.deepEqual(merged.axis, {
      labelColor: "#000000",
      titleColor: "#e6ebf1",
      domainColor: "#5d6b7a",
      tickColor: "#5d6b7a",
      gridColor: "#2e3a48",
      labelFontSize: 12,
    });
  });

  test("themedSpec drops the Vega title without changing its input", () => {
    const spec = { title: { text: "Sales", subtitle: "capped" }, config: { background: "transparent" } };
    const themed = themedSpec(spec, chartThemeConfig(colours));
    assert.equal("title" in themed, false);
    assert.equal(spec.title.text, "Sales");
    assert.equal((themed.config as ChartSpec).background, "transparent");
    assert.deepEqual(((themed.config as ChartSpec).legend as ChartSpec).labelColor, "#e6ebf1");
  });

  test("readSpec recognises each chart type", () => {
    assert.equal(readSpec(bar([])).kind, "bar");
    assert.equal(readSpec({ mark: "line" }).kind, "line");
    assert.equal(readSpec({ mark: { type: "point" } }).kind, "scatter");
    assert.equal(readSpec({ mark: { type: "rect" } }).kind, "heatmap");
    assert.equal(readSpec({ repeat: { column: [] } }).kind, "table");
    assert.equal(readSpec({ mark: "arc" }).kind, "other");
  });
});
