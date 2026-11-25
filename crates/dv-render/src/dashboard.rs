//! Builds the grid of every dashboard in a report, for the HTML page.
//!
//! Engine-independent: the caller supplies how to read a dataset's columns and how
//! to run one placed visual, so the CLI and the desktop app share the placement
//! rules (parameters bound, shared filters added per visual) and differ only in
//! which engine they run queries on.

use dv_core::{CoreError, ParamValues, Report, Visual, apply_dashboard_filters, bind_visual};

use crate::Rendered;
use crate::html::{ChartPlacement, DashboardView};

/// A dashboard with each placed visual already bound and filtered, not yet run.
#[derive(Debug, Clone)]
pub struct PlannedDashboard {
    pub id: String,
    pub title: String,
    pub columns: u32,
    pub row_height: u32,
    /// Placements: grid position and the visual to run.
    pub charts: Vec<(u32, u32, u32, u32, Visual)>,
}

/// Binds and filters every placed visual of every dashboard, without running anything.
///
/// The visual's parameters are bound from `values`, and the dashboard's shared filters are
/// added when the visual's dataset has the filtered column (`columns_of` gives those column
/// names). A visual on two dashboards appears twice, once per placement. `core_error`
/// turns core errors (a bad parameter, an unknown visual) into the caller's error type.
pub fn dashboard_plan<E>(
    report: &Report,
    values: &ParamValues,
    mut columns_of: impl FnMut(&str) -> Result<Vec<String>, E>,
    core_error: impl Fn(CoreError) -> E,
) -> Result<Vec<PlannedDashboard>, E> {
    let mut planned = Vec::new();
    for dashboard in &report.dashboards {
        let mut charts = Vec::new();
        for item in &dashboard.items {
            let visual = report
                .visuals
                .iter()
                .find(|v| v.id == item.visual)
                .ok_or_else(|| {
                    core_error(CoreError::Invalid(format!(
                        "dashboard '{}' places unknown visual '{}'",
                        dashboard.id, item.visual
                    )))
                })?;
            let bound = bind_visual(visual, values).map_err(&core_error)?;
            let columns = columns_of(&visual.dataset)?;
            let shared = apply_dashboard_filters(dashboard, &bound, &columns, values)
                .map_err(&core_error)?;
            charts.push((item.x, item.y, item.w, item.h, shared));
        }
        planned.push(PlannedDashboard {
            id: dashboard.id.clone(),
            title: dashboard.title.clone(),
            columns: dashboard.columns,
            row_height: dashboard.row_height,
            charts,
        });
    }
    Ok(planned)
}

/// Returns one [`DashboardView`] per dashboard, in report order: [`dashboard_plan`], then
/// `render` runs each planned visual.
pub fn dashboard_views<E>(
    report: &Report,
    values: &ParamValues,
    columns_of: impl FnMut(&str) -> Result<Vec<String>, E>,
    mut render: impl FnMut(&Visual) -> Result<Rendered, E>,
    core_error: impl Fn(CoreError) -> E,
) -> Result<Vec<DashboardView>, E> {
    let mut views = Vec::new();
    for dashboard in dashboard_plan(report, values, columns_of, core_error)? {
        let mut charts = Vec::new();
        for (x, y, w, h, visual) in &dashboard.charts {
            charts.push(ChartPlacement {
                x: *x,
                y: *y,
                w: *w,
                h: *h,
                chart: render(visual)?,
            });
        }
        views.push(DashboardView {
            id: dashboard.id,
            title: dashboard.title,
            columns: dashboard.columns,
            row_height: dashboard.row_height,
            charts,
        });
    }
    Ok(views)
}
