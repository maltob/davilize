// Loads the Rust side (dv-wasm). The module is built by `scripts/build-site.sh` into
// `public/pkg`, so it is served next to the page and is not part of the bundle.

import type { ApiError } from "../types";
import { assetUrl } from "./platform";

interface DvWasm {
  open_report(text: string): string;
  save_report(reportJson: string): string;
  check_dataset(datasetJson: string): string;
  dataset_base_sql(datasetJson: string, relationColumnsJson: string): string;
  dataset_layers(datasetJson: string, baseSql: string, baseColumnsJson: string): string;
  calc_parse_query(field: string, expression: string): string;
  calc_verify(field: string, serialized: string): string;
  compile_visual(
    visualJson: string,
    parametersJson: string,
    valuesJson: string,
    filtersJson: string,
    columnsJson: string,
  ): string;
  prepare_visual(visualJson: string, ipc: Uint8Array): string;
  export_plan(reportJson: string, valuesJson: string, columnsJson: string, byDashboard: boolean): string;
  export_html(
    reportJson: string,
    valuesJson: string,
    columnsJson: string,
    results: Uint8Array,
    lengthsJson: string,
  ): string;
  export_xlsx(visualsJson: string, results: Uint8Array, lengthsJson: string): Uint8Array;
  xlsx_error(visualsJson: string, results: Uint8Array, lengthsJson: string): string;
  vfs_clear(): void;
  vfs_put(path: string, bytes: Uint8Array): void;
  vfs_take(path: string): Uint8Array | undefined;
  vfs_list(): string;
  interop_import(path: string, outDir: string): string;
  interop_export(reportJson: string, format: string, out: string): string;
}

let loading: Promise<DvWasm> | undefined;

export function wasm(): Promise<DvWasm> {
  loading ??= (async () => {
    const module = await import(/* @vite-ignore */ assetUrl("pkg/dv_wasm.js"));
    await module.default(assetUrl("pkg/dv_wasm_bg.wasm"));
    return module as DvWasm;
  })();
  return loading;
}

/** An error in the shape the rest of the app expects. */
export function apiError(kind: string, message: string): ApiError {
  return { kind, message };
}

/** Parses a dv-wasm reply; a `{"ok": false}` reply is thrown as an `invalid` error. */
export function unwrap<T = Record<string, unknown>>(text: string): T {
  const reply = JSON.parse(text) as { ok: boolean; error?: string };
  if (!reply.ok) {
    const message = (reply.error ?? "unknown error").replace(/^invalid (report|spec): /, "");
    throw apiError("invalid", message);
  }
  return reply as unknown as T;
}
