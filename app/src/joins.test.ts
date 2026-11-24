// Unit tests for the joined-dataset helpers in model.ts. Run with: npm test (in app/)
// or node --experimental-strip-types --test src/joins.test.ts
/// <reference types="node" />
import { describe, test } from "node:test";
import assert from "node:assert/strict";
import {
  addJoinedDataset,
  buildJoinedDataset,
  datasetDependents,
  datasetsToRefresh,
  describeJoin,
  joinKeysError,
  joinKindOf,
  joinNameError,
  joinableRelations,
  removeDataset,
  replaceDataset,
} from "./model.ts";
import type { ColumnView, Dataset, Report, Visual } from "./types.ts";

const col = (name: string): ColumnView => ({ name, data_type: "VARCHAR", family: "text", role: "dimension" });

function report(extra: Partial<Report> = {}): Report {
  return {
    version: 1,
    name: "t",
    sources: [
      { name: "orders_csv", type: "file", path: "orders.csv", format: "csv" },
      { name: "regions_csv", type: "file", path: "regions.csv", format: "csv" },
    ],
    datasets: [
      { name: "orders", source: "orders_csv" },
      { name: "regions", source: "regions_csv" },
    ],
    visuals: [],
    parameters: [],
    ...extra,
  };
}

const joined: Dataset = {
  name: "orders_regions",
  source: "orders",
  joins: [{ with: "regions", kind: "left", on: [{ left: "region", right: "region" }] }],
};

function visual(id: string, dataset: string, title = ""): Visual {
  return {
    id,
    title,
    dataset,
    chart: "table",
    encodings: {},
    dimensions: [],
    measures: [],
    filters: [],
  };
}

describe("joinNameError", () => {
  test("accepts letters, digits, _ and -", () => {
    assert.equal(joinNameError("orders_regions-2", []), null);
  });

  test("rejects an empty name and other characters", () => {
    assert.equal(joinNameError("", []), "Enter a name.");
    assert.equal(joinNameError("a b", []), "Use only letters, digits, _ or -.");
    assert.notEqual(joinNameError("a.b", []), null);
  });

  test("rejects a name taken by a source or dataset, ignoring case", () => {
    assert.notEqual(joinNameError("Orders", ["orders_csv", "orders"]), null);
    assert.notEqual(joinNameError("regions_csv", ["regions_csv"]), null);
  });
});

describe("joinableRelations", () => {
  test("lists datasets in order with their columns", () => {
    const r = report();
    const schemas = { orders: [col("region")], regions: [col("region"), col("manager")] };
    const relations = joinableRelations(r, schemas);
    assert.deepEqual(
      relations.map((x) => x.name),
      ["orders", "regions"],
    );
    assert.deepEqual(
      relations[1]!.columns.map((c) => c.name),
      ["region", "manager"],
    );
  });

  test("a dataset without a schema has no columns", () => {
    assert.deepEqual(joinableRelations(report(), {})[0]!.columns, []);
  });

  test("an existing dataset may only use datasets defined before it", () => {
    const r = addJoinedDataset(report(), joined);
    assert.deepEqual(
      joinableRelations(r, {}, "regions").map((x) => x.name),
      ["orders"],
    );
    assert.deepEqual(
      joinableRelations(r, {}, "orders_regions").map((x) => x.name),
      ["orders", "regions"],
    );
  });
});

describe("datasetDependents", () => {
  test("nothing uses a plain dataset that is unused", () => {
    assert.deepEqual(datasetDependents(addJoinedDataset(report(), joined), "orders_regions"), []);
  });

  test("a dataset used as a join input or source is reported", () => {
    const r = addJoinedDataset(report(), joined);
    assert.deepEqual(datasetDependents(r, "regions"), ["orders_regions"]);
    assert.deepEqual(datasetDependents(r, "orders"), ["orders_regions"]);
  });

  test("a dataset used by a visual is reported by title, or id when untitled", () => {
    const r = addJoinedDataset(report({ visuals: [visual("v1", "orders_regions", "Sales")] }), joined);
    assert.deepEqual(datasetDependents(r, "orders_regions"), ["Sales"]);
    const r2 = { ...r, visuals: [visual("v2", "orders_regions")] };
    assert.deepEqual(datasetDependents(r2, "orders_regions"), ["v2"]);
  });

  test("a dataset does not count as its own dependent", () => {
    const r = report();
    assert.deepEqual(datasetDependents(r, "orders"), []);
  });
});

describe("joinKeysError", () => {
  test("requires at least one key", () => {
    assert.notEqual(joinKeysError([]), null);
  });

  test("requires both sides of every key", () => {
    assert.notEqual(joinKeysError([{ left: "region", right: "" }]), null);
    assert.equal(joinKeysError([{ left: "region", right: "region" }]), null);
  });

  test("rejects a repeated key pair", () => {
    const keys = [
      { left: "region", right: "region" },
      { left: "region", right: "region" },
    ];
    assert.notEqual(joinKeysError(keys), null);
  });

  test("allows the same column name on different keys", () => {
    const keys = [
      { left: "a", right: "b" },
      { left: "b", right: "a" },
    ];
    assert.equal(joinKeysError(keys), null);
  });
});

