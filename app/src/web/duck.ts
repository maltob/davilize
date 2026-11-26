// DuckDB-Wasm, loaded from a CDN on first use (the same way exported HTML pages load Vega).

const DUCKDB = "https://cdn.jsdelivr.net/npm/@duckdb/duckdb-wasm@1.32.0";
const ARROW = "https://cdn.jsdelivr.net/npm/apache-arrow@17.0.0/+esm";

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
    const [duckdb, arrow] = await Promise.all([
      import(/* @vite-ignore */ `${DUCKDB}/+esm`),
      import(/* @vite-ignore */ ARROW),
    ]);
    const bundle = await duckdb.selectBundle(duckdb.getJsDelivrBundles());
    const workerUrl = URL.createObjectURL(
      new Blob([`importScripts("${bundle.mainWorker}");`], { type: "text/javascript" }),
    );
    const db = new duckdb.AsyncDuckDB(new duckdb.ConsoleLogger(duckdb.LogLevel.WARNING), new Worker(workerUrl));
    await db.instantiate(bundle.mainModule, bundle.pthreadWorker);
    URL.revokeObjectURL(workerUrl);
    const conn = await db.connect();
    return { db, conn, arrow, duckdb };
  })();
  return loading;
}
