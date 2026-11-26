// A browser has no file system, so the files the user picks are kept in a store under their
// names: in memory while they are being saved, then in the origin private file system (OPFS) so
// they survive a reload. A report's relative paths ("sample.csv") are looked up by file name.
//
// An entry is either bytes or a lazy `File`. A `File` costs no memory until something reads it,
// and DuckDB can read it in slices, which is how large data files stay usable.

import { zipSync } from "fflate";

import { clearStored, deleteStored, listStored, readStored, writeStored } from "./opfs";
import { apiError } from "./wasm";

export type StoredEntry = Uint8Array | File;

const store = new Map<string, StoredEntry>();

export const baseName = (path: string): string => path.split(/[\\/]/).pop() ?? path;

/** Files of a picked project folder, by their path inside it ("Project/Report/definition/report.json"). */
const raw = new Map<string, Uint8Array>();

export function rawFilesUnder(folder: string): [string, Uint8Array][] {
  const prefix = `${folder}/`;
  return [...raw].filter(([name]) => name.startsWith(prefix));
}

/** Resolves once files kept from earlier visits are back in the store. */
export const filesReady: Promise<void> = (async () => {
  for (const { name, file } of await listStored()) {
    if (!store.has(name)) store.set(name, file);
  }
})();

/** Adds a file (bytes or a picked `File`) and keeps a copy in OPFS where the browser allows it. */
export function putFile(name: string, data: StoredEntry): void {
  const key = baseName(name);
  store.set(key, data);
  void writeStored(key, data).then(async (kept) => {
    if (!kept || store.get(key) !== data) return;
    // From here on read the stored copy: it does not depend on the original file staying put.
    const stored = await readStored(key);
    if (stored && store.get(key) === data) store.set(key, stored);
  });
}

/** The entry for a file, bytes or a lazy `File`. */
export async function fileEntry(path: string): Promise<StoredEntry | undefined> {
  await filesReady;
  return store.get(baseName(path));
}

/** A file's bytes. */
export async function readFile(path: string): Promise<Uint8Array | undefined> {
  const entry = await fileEntry(path);
  if (!entry) return undefined;
  return entry instanceof File ? new Uint8Array(await entry.arrayBuffer()) : entry;
}

/** The files under `limit` bytes, for the converters (which read from an in-memory copy). */
export async function smallFiles(limit: number): Promise<[string, Uint8Array][]> {
  await filesReady;
  const found: [string, Uint8Array][] = [];
  for (const [name, entry] of store) {
    const size = entry instanceof File ? entry.size : entry.length;
    if (size > limit) continue;
    found.push([name, entry instanceof File ? new Uint8Array(await entry.arrayBuffer()) : entry]);
  }
  return found;
}

/** File extensions a SQLite database may have. */
export const SQLITE_EXTENSIONS = ["sqlite", "sqlite3", "db"];

/**
 * The stored file for a database connection: a report names a connection ("sample_db"), and the
 * browser has no environment variables, so the file is the one named after it (`sample_db.sqlite`,
 * `.sqlite3` or `.db`, in any letter case).
 */
export async function databaseFileFor(connection: string): Promise<string | undefined> {
  await filesReady;
  const wanted = SQLITE_EXTENSIONS.map((ext) => `${connection}.${ext}`.toLowerCase());
  return [...store.keys()].find((name) => wanted.includes(name.toLowerCase()));
}

export function hasFile(path: string): boolean {
  return store.has(baseName(path));
}

/** What the browser is keeping, for the "stored in this browser" list. */
export async function storedFiles(): Promise<{ name: string; size: number }[]> {
  await filesReady;
  return [...store]
    .map(([name, entry]) => ({ name, size: entry instanceof File ? entry.size : entry.length }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

export async function removeStoredFile(name: string): Promise<void> {
  store.delete(baseName(name));
  await deleteStored(baseName(name));
}

export async function clearStoredFiles(): Promise<void> {
  store.clear();
  raw.clear();
  await clearStored();
}

/** Shows the browser's file chooser. Resolves to null when the user cancels. */
function chooseFiles(accept: string, multiple: boolean): Promise<File[] | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = accept;
    input.multiple = multiple;
    input.style.display = "none";
    document.body.appendChild(input);
    const done = (files: File[] | null) => {
      input.remove();
      resolve(files);
    };
    input.addEventListener("change", () => done(input.files && input.files.length ? [...input.files] : null));
    input.addEventListener("cancel", () => done(null));
    input.click();
  });
}

async function keep(files: File[]): Promise<string[]> {
  // A picked File is lazy: it is not read into memory here.
  for (const file of files) putFile(file.name, file);
  return files.map((f) => f.name);
}

export async function pickDataFilesWeb(extensions: string[]): Promise<string[] | null> {
  const files = await chooseFiles(extensions.map((e) => `.${e}`).join(","), true);
  return files ? keep(files) : null;
}

/** A report and, optionally, the data files it reads. Resolves to the report's name. */
export async function pickReportWeb(): Promise<string | null> {
  const files = await chooseFiles(
    ".dvz,.csv,.parquet,.json,.jsonl,.ndjson,.arrow,.feather,.ipc,.sqlite,.sqlite3,.db",
    true,
  );
  if (!files) return null;
  const names = await keep(files);
  const report = names.find((n) => n.toLowerCase().endsWith(".dvz"));
  if (!report) throw apiError("invalid", "Choose a .dvz report file (and the data files it reads).");
  return report;
}

export function pickReportNameToSaveWeb(current: string | null): string | null {
  const name = window.prompt("Save the report as (a download)", baseName(current ?? "report.dvz"));
  return name && name.trim() ? name.trim() : null;
}

/** Hands a file to the browser's download. */
export function download(name: string, bytes: Uint8Array | string, type = "application/octet-stream"): void {
  const blob = new Blob([bytes as BlobPart], { type });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  document.body.appendChild(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}

/** One file downloads as itself; several download as one zip. Returns the names written. */
export function downloadAll(zipName: string, files: { name: string; bytes: Uint8Array }[]): string[] {
  if (files.length === 1) {
    download(files[0]!.name, files[0]!.bytes);
    return [files[0]!.name];
  }
  const entries: Record<string, Uint8Array> = {};
  for (const f of files) entries[f.name] = f.bytes;
  download(zipName, zipSync(entries), "application/zip");
  return files.map((f) => f.name);
}

/** A Tableau or Power BI file. Resolves to its name. */
export async function pickImportFileWeb(extensions: string[]): Promise<string | null> {
  const files = await chooseFiles(extensions.map((e) => `.${e}`).join(","), false);
  return files ? (await keep(files))[0]! : null;
}

/** A Power BI Project folder. Resolves to the folder's name. */
export function pickProjectFolderWeb(): Promise<string | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.setAttribute("webkitdirectory", "");
    input.style.display = "none";
    document.body.appendChild(input);
    const done = (value: string | null) => {
      input.remove();
      resolve(value);
    };
    input.addEventListener("change", async () => {
      const files = input.files ? [...input.files] : [];
      if (files.length === 0) return done(null);
      raw.clear();
      for (const file of files) {
        raw.set(file.webkitRelativePath || file.name, new Uint8Array(await file.arrayBuffer()));
      }
      done((files[0]!.webkitRelativePath || files[0]!.name).split("/")[0]!);
    });
    input.addEventListener("cancel", () => done(null));
    input.click();
  });
}
