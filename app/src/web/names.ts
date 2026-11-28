// Pure helpers shared by the browser engine (no DOM, so they run under `node --test`).

import type { Family } from "../types";

const NUMBER_TYPES = [
  "TINYINT",
  "SMALLINT",
  "INTEGER",
  "INT",
  "INT4",
  "BIGINT",
  "INT8",
  "HUGEINT",
  "UTINYINT",
  "USMALLINT",
  "UINTEGER",
  "UBIGINT",
  "UHUGEINT",
  "FLOAT",
  "REAL",
  "DOUBLE",
  "DECIMAL",
  "NUMERIC",
];
const TEXT_TYPES = ["VARCHAR", "TEXT", "STRING", "CHAR", "BPCHAR", "UUID", "ENUM"];

/** Maps a DuckDB type name to the family used for icons and default roles. */
export function family(dataType: string): Family {
  const base = dataType.split("(")[0]!.trim().toUpperCase();
  if (NUMBER_TYPES.includes(base)) return "number";
  if (base === "BOOLEAN" || base === "BOOL") return "bool";
  if (TEXT_TYPES.includes(base)) return "text";
  if (base.startsWith("DATE") || base.startsWith("TIME")) return "date";
  return "other";
}

const WHOLE_NUMBER_TYPES = NUMBER_TYPES.slice(0, NUMBER_TYPES.indexOf("FLOAT"));

/**
 * A whole-number column whose name says it identifies rows (`id`, `order_id`, `customerId`,
 * `product_key`). Summing one is meaningless, so it starts as a dimension. Keep in step with
 * `is_identifier` in `app/src-tauri/src/service.rs`.
 */
export function isIdentifier(name: string, dataType: string): boolean {
  const base = dataType.split("(")[0]!.trim().toUpperCase();
  if (!WHOLE_NUMBER_TYPES.includes(base)) return false;
  const lower = name.toLowerCase();
  if (lower === "id" || lower === "key" || lower.endsWith("_id") || lower.endsWith("_key")) return true;
  return /[a-z]Id$/.test(name);
}

/** Turns a file name stem into a valid relation name (letters, digits, `_`). */
export function relationBase(stem: string): string {
  const name = stem.replace(/[^A-Za-z0-9_]/g, "_");
  return /^_*$/.test(name) ? "data" : name;
}

export function uniqueRelation(base: string, taken: Set<string>): string {
  const free = (name: string) => !taken.has(name) && !taken.has(`${name}_source`);
  if (free(base)) return base;
  for (let n = 2; ; n++) if (free(`${base}_${n}`)) return `${base}_${n}`;
}

/** A file name safe for output files, derived from a report name. */
export function outputStem(name: string): string {
  const stem = name.replace(/[^A-Za-z0-9_-]/g, "_").replace(/^_+|_+$/g, "");
  return stem === "" ? "report" : stem;
}

