// DuckDB-Wasm, served with the app (no CDN), so the page works offline once it is cached by the
// service worker. Only the "eh" build is shipped: every current browser supports WebAssembly
// exceptions. The library loads on first use, in its own chunk.

import engine from "@duckdb/duckdb-wasm/dist/duckdb-eh.wasm?url";
import worker from "@duckdb/duckdb-wasm/dist/duckdb-browser-eh.worker.js?url";

/* eslint-disable @typescript-eslint/no-explicit-any */
export interface Duck {
  db: any;
  conn: any;
  arrow: any;
  duckdb: any;
}

let loading: Promise<Duck> | undefined;

export function duck(): Promise<Duck> {
  loading ??= (async () => {
    const [duckdb, arrow]: any[] = await Promise.all([import("@duckdb/duckdb-wasm"), import("apache-arrow")]);
    const db = new duckdb.AsyncDuckDB(new duckdb.ConsoleLogger(duckdb.LogLevel.WARNING), new Worker(worker));
    await db.instantiate(engine);
    const conn = await db.connect();
    return { db, conn, arrow, duckdb };
  })();
  return loading;
}
