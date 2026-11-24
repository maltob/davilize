//! Writes a Davilize report as a Power BI Project folder or a `.pbit` template.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;

use dv_core::{
    ChartType, Dashboard, Dataset, FileFormat, FilterScope, Measure, Report, SortDirection,
    SourceKind, Visual,
};
use serde_json::{Value, json};

use super::layout::{
    PAGE_SCHEMA, PAGES_SCHEMA, REPORT_SCHEMA, VERSION_SCHEMA, legacy_layout, visual_to_pbir,
};
use super::model::{
    csv_columns, csv_query, schema_json, tmdl_model, tmdl_relationships, tmdl_table,
};
use super::{
    FieldRef, FilterSpec, Model, ModelColumn, ModelRelationship, ModelTable, PageSpec, VisualSpec,
    encode_utf16le,
};
use crate::expr;
use crate::util::{dataset_chain, sanitize};
use crate::{Exported, InteropError, io_err};

const PAGE_WIDTH: f64 = 1280.0;
const PAGE_MIN_HEIGHT: f64 = 720.0;

struct Built {
    model: Model,
    queries: Vec<(String, Vec<String>)>,
    pages: Vec<PageSpec>,
    warnings: Vec<String>,
}

fn strip_verbatim(path: &Path) -> String {
    let text = path.to_string_lossy().into_owned();
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
}

struct Builder<'a> {
    report: &'a Report,
    report_dir: &'a Path,
    model: Model,
    queries: Vec<(String, Vec<String>)>,
    warnings: Vec<String>,
    /// Dataset name -> table name.
    dataset_tables: HashMap<String, String>,
    /// Source name -> table name, for tables created for join targets.
    source_tables: HashMap<String, String>,
}

impl Builder<'_> {
    fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }

    /// Loads a CSV source as table columns and a query.
    fn csv_source(&mut self, source: &str) -> Option<(Vec<ModelColumn>, Vec<String>)> {
        let src = self.report.sources.iter().find(|s| s.name == source)?;
        let SourceKind::File { path, format } = &src.kind else {
            self.warn(format!(
                "source '{source}' is a database; reports hold no connection details, so it was not exported"
            ));
            return None;
        };
        if *format != FileFormat::Csv {
            self.warn(format!(
                "source '{source}' is {format:?}; only CSV files are exported to Power BI"
            ));
            return None;
        }
        let absolute = if path.is_absolute() {
            path.clone()
        } else {
            self.report_dir.join(path)
        };
        let absolute = std::fs::canonicalize(&absolute).unwrap_or(absolute);
        let Some(columns) = csv_columns(&absolute) else {
            self.warn(format!(
                "the data file of source '{source}' ({}) could not be read for its columns",
                absolute.display()
            ));
            return None;
        };
        let query = csv_query(&strip_verbatim(&absolute), &columns);
        Some((columns, query))
    }

    fn add_dataset(&mut self, dataset: &Dataset) {
        let (chain, base) = match dataset_chain(self.report, dataset) {
            Ok(v) => v,
            Err(reason) => {
                self.warn(format!("{reason}, which cannot be exported to Power BI"));
                return;
            }
        };
        let Some((mut columns, query)) = self.csv_source(base) else {
            return;
        };
        let mut joins = Vec::new();
        for d in &chain {
            for field in &d.calculated_fields {
                match expr::duckdb_to_dax(&field.expression) {
                    Ok(dax) => columns.push(ModelColumn {
                        name: field.name.clone(),
                        data_type: "double".into(),
                        expression: Some(dax),
                    }),
                    Err(reason) => self.warn(format!(
                        "calculated field '{}' in '{}' was not exported: {reason}",
                        field.name, dataset.name
                    )),
                }
            }
            joins.extend(d.joins.iter());
        }
        let table_name = dataset.name.clone();
        self.model.tables.push(ModelTable {
            name: table_name.clone(),
            columns,
            ..ModelTable::default()
        });
        self.queries.push((table_name.clone(), query));
        self.dataset_tables
            .insert(dataset.name.clone(), table_name.clone());

        for join in joins {
            let with = match self.report.datasets.iter().find(|d| d.name == join.with) {
                Some(other) if other.sql.is_none() && other.joins.is_empty() => {
                    other.source.clone()
                }
                Some(_) => {
                    self.warn(format!(
                        "dataset '{}' joins the derived dataset '{}', which was not exported",
                        dataset.name, join.with
                    ));
                    continue;
                }
                None => join.with.clone(),
            };
            let target = match self.source_tables.get(&with) {
                Some(t) => t.clone(),
                None => {
                    let Some((cols, query)) = self.csv_source(&with) else {
                        continue;
                    };
                    let name = with.clone();
                    self.model.tables.push(ModelTable {
                        name: name.clone(),
                        columns: cols,
                        ..ModelTable::default()
                    });
                    self.queries.push((name.clone(), query));
                    self.source_tables.insert(with.clone(), name.clone());
                    name
                }
            };
            if join.on.len() > 1 {
                self.warn(format!(
                    "the join of '{}' with '{with}' has several keys; Power BI relationships have one, so only the first was kept",
                    dataset.name
                ));
            }
            let key = &join.on[0];
            self.model.relationships.push(ModelRelationship {
                from_table: table_name.clone(),
                from_column: key.left.clone(),
                to_table: target,
                to_column: key.right.clone(),
            });
            if !matches!(
                join.kind,
                dv_core::JoinKind::Left | dv_core::JoinKind::Inner
            ) {
                self.warn(format!(
                    "the {:?} join of '{}' became an ordinary many-to-one relationship",
                    join.kind, dataset.name
                ));
            }
        }
    }

    /// The table that holds a field: the dataset's table, else one of its join targets.
    fn field_table(&self, dataset: &str, field: &str) -> String {
        let root = self
            .dataset_tables
            .get(dataset)
            .cloned()
            .unwrap_or_else(|| dataset.to_string());
        let has = |table: &str| {
            self.model
                .tables
                .iter()
                .find(|t| t.name == table)
                .is_some_and(|t| t.columns.iter().any(|c| c.name == field))
        };
        if has(&root) {
            return root;
        }
        self.model
            .relationships
            .iter()
            .filter(|r| r.from_table == root)
            .map(|r| r.to_table.clone())
            .find(|t| has(t))
            .unwrap_or(root)
    }
}

