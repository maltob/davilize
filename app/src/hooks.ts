import { useEffect, useState } from "react";
import { api, toApiError } from "./api";
import type { ApiError, RenderedVisual, Visual } from "./types";
import { hasFields } from "./model";

export interface RenderState {
  loading: boolean;
  result: RenderedVisual | null;
  error: ApiError | null;
}

const IDLE: RenderState = { loading: false, result: null, error: null };

/**
 * Runs a visual through the backend whenever it changes. Results from older
 * requests are ignored, and `delay` debounces fast edits to the shelves.
 */
export function useRender(visual: Visual | null, delay: number): RenderState {
  const [state, setState] = useState<RenderState>(IDLE);
  useEffect(() => {
    if (!visual || !hasFields(visual)) {
      setState(IDLE);
      return;
    }
    let current = true;
    setState((prev) => ({ ...prev, loading: true }));
    const timer = window.setTimeout(() => {
      api
        .renderVisual(visual)
        .then((result) => {
          if (current) setState({ loading: false, result, error: null });
        })
        .catch((err: unknown) => {
          if (current) setState({ loading: false, result: null, error: toApiError(err) });
        });
    }, delay);
    return () => {
      current = false;
      window.clearTimeout(timer);
    };
  }, [visual, delay]);
  return state;
}

export interface TableState {
  loading: boolean;
  table: import("./types").QueryTable | null;
  error: ApiError | null;
}

/** Loads the preview or the profile of a dataset, ignoring stale responses. */
export function useDatasetTable(dataset: string | null, mode: "preview" | "profile"): TableState {
  const [state, setState] = useState<TableState>({ loading: false, table: null, error: null });
  useEffect(() => {
    if (!dataset) {
      setState({ loading: false, table: null, error: null });
      return;
    }
    let current = true;
    setState({ loading: true, table: null, error: null });
    const request = mode === "preview" ? api.previewDataset(dataset, 500) : api.profileDataset(dataset);
    request
      .then((table) => {
        if (current) setState({ loading: false, table, error: null });
      })
      .catch((err: unknown) => {
        if (current) setState({ loading: false, table: null, error: toApiError(err) });
      });
    return () => {
      current = false;
    };
  }, [dataset, mode]);
  return state;
}
