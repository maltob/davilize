//! Builds the grid of every dashboard in a report, for the HTML page.
//!
//! Engine-independent: the caller supplies how to read a dataset's columns and how
//! to run one placed visual, so the CLI and the desktop app share the placement
//! rules (parameters bound, shared filters added per visual) and differ only in
//! which engine they run queries on.

use dv_core::{CoreError, ParamValues, Report, Visual, apply_dashboard_filters, bind_visual};

use crate::Rendered;
use crate::html::{ChartPlacement, DashboardView};

/// Returns one [`DashboardView`] per dashboard, in report order.
///
/// Each placement is bound and filtered on its own: the visual's parameters are
/// bound from `values`, the dashboard's shared filters are added when the visual's
/// dataset has the filtered column (`columns_of` gives those column names), and
/// `render` runs the result. A visual on two dashboards is rendered twice, once per
/// placement. `core_error` turns core errors (a bad parameter, an unknown visual)
/// into the caller's error type.
pub fn dashboard_views<E>(
    report: &Report,
    values: &ParamValues,
    mut columns_of: impl FnMut(&str) -> Result<Vec<String>, E>,
    mut render: impl FnMut(&Visual) -> Result<Rendered, E>,
    core_error: impl Fn(CoreError) -> E,
) -> Result<Vec<DashboardView>, E> {
    let mut views = Vec::new();
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
            let chart = render(&shared)?;
            charts.push(ChartPlacement {
                x: item.x,
                y: item.y,
                w: item.w,
                h: item.h,
                chart,
            });
        }
        views.push(DashboardView {
            id: dashboard.id.clone(),
            title: dashboard.title.clone(),
            columns: dashboard.columns,
            row_height: dashboard.row_height,
            charts,
        });
    }
    Ok(views)
}