fn param_default(report: &Report, value: &Value) -> Option<Option<Value>> {
    let name = dv_core::param_ref(value).ok().flatten()?;
    Some(
        report
            .parameters
            .iter()
            .find(|p| p.name == name)
            .and_then(|p| p.default.clone()),
    )
}

fn visual_spec(b: &mut Builder<'_>, visual: &Visual) -> Option<VisualSpec> {
    let dataset = visual.dataset.as_str();
    let measure_by_out: HashMap<String, &Measure> = visual
        .measures
        .iter()
        .map(|m| (m.output_name(), m))
        .collect();
    let field = |b: &Builder<'_>, name: &str| -> FieldRef {
        match measure_by_out.get(name) {
            Some(m) => FieldRef::Agg {
                table: b.field_table(dataset, &m.field),
                column: m.field.clone(),
                agg: m.agg,
            },
            None => FieldRef::Column {
                table: b.field_table(dataset, name),
                column: name.to_string(),
            },
        }
    };
    let is_measure =
        |n: &Option<String>| n.as_ref().is_some_and(|n| measure_by_out.contains_key(n));

    let enc = &visual.encodings;
    let mut roles: Vec<(String, FieldRef)> = Vec::new();
    let mut used: HashSet<String> = HashSet::new();
    let put = |b: &Builder<'_>,
               role: &str,
               name: &Option<String>,
               roles: &mut Vec<(String, FieldRef)>,
               used: &mut HashSet<String>| {
        if let Some(n) = name
            && used.insert(n.clone())
        {
            roles.push((role.to_string(), field(b, n)));
        }
    };

    let visual_type = match visual.chart {
        ChartType::Bar | ChartType::Line => {
            let horizontal = visual.chart == ChartType::Bar
                && is_measure(&enc.x)
                && enc.y.is_some()
                && !is_measure(&enc.y);
            let (category, value) = if horizontal {
                (enc.y.clone(), enc.x.clone())
            } else {
                (
                    enc.x
                        .clone()
                        .filter(|x| !measure_by_out.contains_key(x))
                        .or_else(|| visual.dimensions.first().cloned()),
                    enc.y
                        .clone()
                        .or_else(|| visual.measures.first().map(Measure::output_name)),
                )
            };
            put(b, "Category", &category, &mut roles, &mut used);
            if is_measure(&enc.color) {
                b.warn(format!(
                    "visual '{}': a measure on color has no Power BI equivalent and was added as a value",
                    visual.id
                ));
            } else {
                put(b, "Series", &enc.color, &mut roles, &mut used);
            }
            put(b, "Y", &value, &mut roles, &mut used);
            for d in &visual.dimensions {
                put(b, "Category", &Some(d.clone()), &mut roles, &mut used);
            }
            for m in &visual.measures {
                put(b, "Y", &Some(m.output_name()), &mut roles, &mut used);
            }
            match (visual.chart, horizontal) {
                (ChartType::Line, _) => "lineChart",
                (_, true) => "clusteredBarChart",
                _ => "clusteredColumnChart",
            }
        }
        ChartType::Scatter => {
            put(b, "X", &enc.x, &mut roles, &mut used);
            put(b, "Y", &enc.y, &mut roles, &mut used);
            put(b, "Size", &enc.size, &mut roles, &mut used);
            put(b, "Series", &enc.color, &mut roles, &mut used);
            for d in &visual.dimensions {
                put(b, "Category", &Some(d.clone()), &mut roles, &mut used);
            }
            for m in &visual.measures {
                put(b, "Y", &Some(m.output_name()), &mut roles, &mut used);
            }
            "scatterChart"
        }
        ChartType::Table => {
            for d in &visual.dimensions {
                put(b, "Values", &Some(d.clone()), &mut roles, &mut used);
            }
            for m in &visual.measures {
                put(b, "Values", &Some(m.output_name()), &mut roles, &mut used);
            }
            "tableEx"
        }
        ChartType::Heatmap => {
            b.warn(format!(
                "heatmap '{}' became a matrix; add conditional formatting in Power BI for the colors",
                visual.id
            ));
            put(b, "Rows", &enc.y, &mut roles, &mut used);
            put(b, "Columns", &enc.x, &mut roles, &mut used);
            put(b, "Values", &enc.color, &mut roles, &mut used);
            for d in &visual.dimensions {
                put(b, "Rows", &Some(d.clone()), &mut roles, &mut used);
            }
            for m in &visual.measures {
                put(b, "Values", &Some(m.output_name()), &mut roles, &mut used);
            }
            "pivotTable"
        }
    };
    if roles.is_empty() {
        b.warn(format!(
            "visual '{}' has no fields and was skipped",
            visual.id
        ));
        return None;
    }

    let mut filters = Vec::new();
    for f in &visual.filters {
        let mut values = Vec::new();
        let mut skip = false;
        for v in &f.values {
            match param_default(b.report, v) {
                Some(Some(default)) => values.push(default),
                Some(None) => skip = true,
                None => values.push(v.clone()),
            }
        }
        if skip {
            b.warn(format!(
                "the filter on '{}' in '{}' uses a parameter without a default and was skipped",
                f.field, visual.id
            ));
            continue;
        }
        if f.values
            .iter()
            .any(|v| dv_core::param_ref(v).ok().flatten().is_some())
        {
            b.warn(format!(
                "the parameter in the filter on '{}' in '{}' was replaced by its default value",
                f.field, visual.id
            ));
        }
        let field_ref = match f.scope {
            FilterScope::Row => FieldRef::Column {
                table: b.field_table(dataset, &f.field),
                column: f.field.clone(),
            },
            FilterScope::Aggregate => field(b, &f.field),
        };
        filters.push(FilterSpec {
            field: field_ref,
            op: f.op,
            values,
        });
    }
    if visual.limit.is_some() {
        b.warn(format!("the row limit of '{}' was not exported", visual.id));
    }
    let sort = visual
        .sort
        .as_ref()
        .map(|s| (field(b, &s.by), s.direction == SortDirection::Desc));

    Some(VisualSpec {
        name: sanitize(&visual.id),
        visual_type: visual_type.to_string(),
        title: Some(visual.title.clone()),
        roles,
        x: 0.0,
        y: 0.0,
        width: 400.0,
        height: 300.0,
        filters,
        sort,
    })
}

