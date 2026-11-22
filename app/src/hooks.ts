import { useEffect, useState } from "react";
import { api, toApiError } from "./api";
import type { ApiError, Filter, Parameter, RenderedVisual, Visual } from "./types";
import { hasFields, requestValues } from "./model";

export interface RenderState {
  loading: boolean;
  result: RenderedVisual | null;
  error: ApiError | null;
}

const IDLE: RenderState = { loading: false, result: null, error: null };

/**
 * Runs a visual through the backend whenever it changes or a parameter value changes.
 * Results from older requests are ignored, and `delay` debounces fast edits to the shelves.
 * While `blocked` (a parameter value is invalid) nothing is sent; the reason is shown instead.
 * With `dashboardFilters`, the visual is run as placed on a dashboard (shared filters added).
 */
export function useRender(
  visual: Visual | null,
  delay: number,
  parameters: Parameter[],
  values: Record<string, string>,
  blocked: boolean,
  dashboardFilters: Filter[] | null = null,
): RenderState {
  const [state, setState] = useState<RenderState>(IDLE);
  const requested = requestValues(parameters, values);
  const valuesKey = JSON.stringify(requested);
  const parametersKey = JSON.stringify(parameters);
  const filtersKey = JSON.stringify(dashboardFilters);
  useEffect(() => {
    if (!visual || !hasFields(visual)) {
      setState(IDLE);
      return;
    }
    if (blocked) {
      setState({
        loading: false,
        result: null,
        error: { kind: "invalid", message: "Fix the parameter values above to run this visual." },
      });
      return;
    }
    let current = true;
    setState((prev) => ({ ...prev, loading: true }));
    const timer = window.setTimeout(() => {
      const request = dashboardFilters
        ? api.renderDashboardVisual(visual, dashboardFilters, parameters, requested)
        : api.renderVisual(visual, parameters, requested);
      request
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
    // `requested` and `parameters` are represented by their JSON keys, which keeps
    // the effect from re-running on every render when the objects are new but equal.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visual, delay, valuesKey, parametersKey, blocked, filtersKey]);
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
