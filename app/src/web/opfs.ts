// The browser's origin private file system (OPFS): storage that survives a reload, private to
// this site, and not visible in the user's own folders. The page keeps picked data files and
// saved reports here so they do not have to be chosen again. Everything degrades to
// "in memory for this session" where OPFS is missing or refuses (private windows, some
// browsers, a full disk), so each function reports failure instead of throwing.

const FOLDER = "davilize-files";

/* eslint-disable @typescript-eslint/no-explicit-any */
async function folder(): Promise<any | null> {
  try {
    if (!navigator.storage?.getDirectory) return null;
    const root = await navigator.storage.getDirectory();
    return await root.getDirectoryHandle(FOLDER, { create: true });
  } catch {
    return null;
  }
}

const encode = (name: string) => encodeURIComponent(name);
const decode = (name: string) => decodeURIComponent(name);

let asked = false;

/** Asks the browser not to evict stored files under storage pressure (it may decline silently). */
async function requestPersistence(): Promise<void> {
  if (asked) return;
  asked = true;
  try {
    await navigator.storage?.persist?.();
  } catch {
    // Not available: files may be evicted when the disk is nearly full.
  }
}

/** Stores a file. Bytes and `File`s both work; a `File` is streamed, so a large one is never held in memory. */
export async function writeStored(name: string, data: Uint8Array | File): Promise<boolean> {
  const dir = await folder();
  if (!dir) return false;
  try {
    const handle = await dir.getFileHandle(encode(name), { create: true });
    if (typeof handle.createWritable !== "function") return false;
    const writable = await handle.createWritable();
    if (data instanceof File) {
      await data.stream().pipeTo(writable);
    } else {
      await writable.write(data);
      await writable.close();
    }
    void requestPersistence();
    return true;
  } catch {
    return false;
  }
}

/** A stored file as a lazy `File` (its bytes are read only when something asks for them). */
export async function readStored(name: string): Promise<File | undefined> {
  const dir = await folder();
  if (!dir) return undefined;
  try {
    return await (await dir.getFileHandle(encode(name))).getFile();
  } catch {
    return undefined;
  }
}

/** Every stored file: its name and a lazy `File`. */
export async function listStored(): Promise<{ name: string; file: File }[]> {
  const dir = await folder();
  if (!dir) return [];
  const found: { name: string; file: File }[] = [];
  try {
    for await (const [key, handle] of dir.entries()) {
      if (handle.kind !== "file") continue;
      found.push({ name: decode(key), file: await handle.getFile() });
    }
  } catch {
    // A partial list is better than none.
  }
  return found;
}

export async function deleteStored(name: string): Promise<void> {
  const dir = await folder();
  try {
    await dir?.removeEntry(encode(name));
  } catch {
    // Already gone.
  }
}

export async function clearStored(): Promise<void> {
  for (const { name } of await listStored()) await deleteStored(name);
}