fn build(report: &Report, report_dir: &Path) -> Built {
    let mut b = Builder {
        report,
        report_dir,
        model: Model::default(),
        queries: Vec::new(),
        warnings: Vec::new(),
        dataset_tables: HashMap::new(),
        source_tables: HashMap::new(),
    };
    for dataset in &report.datasets {
        b.add_dataset(dataset);
    }
    if !report.parameters.is_empty() {
        b.warn("parameters are not exported as Power BI parameters; their defaults were substituted into filters");
    }

    let mut specs: HashMap<String, VisualSpec> = HashMap::new();
    for visual in &report.visuals {
        if !b.dataset_tables.contains_key(&visual.dataset) {
            b.warn(format!(
                "visual '{}' was skipped because its dataset could not be exported",
                visual.id
            ));
            continue;
        }
        if let Some(spec) = visual_spec(&mut b, visual) {
            specs.insert(visual.id.clone(), spec);
        }
    }

    let mut pages: Vec<PageSpec> = Vec::new();
    let mut placed: HashSet<String> = HashSet::new();
    for dashboard in &report.dashboards {
        pages.push(dashboard_page(&mut b, dashboard, &specs, &mut placed));
    }
    let leftover: Vec<&Visual> = report
        .visuals
        .iter()
        .filter(|v| specs.contains_key(&v.id) && !placed.contains(&v.id))
        .collect();
    if !leftover.is_empty() {
        let name = if pages.is_empty() {
            "Report"
        } else {
            "Other visuals"
        };
        let mut page = PageSpec {
            name: sanitize(name),
            display_name: name.to_string(),
            width: PAGE_WIDTH,
            height: PAGE_MIN_HEIGHT,
            visuals: Vec::new(),
        };
        for (i, v) in leftover.iter().enumerate() {
            let mut spec = specs[&v.id].clone();
            spec.x = 20.0 + (i % 2) as f64 * 620.0;
            spec.y = 20.0 + (i / 2) as f64 * 340.0;
            spec.width = 600.0;
            spec.height = 320.0;
            page.height = page.height.max(spec.y + spec.height + 20.0);
            page.visuals.push(spec);
        }
        pages.push(page);
    }

    Built {
        model: b.model,
        queries: b.queries,
        pages,
        warnings: b.warnings,
    }
}

