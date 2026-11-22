/**
 * Undo/redo history for report edits. Pure: every function returns a new value,
 * and nothing here touches React, so it is tested with node:test alone.
 *
 * Each change records the state it replaced under a label such as "Rename report".
 * A push with the same non-null key as the previous push, and no more than
 * COALESCE_MS after it, is merged into that step, so typing a title is one undo.
 */

/** Most undo steps kept. Older steps are dropped. */
export const HISTORY_LIMIT = 100;
/** Pushes with the same key this close together form one step. */
export const COALESCE_MS = 800;

interface Step<T> {
  /** The state to return to. */
  value: T;
  /** What the change was, for "Undid: ..." messages. */
  label: string;
}

export interface History<T> {
  readonly present: T;
  /** Earlier states, oldest first. The last entry is the state before the latest change. */
  readonly past: readonly Step<T>[];
  /** Undone states, the next one to redo last. */
  readonly future: readonly Step<T>[];
  readonly lastKey: string | null;
  readonly lastAt: number;
}

export function createHistory<T>(present: T): History<T> {
  return { present, past: [], future: [], lastKey: null, lastAt: 0 };
}

/**
 * Records `next` as the new present state. Pushing the identical value is a no-op.
 * Any redo steps are discarded.
 */
export function push<T>(
  history: History<T>,
  next: T,
  label: string,
  key: string | null = null,
  now: number = Date.now(),
): History<T> {
  if (Object.is(next, history.present)) return history;
  const coalesce =
    key !== null && key === history.lastKey && history.past.length > 0 && now - history.lastAt <= COALESCE_MS;
  if (coalesce) {
    return { ...history, present: next, future: [], lastAt: now };
  }
  const past = [...history.past, { value: history.present, label }].slice(-HISTORY_LIMIT);
  return { present: next, past, future: [], lastKey: key, lastAt: now };
}

/** Replaces the present state without recording a step, for example after a save. */
export function replace<T>(history: History<T>, value: T): History<T> {
  return { ...history, present: value, lastKey: null, lastAt: 0 };
}

export function canUndo<T>(history: History<T>): boolean {
  return history.past.length > 0;
}

export function canRedo<T>(history: History<T>): boolean {
  return history.future.length > 0;
}

/** Label of the step that undo would revert, or null. */
export function undoLabel<T>(history: History<T>): string | null {
  return history.past[history.past.length - 1]?.label ?? null;
}

/** Label of the step that redo would reapply, or null. */
export function redoLabel<T>(history: History<T>): string | null {
  return history.future[history.future.length - 1]?.label ?? null;
}

export function undo<T>(history: History<T>): History<T> {
  const step = history.past[history.past.length - 1];
  if (!step) return history;
  return {
    present: step.value,
    past: history.past.slice(0, -1),
    future: [...history.future, { value: history.present, label: step.label }],
    lastKey: null,
    lastAt: 0,
  };
}

export function redo<T>(history: History<T>): History<T> {
  const step = history.future[history.future.length - 1];
  if (!step) return history;
  return {
    present: step.value,
    past: [...history.past, { value: history.present, label: step.label }],
    future: history.future.slice(0, -1),
    lastKey: null,
    lastAt: 0,
  };
}