describe("buildJoinedDataset", () => {
  test("makes a source from the left dataset and one join to the right", () => {
    const d = buildJoinedDataset("orders_regions", "orders", "regions", "left", [
      { left: "region", right: "region" },
    ]);
    assert.deepEqual(d, {
      name: "orders_regions",
      source: "orders",
      joins: [{ with: "regions", kind: "left", on: [{ left: "region", right: "region" }] }],
    });
    assert.equal(d.sql, undefined);
  });

  test("keeps later joins and calculated fields of the dataset being edited", () => {
    const base: Dataset = {
      name: "orders_regions",
      source: "orders",
      joins: [
        { with: "regions", on: [{ left: "region", right: "region" }] },
        { with: "other", on: [{ left: "id", right: "id" }] },
      ],
      calculated_fields: [{ name: "x", expression: "units * 2" }],
    };
    const d = buildJoinedDataset("orders_regions", "orders", "regions", "full", [{ left: "region", right: "region" }], base);
    assert.equal(d.joins?.length, 2);
    assert.equal(d.joins?.[1]?.with, "other");
    assert.deepEqual(d.calculated_fields, [{ name: "x", expression: "units * 2" }]);
  });

  test("omits calculated_fields when there are none", () => {
    const d = buildJoinedDataset("n", "orders", "regions", "inner", [{ left: "a", right: "b" }], joined);
    assert.equal("calculated_fields" in d, false);
  });
});

describe("joinKindOf and describeJoin", () => {
  test("a missing kind is inner", () => {
    assert.equal(joinKindOf({ with: "r", on: [] }), "inner");
    assert.equal(joinKindOf({ with: "r", kind: "full", on: [] }), "full");
  });

  test("describes one join as SQL reads", () => {
    assert.equal(describeJoin(joined), "orders LEFT JOIN regions ON region = region");
  });

  test("an inner join with no kind says INNER", () => {
    const d: Dataset = { name: "x", source: "a", joins: [{ with: "b", on: [{ left: "k", right: "k2" }] }] };
    assert.equal(describeJoin(d), "a INNER JOIN b ON k = k2");
  });

  test("several keys are joined with AND", () => {
    const d: Dataset = {
      name: "x",
      source: "a",
      joins: [
        {
          with: "b",
          kind: "right",
          on: [
            { left: "k", right: "k" },
            { left: "y", right: "z" },
          ],
        },
      ],
    };
    assert.equal(describeJoin(d), "a RIGHT JOIN b ON k = k AND y = z");
  });

  test("a dataset without joins has no description", () => {
    assert.equal(describeJoin({ name: "orders", source: "orders_csv" }), "");
  });
});

describe("adding, replacing and removing joined datasets", () => {
  test("addJoinedDataset appends", () => {
    const r = addJoinedDataset(report(), joined);
    assert.deepEqual(
      r.datasets.map((d) => d.name),
      ["orders", "regions", "orders_regions"],
    );
  });

  test("replaceDataset keeps the position of the dataset", () => {
    const renamed: Dataset = { ...joined, name: "renamed" };
    const r = replaceDataset(addJoinedDataset(report(), joined), "orders_regions", renamed);
    assert.deepEqual(
      r.datasets.map((d) => d.name),
      ["orders", "regions", "renamed"],
    );
  });

  test("removeDataset drops the dataset and leaves the others", () => {
    const r = removeDataset(addJoinedDataset(report(), joined), "orders_regions");
    assert.deepEqual(
      r.datasets.map((d) => d.name),
      ["orders", "regions"],
    );
  });

  test("the input report is not changed", () => {
    const before = report();
    addJoinedDataset(before, joined);
    assert.equal(before.datasets.length, 2);
  });
});

describe("datasetsToRefresh", () => {
  test("nothing changed means nothing to refresh", () => {
    const r = addJoinedDataset(report(), joined);
    assert.deepEqual(datasetsToRefresh(r, r), []);
  });

  test("a new joined dataset is refreshed", () => {
    const before = report();
    const after = addJoinedDataset(before, joined);
    assert.deepEqual(
      datasetsToRefresh(before, after).map((d) => d.name),
      ["orders_regions"],
    );
  });

  test("a dataset whose joins changed is refreshed", () => {
    const before = addJoinedDataset(report(), joined);
    const changedJoin: Dataset = { ...joined, joins: [{ with: "regions", kind: "full", on: [{ left: "region", right: "region" }] }] };
    const after = replaceDataset(before, "orders_regions", changedJoin);
    assert.deepEqual(
      datasetsToRefresh(before, after).map((d) => d.name),
      ["orders_regions"],
    );
  });

  test("datasets built on a changed dataset are refreshed after it, in report order", () => {
    const calc: Dataset = { name: "orders", source: "orders_csv", calculated_fields: [{ name: "x", expression: "1" }] };
    const second: Dataset = { name: "second", source: "orders", joins: [{ with: "regions", on: [{ left: "a", right: "a" }] }] };
    const before = addJoinedDataset(addJoinedDataset(report(), joined), second);
    const after = { ...before, datasets: [calc, before.datasets[1]!, before.datasets[2]!, second] };
    assert.deepEqual(
      datasetsToRefresh(before, after).map((d) => d.name),
      ["orders", "orders_regions", "second"],
    );
  });

  test("a dataset whose right-hand input changed is refreshed", () => {
    const before = addJoinedDataset(report(), joined);
    const regions: Dataset = { name: "regions", source: "regions_csv", calculated_fields: [{ name: "y", expression: "2" }] };
    const after = { ...before, datasets: [before.datasets[0]!, regions, before.datasets[2]!] };
    assert.deepEqual(
      datasetsToRefresh(before, after).map((d) => d.name),
      ["regions", "orders_regions"],
    );
  });

  test("a removed dataset is not listed", () => {
    const before = addJoinedDataset(report(), joined);
    const after = removeDataset(before, "orders_regions");
    assert.deepEqual(datasetsToRefresh(before, after), []);
  });
});