fn dashboard_page(
    b: &mut Builder<'_>,
    dashboard: &Dashboard,
    specs: &HashMap<String, VisualSpec>,
    placed: &mut HashSet<String>,
) -> PageSpec {
    let rows = dashboard.items.iter().map(|i| i.y + i.h).max().unwrap_or(1);
    let cell_w = PAGE_WIDTH / f64::from(dashboard.columns.max(1));
    let cell_h = f64::from(dashboard.row_height);
    let mut page = PageSpec {
        name: sanitize(&dashboard.id),
        display_name: dashboard.title.clone(),
        width: PAGE_WIDTH,
        height: (f64::from(rows) * cell_h).max(PAGE_MIN_HEIGHT),
        visuals: Vec::new(),
    };
    for item in &dashboard.items {
        let Some(base) = specs.get(&item.visual) else {
            continue;
        };
        placed.insert(item.visual.clone());
        let mut spec = base.clone();
        spec.x = f64::from(item.x) * cell_w;
        spec.y = f64::from(item.y) * cell_h;
        spec.width = f64::from(item.w) * cell_w;
        spec.height = f64::from(item.h) * cell_h;
        let visual = b.report.visuals.iter().find(|v| v.id == item.visual);
        for f in &dashboard.filters {
            let Some(visual) = visual else { continue };
            let table = b.field_table(&visual.dataset, &f.field);
            let known = b
                .model
                .tables
                .iter()
                .find(|t| t.name == table)
                .is_some_and(|t| t.columns.iter().any(|c| c.name == f.field));
            if known
                && !f
                    .values
                    .iter()
                    .any(|v| dv_core::param_ref(v).ok().flatten().is_some())
            {
                spec.filters.push(FilterSpec {
                    field: FieldRef::Column {
                        table,
                        column: f.field.clone(),
                    },
                    op: f.op,
                    values: f.values.clone(),
                });
            }
        }
        page.visuals.push(spec);
    }
    page
}

// -------------------------------------------------------------- writing

fn write_text(path: &Path, text: &str) -> Result<(), InteropError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io_err(parent))?;
    }
    std::fs::write(path, text).map_err(io_err(path))
}

fn write_json(path: &Path, value: &Value) -> Result<(), InteropError> {
    let mut text =
        serde_json::to_string_pretty(value).map_err(|e| InteropError::Format(e.to_string()))?;
    text.push('\n');
    write_text(path, &text)
}

