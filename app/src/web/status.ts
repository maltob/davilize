// Whether the browser data engine is still starting. Kept apart from duck.ts so the page can
// listen without importing the engine itself (the desktop build never ships it).

let starting = false;
const listeners = new Set<(starting: boolean) => void>();

/** Calls `listener` with the current state and on each change. Returns an unsubscribe function. */
export function onEngineStarting(listener: (starting: boolean) => void): () => void {
  listeners.add(listener);
  listener(starting);
  return () => {
    listeners.delete(listener);
  };
}

export function setEngineStarting(value: boolean): void {
  starting = value;
  listeners.forEach((listener) => listener(value));
}