pub(crate) fn export_pbip(
    report: &Report,
    report_dir: &Path,
    out: &Path,
) -> Result<Exported, InteropError> {
    let built = build(report, report_dir);
    let name = sanitize(&report.name);
    std::fs::create_dir_all(out).map_err(io_err(out))?;

    write_json(
        &out.join(format!("{name}.pbip")),
        &json!({
            "version": "1.0",
            "artifacts": [{ "report": { "path": format!("{name}.Report") } }],
            "settings": { "enableAutoRecovery": true },
        }),
    )?;

    let model_dir = out.join(format!("{name}.SemanticModel"));
    write_json(
        &model_dir.join("definition.pbism"),
        &json!({ "version": "4.0", "settings": {} }),
    )?;
    let def = model_dir.join("definition");
    write_text(
        &def.join("database.tmdl"),
        "database\n\tcompatibilityLevel: 1600\n",
    )?;
    write_text(&def.join("model.tmdl"), &tmdl_model(&built.model))?;
    if !built.model.relationships.is_empty() {
        write_text(
            &def.join("relationships.tmdl"),
            &tmdl_relationships(&built.model),
        )?;
    }
    for table in &built.model.tables {
        let query = built
            .queries
            .iter()
            .find(|(t, _)| *t == table.name)
            .map(|(_, q)| q.as_slice());
        write_text(
            &def.join("tables")
                .join(format!("{}.tmdl", sanitize(&table.name))),
            &tmdl_table(table, query),
        )?;
    }

    let report_dir_out = out.join(format!("{name}.Report"));
    write_json(
        &report_dir_out.join("definition.pbir"),
        &json!({
            "version": "4.0",
            "datasetReference": { "byPath": { "path": format!("../{name}.SemanticModel") } },
        }),
    )?;
    let rdef = report_dir_out.join("definition");
    write_json(
        &rdef.join("version.json"),
        &json!({ "$schema": VERSION_SCHEMA, "version": "2.0.0" }),
    )?;
    write_json(
        &rdef.join("report.json"),
        &json!({
            "$schema": REPORT_SCHEMA,
            "themeCollection": { "baseTheme": {
                "name": "CY24SU10",
                "reportVersionAtImport": "5.59",
                "type": "SharedResources",
            } },
            "layoutOptimization": "None",
        }),
    )?;
    write_json(
        &rdef.join("pages").join("pages.json"),
        &json!({
            "$schema": PAGES_SCHEMA,
            "pageOrder": built.pages.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
            "activePageName": built.pages.first().map(|p| p.name.clone()),
        }),
    )?;
    for page in &built.pages {
        let page_dir = rdef.join("pages").join(&page.name);
        write_json(
            &page_dir.join("page.json"),
            &json!({
                "$schema": PAGE_SCHEMA,
                "name": page.name,
                "displayName": page.display_name,
                "displayOption": "FitToPage",
                "height": page.height,
                "width": page.width,
            }),
        )?;
        for (z, visual) in page.visuals.iter().enumerate() {
            write_json(
                &page_dir
                    .join("visuals")
                    .join(&visual.name)
                    .join("visual.json"),
                &visual_to_pbir(visual, z as u32),
            )?;
        }
    }
    Ok(Exported {
        files: vec![out.to_path_buf(), out.join(format!("{name}.pbip"))],
        warnings: built.warnings,
    })
}

pub(crate) fn export_pbit(
    report: &Report,
    report_dir: &Path,
    out: &Path,
) -> Result<Exported, InteropError> {
    let built = build(report, report_dir);
    let parts: Vec<(&str, String)> = vec![
        ("Version", "1.28".to_string()),
        (
            "DataModelSchema",
            serde_json::to_string(&schema_json(&built.model, &built.queries))
                .map_err(|e| InteropError::Format(e.to_string()))?,
        ),
        (
            "Report/Layout",
            serde_json::to_string(&legacy_layout(&built.pages))
                .map_err(|e| InteropError::Format(e.to_string()))?,
        ),
        (
            "Metadata",
            json!({ "Version": 5, "AutoCreatedRelationships": [], "FileDescription": report.name })
                .to_string(),
        ),
        (
            "Settings",
            json!({
                "Version": 4,
                "ReportSettings": {},
                "QueriesSettings": { "TypeDetectionEnabled": true, "RelationshipImportEnabled": true, "Version": "2.126.0" },
            })
            .to_string(),
        ),
    ];
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(io_err(parent))?;
    }
    let file = std::fs::File::create(out).map_err(io_err(out))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let archive_err = |e: zip::result::ZipError| InteropError::Archive(e.to_string());
    let mut content_types = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">",
    );
    for (name, _) in &parts {
        content_types.push_str(&format!(
            "<Override PartName=\"/{name}\" ContentType=\"\" />"
        ));
    }
    content_types.push_str("</Types>");
    zip.start_file("[Content_Types].xml", options)
        .map_err(archive_err)?;
    zip.write_all(content_types.as_bytes())
        .map_err(io_err(out))?;
    for (name, text) in &parts {
        zip.start_file(*name, options).map_err(archive_err)?;
        zip.write_all(&encode_utf16le(text)).map_err(io_err(out))?;
    }
    zip.finish().map_err(archive_err)?;
    Ok(Exported {
        files: vec![out.to_path_buf()],
        warnings: built.warnings,
    })
}
