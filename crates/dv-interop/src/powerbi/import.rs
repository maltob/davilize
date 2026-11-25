//! Builds a Davilize report from a Power BI model and its report pages.

use std::io::Read;
use std::path::{Path, PathBuf};

use dv_core::{
    Aggregate, CalculatedField, ChartType, Dashboard, DashboardItem, Dataset, Encodings,
    FileFormat, Filter, FilterOp, FilterScope, Join, JoinKey, JoinKind, Measure, Report, Sort,
    SortDirection, Source, SourceKind, Visual,
};
use serde_json::Value;

use super::layout::{pages_from_legacy, visual_from_pbir};
use super::model::{parse_schema_json, parse_tmdl_relationships, parse_tmdl_table};
use super::{
    FieldRef, Model, ModelColumn, ModelRelationship, ModelTable, PageSpec, VisualSpec, decode_text,
};
use crate::expr;
use crate::util::{UniqueNames, fit_items, sanitize, to_grid};
use crate::{Format, ImportOptions, Imported, InteropError, io_err};

const MAX_PART_BYTES: u64 = 512 << 20;

struct Loaded {
    model: Option<Model>,
    pages: Vec<PageSpec>,
    /// Folder that relative data paths are resolved against.
    base: PathBuf,
}

pub(crate) fn import(
    path: &Path,
    format: Format,
    out_dir: &Path,
    options: &ImportOptions<'_>,
) -> Result<Imported, InteropError> {
    let mut warnings = Vec::new();
    let name = path.file_stem().map_or_else(
        || "report".to_string(),
        |s| s.to_string_lossy().into_owned(),
    );
    let loaded = match format {
        Format::Pbip => read_project(path, &mut warnings)?,
        Format::Pbit => read_archive(path, true, &mut warnings)?,
        _ => read_archive(path, false, &mut warnings)?,
    };
    let mut report = Report::new(name);
    build(&mut report, loaded, out_dir, options, &mut warnings);
    Ok(Imported {
        report,
        warnings,
        files: Vec::new(),
    })
}

// -------------------------------------------------------------- reading

fn read_json(path: &Path) -> Result<Value, InteropError> {
    let bytes = crate::fsx::read(path).map_err(io_err(path))?;
    serde_json::from_str(&decode_text(&bytes))
        .map_err(|e| InteropError::Format(format!("{}: {e}", path.display())))
}

fn subdirs_ending(dir: &Path, suffix: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = crate::fsx::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            crate::fsx::is_dir(p)
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().ends_with(suffix))
        })
        .collect();
    found.sort();
    found
}

fn read_project(path: &Path, warnings: &mut Vec<String>) -> Result<Loaded, InteropError> {
    let mut root = if crate::fsx::is_dir(path) {
        path.to_path_buf()
    } else {
        path.parent().map(Path::to_path_buf).unwrap_or_default()
    };
    if root.file_name().is_some_and(|n| {
        let n = n.to_string_lossy();
        n.ends_with(".Report") || n.ends_with(".SemanticModel")
    }) {
        root = root.parent().map(Path::to_path_buf).unwrap_or(root);
    }

    let mut model = None;
    if let Some(dir) = subdirs_ending(&root, ".SemanticModel").first() {
        model = read_semantic_model(dir, warnings)?;
    }
    let mut pages = Vec::new();
    match subdirs_ending(&root, ".Report").first() {
        Some(dir) => pages = read_report_folder(dir, warnings)?,
        None => warnings.push("no .Report folder was found in the project".into()),
    }
    if model.is_none() && pages.is_empty() {
        return Err(InteropError::Format(format!(
            "{}: not a Power BI project (no .SemanticModel or .Report folder)",
            path.display()
        )));
    }
    Ok(Loaded {
        model,
        pages,
        base: root,
    })
}

fn read_semantic_model(
    dir: &Path,
    warnings: &mut Vec<String>,
) -> Result<Option<Model>, InteropError> {
    let definition = dir.join("definition");
    if crate::fsx::is_dir(&definition) {
        let mut model = Model::default();
        let tables_dir = definition.join("tables");
        let mut files: Vec<PathBuf> = crate::fsx::read_dir(&tables_dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "tmdl"))
            .collect();
        files.sort();
        for file in files {
            let text = crate::fsx::read_to_string(&file).map_err(io_err(&file))?;
            match parse_tmdl_table(&text) {
                Some(t) => model.tables.push(t),
                None => warnings.push(format!("{} is not a table definition", file.display())),
            }
        }
        let rel = definition.join("relationships.tmdl");
        if let Ok(text) = crate::fsx::read_to_string(&rel) {
            model.relationships = parse_tmdl_relationships(&text);
        }
        return Ok(Some(model));
    }
    let bim = dir.join("model.bim");
    if crate::fsx::is_file(&bim) {
        return Ok(parse_schema_json(&read_json(&bim)?));
    }
    warnings.push(format!(
        "{} has neither a definition folder nor model.bim",
        dir.display()
    ));
    Ok(None)
}

fn read_report_folder(
    dir: &Path,
    warnings: &mut Vec<String>,
) -> Result<Vec<PageSpec>, InteropError> {
    let pages_dir = dir.join("definition").join("pages");
    if crate::fsx::is_dir(&pages_dir) {
        let order: Vec<String> = read_json(&pages_dir.join("pages.json"))
            .ok()
            .and_then(|v| {
                v.get("pageOrder").and_then(Value::as_array).map(|a| {
                    a.iter()
                        .filter_map(|n| n.as_str().map(str::to_string))
                        .collect()
                })
            })
            .unwrap_or_default();
        let mut names: Vec<String> = crate::fsx::read_dir(&pages_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| crate::fsx::is_dir(&e.path()))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort_by_key(|n| order.iter().position(|o| o == n).unwrap_or(usize::MAX));
        let mut pages = Vec::new();
        for name in names {
            let page_dir = pages_dir.join(&name);
            let page_json = read_json(&page_dir.join("page.json")).unwrap_or(Value::Null);
            let mut visuals = Vec::new();
            let visuals_dir = page_dir.join("visuals");
            let mut vdirs: Vec<PathBuf> = crate::fsx::read_dir(&visuals_dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| crate::fsx::is_dir(p))
                .collect();
            vdirs.sort();
            for vdir in vdirs {
                let file = vdir.join("visual.json");
                if !crate::fsx::is_file(&file) {
                    continue;
                }
                let json = read_json(&file)?;
                match visual_from_pbir(&json, warnings) {
                    Some(v) => visuals.push(v),
                    None => warnings.push(format!(
                        "{} is not a data visual and was skipped",
                        vdir.file_name().unwrap_or_default().to_string_lossy()
                    )),
                }
            }
            pages.push(PageSpec {
                display_name: page_json
                    .get("displayName")
                    .and_then(Value::as_str)
                    .unwrap_or(&name)
                    .to_string(),
                width: page_json
                    .get("width")
                    .and_then(Value::as_f64)
                    .unwrap_or(1280.0),
                height: page_json
                    .get("height")
                    .and_then(Value::as_f64)
                    .unwrap_or(720.0),
                name,
                visuals,
            });
        }
        return Ok(pages);
    }
    let legacy = dir.join("report.json");
    if crate::fsx::is_file(&legacy) {
        return Ok(pages_from_legacy(&read_json(&legacy)?, warnings));
    }
    warnings.push(format!("{} has no report definition", dir.display()));
    Ok(Vec::new())
}

fn read_archive(
    path: &Path,
    has_model: bool,
    warnings: &mut Vec<String>,
) -> Result<Loaded, InteropError> {
    let file = crate::fsx::open(path).map_err(io_err(path))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| InteropError::Archive(format!("{}: {e}", path.display())))?;
    let mut part = |name: &str| -> Option<String> {
        let mut entry = archive.by_name(name).ok()?;
        let mut bytes = Vec::new();
        entry
            .by_ref()
            .take(MAX_PART_BYTES)
            .read_to_end(&mut bytes)
            .ok()?;
        Some(decode_text(&bytes))
    };
    let layout_text = part("Report/Layout").ok_or_else(|| {
        InteropError::Archive(format!("{}: no Report/Layout part inside", path.display()))
    })?;
    let layout: Value = serde_json::from_str(&layout_text)
        .map_err(|e| InteropError::Format(format!("{}: Report/Layout: {e}", path.display())))?;
    let pages = pages_from_legacy(&layout, warnings);

    let mut model = None;
    if has_model {
        match part("DataModelSchema") {
            Some(text) => {
                model = serde_json::from_str::<Value>(&text)
                    .ok()
                    .and_then(|v| parse_schema_json(&v));
                if model.is_none() {
                    warnings.push("the template's DataModelSchema could not be read".into());
                }
            }
            None => warnings.push("the template has no DataModelSchema".into()),
        }
    } else {
        match read_datamodel(path, warnings) {
            Some(m) => model = Some(m),
            None => warnings.push(
                "the .pbix data model could not be read; only the report pages were imported, and the data sources are placeholders you must point at your data".into(),
            ),
        }
    }
    Ok(Loaded {
        model,
        pages,
        base: path.parent().map(Path::to_path_buf).unwrap_or_default(),
    })
}

/// Reads the compressed `DataModel` of a `.pbix` (an XPress9 ABF backup).
#[cfg(feature = "pbix")]
fn read_datamodel(path: &Path, warnings: &mut Vec<String>) -> Option<Model> {
    use ripbi_core::{ColumnKind, PartitionSource};
    let ingested = ripbi_core::ingest::semantic_model(path)
        .map_err(|e| warnings.push(format!("the .pbix data model could not be decoded: {e}")))
        .ok()?;
    let db = ingested.value;
    let mut model = Model::default();
    for table in &db.tables {
        if table.is_local_date_table || table.is_template_date_table || table.is_private {
            continue;
        }
        if table.is_calculated() {
            warnings.push(format!(
                "table '{}' is a DAX calculated table and was not imported; its columns will not resolve",
                table.name
            ));
            continue;
        }
        let mut out = ModelTable {
            name: table.name.clone(),
            ..ModelTable::default()
        };
        for column in &table.columns {
            let expression = match &column.kind {
                ColumnKind::Data => None,
                ColumnKind::Calculated { expression } => Some(expression.clone()),
                ColumnKind::CalculatedTableColumn => continue,
            };
            out.columns.push(ModelColumn {
                name: column.name.clone(),
                data_type: "string".into(),
                expression,
            });
        }
        for measure in &table.measures {
            out.measures
                .push((measure.name.clone(), measure.expression.clone()));
        }
        for partition in &table.partitions {
            if let PartitionSource::M { expression } = &partition.source
                && out.csv_path.is_none()
            {
                out.csv_path = super::model::csv_path_from_query(expression);
            }
        }
        model.tables.push(out);
    }
    for rel in db.relationships.iter().filter(|r| r.is_active) {
        let known = |t: &str| model.tables.iter().any(|m| m.name == t);
        if known(&rel.from_table) && known(&rel.to_table) {
            model.relationships.push(ModelRelationship {
                from_table: rel.from_table.clone(),
                from_column: rel.from_column.clone(),
                to_table: rel.to_table.clone(),
                to_column: rel.to_column.clone(),
            });
        }
    }
    Some(model)
}

/// Without the `pbix` feature the compressed model cannot be decoded.
#[cfg(not(feature = "pbix"))]
fn read_datamodel(_path: &Path, warnings: &mut Vec<String>) -> Option<Model> {
    warnings.push(
        "the .pbix data model is compressed; reading it needs the desktop app or the CLI".into(),
    );
    None
}

// ------------------------------------------------------------- building

/// The column names in the first line of a CSV file, when it can be read.
fn csv_header(path: &Path) -> Vec<String> {
    use std::io::{BufRead, BufReader};
    let Ok(file) = crate::fsx::open(path) else {
        return Vec::new();
    };
    let mut line = String::new();
    if BufReader::new(file).read_line(&mut line).is_err() {
        return Vec::new();
    }
    line.trim_start_matches('\u{feff}')
        .trim_end()
        .split(',')
        .map(|c| c.trim().trim_matches('"').to_string())
        .filter(|c| !c.is_empty())
        .collect()
}

/// The table of a visual that holds its aggregated fields: the "fact" side of its query.
fn fact_table(spec: &VisualSpec) -> Option<&str> {
    spec.roles
        .iter()
        .find(|(_, f)| !matches!(f, FieldRef::Column { .. }))
        .or_else(|| spec.roles.first())
        .map(|(_, f)| f.table())
}

/// A model derived from what the pages use, when the file has no readable model.
///
/// A `.pbix` does not say how its tables relate, so relationships are guessed:
/// a CSV named after each table, next to the file, tells us its columns, and a
/// column name shared by a visual's fact table and another table it uses is
/// taken as the join key.
fn model_from_pages(pages: &[PageSpec], base: &Path) -> Model {
    let mut model = Model::default();
    for visual in pages.iter().flat_map(|p| &p.visuals) {
        for (_, field) in &visual.roles {
            let table = match model.tables.iter_mut().find(|t| t.name == field.table()) {
                Some(t) => t,
                None => {
                    model.tables.push(ModelTable {
                        name: field.table().to_string(),
                        ..ModelTable::default()
                    });
                    model.tables.last_mut().unwrap()
                }
            };
            let column = match field {
                FieldRef::Column { column, .. } | FieldRef::Agg { column, .. } => column,
                FieldRef::Measure { .. } => continue,
            };
            if !table.columns.iter().any(|c| &c.name == column) {
                table.columns.push(ModelColumn {
                    name: column.clone(),
                    data_type: "string".into(),
                    expression: None,
                });
            }
        }
    }

    for table in &mut model.tables {
        let file = base.join(format!("{}.csv", table.name));
        if crate::fsx::is_file(&file) {
            for name in csv_header(&file) {
                if !table.columns.iter().any(|c| c.name == name) {
                    table.columns.push(ModelColumn {
                        name,
                        data_type: "string".into(),
                        expression: None,
                    });
                }
            }
            table.csv_path = Some(format!("{}.csv", table.name));
        }
    }

    for visual in pages.iter().flat_map(|p| &p.visuals) {
        let Some(fact) = fact_table(visual) else {
            continue;
        };
        let mut others: Vec<&str> = Vec::new();
        for (_, f) in &visual.roles {
            if f.table() != fact && !others.contains(&f.table()) {
                others.push(f.table());
            }
        }
        for other in others {
            if model
                .relationships
                .iter()
                .any(|r| r.from_table == fact && r.to_table == other)
            {
                continue;
            }
            let columns_of = |name: &str| -> Vec<String> {
                model
                    .tables
                    .iter()
                    .find(|t| t.name == name)
                    .map(|t| t.columns.iter().map(|c| c.name.clone()).collect())
                    .unwrap_or_default()
            };
            let theirs = columns_of(other);
            let shared = columns_of(fact).into_iter().find_map(|c| {
                theirs
                    .iter()
                    .find(|o| o.eq_ignore_ascii_case(&c))
                    .map(|o| (c.clone(), o.clone()))
            });
            if let Some((from_column, to_column)) = shared {
                model.relationships.push(ModelRelationship {
                    from_table: fact.to_string(),
                    from_column,
                    to_table: other.to_string(),
                    to_column,
                });
            }
        }
    }
    model
}

struct TableInfo {
    dataset: String,
    source: String,
}

fn data_path(raw: &str, base: &Path, out_dir: &Path) -> (String, bool) {
    let normalised = raw.replace('\\', "/");
    let p = PathBuf::from(&normalised);
    let absolute = if p.is_absolute() { p } else { base.join(p) };
    let exists = crate::fsx::exists(&absolute);
    let shown = match absolute.strip_prefix(out_dir) {
        Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
        Err(_) => absolute.to_string_lossy().replace('\\', "/"),
    };
    (shown, exists)
}

fn build(
    report: &mut Report,
    loaded: Loaded,
    out_dir: &Path,
    options: &ImportOptions<'_>,
    warnings: &mut Vec<String>,
) {
    let Loaded { model, pages, base } = loaded;
    let model = match model {
        Some(m) if !m.tables.is_empty() => m,
        _ => {
            if !pages.is_empty() {
                warnings.push("the file has no readable model; tables were derived from the fields the visuals use".into());
            }
            model_from_pages(&pages, &base)
        }
    };

    // One source and one dataset per table.
    let mut names = UniqueNames::default();
    let mut infos: Vec<TableInfo> = Vec::new();
    for table in &model.tables {
        let source = names.claim(&format!("{}_csv", table.name));
        let dataset = names.claim(&table.name);
        let path = match &table.csv_path {
            Some(p) => {
                let (shown, exists) = data_path(p, &base, out_dir);
                if !exists {
                    warnings.push(format!(
                        "the data file of table '{}' was not found at {shown}",
                        table.name
                    ));
                }
                shown
            }
            None => {
                warnings.push(format!(
                    "table '{}' is not loaded from a CSV file; its source points to {}.csv, which you need to provide",
                    table.name,
                    sanitize(&table.name)
                ));
                format!("{}.csv", sanitize(&table.name))
            }
        };
        report.sources.push(Source {
            name: source.clone(),
            kind: SourceKind::File {
                path: PathBuf::from(path),
                format: FileFormat::Csv,
            },
        });
        infos.push(TableInfo { dataset, source });
    }

    // Datasets: calculated columns, plus a left join for each relationship the table owns.
    for (table, info) in model.tables.iter().zip(&infos) {
        let mut dataset = Dataset {
            name: info.dataset.clone(),
            source: info.source.clone(),
            sql: None,
            joins: Vec::new(),
            calculated_fields: Vec::new(),
        };
        let mut field_names = UniqueNames::default();
        for column in table.columns.iter().filter(|c| c.expression.is_some()) {
            let expression = column.expression.as_deref().unwrap_or("");
            let field = field_names.claim(&column.name);
            let result =
                expr::dax_to_duckdb(expression).and_then(|text| match options.check_expression {
                    Some(check) => check(&field, &text).map(|()| text),
                    None => Ok(text),
                });
            match result {
                Ok(text) => dataset.calculated_fields.push(CalculatedField {
                    name: field,
                    expression: text,
                    description: Some(format!("Imported from Power BI: {}", expression.trim())),
                }),
                Err(reason) => warnings.push(format!(
                    "calculated column '{}' in table '{}' was not imported: {reason}",
                    column.name, table.name
                )),
            }
        }
        for rel in model
            .relationships
            .iter()
            .filter(|r| r.from_table == table.name)
        {
            let Some(target) = model.tables.iter().position(|t| t.name == rel.to_table) else {
                continue;
            };
            dataset.joins.push(Join {
                with: infos[target].source.clone(),
                kind: JoinKind::Left,
                on: vec![JoinKey {
                    left: rel.from_column.clone(),
                    right: rel.to_column.clone(),
                }],
            });
        }
        report.datasets.push(dataset);
    }

    // Visuals and dashboards.
    let mut visual_ids = UniqueNames::default();
    let mut dashboard_ids = UniqueNames::default();
    for page in &pages {
        let mut dashboard =
            Dashboard::new(dashboard_ids.claim(&page.display_name), &page.display_name);
        let rows = ((page.height / f64::from(dashboard.row_height)).round() as u32).max(1);
        for spec in &page.visuals {
            let Some(visual) = import_visual(spec, &model, &infos, &mut visual_ids, warnings)
            else {
                continue;
            };
            dashboard.items.push(DashboardItem {
                visual: visual.id.clone(),
                x: to_grid(spec.x, page.width, dashboard.columns),
                y: to_grid(spec.y, page.height, rows),
                w: to_grid(spec.width, page.width, dashboard.columns).max(1),
                h: to_grid(spec.height, page.height, rows).max(1),
            });
            report.visuals.push(visual);
        }
        if !dashboard.items.is_empty() {
            fit_items(&mut dashboard);
            report.dashboards.push(dashboard);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Bar { horizontal: bool },
    Line,
    Scatter,
    Table,
    Matrix,
}

fn visual_kind(visual_type: &str, title: &str, warnings: &mut Vec<String>) -> Option<Kind> {
    Some(match visual_type {
        "clusteredColumnChart" | "stackedColumnChart" | "hundredPercentStackedColumnChart" => {
            Kind::Bar { horizontal: false }
        }
        "barChart" | "clusteredBarChart" | "stackedBarChart" | "hundredPercentStackedBarChart" => {
            Kind::Bar { horizontal: true }
        }
        "lineChart" => Kind::Line,
        "areaChart"
        | "stackedAreaChart"
        | "lineClusteredColumnComboChart"
        | "lineStackedColumnComboChart" => {
            warnings.push(format!(
                "visual '{title}' ({visual_type}) was imported as a line chart"
            ));
            Kind::Line
        }
        "pieChart" | "donutChart" | "funnel" | "treemap" => {
            warnings.push(format!(
                "visual '{title}' ({visual_type}) was imported as a bar chart"
            ));
            Kind::Bar { horizontal: false }
        }
        "scatterChart" => Kind::Scatter,
        "tableEx" | "table" => Kind::Table,
        "pivotTable" => Kind::Matrix,
        other => {
            warnings.push(format!(
                "visual '{title}' is a {other}, which has no equivalent, and was skipped"
            ));
            return None;
        }
    })
}

struct VisualBuilder<'a> {
    dims: Vec<String>,
    measures: Vec<Measure>,
    model: &'a Model,
    warnings: &'a mut Vec<String>,
    title: String,
}

impl VisualBuilder<'_> {
    /// Registers a field and returns the name channels should use for it.
    fn add(&mut self, field: &FieldRef) -> Option<String> {
        match field {
            FieldRef::Column { column, .. } => {
                if !self.dims.contains(column) {
                    self.dims.push(column.clone());
                }
                Some(column.clone())
            }
            FieldRef::Agg { column, agg, .. } => Some(self.add_measure(*agg, column, None)),
            FieldRef::Measure { table, name } => {
                let expr = self
                    .model
                    .tables
                    .iter()
                    .find(|t| &t.name == table)
                    .and_then(|t| t.measures.iter().find(|(n, _)| n == name))
                    .map(|(_, e)| e.as_str());
                match expr.and_then(expr::simple_dax_aggregate) {
                    Some((agg, column)) => {
                        Some(self.add_measure(agg, &column, Some(sanitize(name))))
                    }
                    None => {
                        self.warnings.push(format!(
                            "visual '{}': measure '{name}' is not a simple aggregate and was skipped",
                            self.title
                        ));
                        None
                    }
                }
            }
        }
    }

    fn add_measure(&mut self, agg: Aggregate, column: &str, alias: Option<String>) -> String {
        let default = format!("{}_{}", agg.as_str(), column);
        let alias = alias.or_else(|| {
            let safe = sanitize(&default);
            (safe != default).then_some(safe)
        });
        let measure = Measure {
            field: column.to_string(),
            agg,
            alias,
        };
        let out = measure.output_name();
        if !self.measures.iter().any(|m| m.output_name() == out) {
            self.measures.push(measure);
        }
        out
    }
}

fn import_visual(
    spec: &VisualSpec,
    model: &Model,
    infos: &[TableInfo],
    ids: &mut UniqueNames,
    warnings: &mut Vec<String>,
) -> Option<Visual> {
    let title = spec.title.clone().unwrap_or_else(|| spec.name.clone());
    let kind = visual_kind(&spec.visual_type, &title, warnings)?;

    // The dataset is the table that reaches all the others through its relationships.
    let mut used: Vec<&str> = Vec::new();
    for (_, f) in &spec.roles {
        if !used.contains(&f.table()) {
            used.push(f.table());
        }
    }
    let reaches = |t: &str| {
        used.iter().all(|u| {
            *u == t
                || model
                    .relationships
                    .iter()
                    .any(|r| r.from_table == t && r.to_table == *u)
        })
    };
    let root = used.iter().copied().find(|t| reaches(t)).or_else(|| {
        let fact = fact_table(spec);
        warnings.push(format!(
            "visual '{title}' uses tables that are not related through a relationship; it is built on '{}' and only that table's columns will resolve until you add a join to the other tables",
            fact.unwrap_or_default()
        ));
        fact
    })?;
    let table_index = model.tables.iter().position(|t| t.name == root)?;
    let dataset = infos[table_index].dataset.clone();

    let mut b = VisualBuilder {
        dims: Vec::new(),
        measures: Vec::new(),
        model,
        warnings,
        title: title.clone(),
    };
    let mut enc = Encodings::default();
    let first = |b: &mut VisualBuilder<'_>, roles: &[&str]| -> Option<String> {
        let mut found = None;
        for (role, field) in &spec.roles {
            if roles.contains(&role.as_str()) {
                let out = b.add(field);
                if found.is_none() {
                    found = out;
                }
            }
        }
        found
    };
    let chart = match kind {
        Kind::Bar { horizontal } => {
            let category = first(&mut b, &["Category"]);
            let value = first(&mut b, &["Y", "Values"]);
            enc.color = first(&mut b, &["Series"]);
            if horizontal {
                enc.x = value;
                enc.y = category;
            } else {
                enc.x = category;
                enc.y = value;
            }
            ChartType::Bar
        }
        Kind::Line => {
            enc.x = first(&mut b, &["Category"]);
            enc.y = first(&mut b, &["Y", "Values"]);
            enc.color = first(&mut b, &["Series"]);
            ChartType::Line
        }
        Kind::Scatter => {
            enc.x = first(&mut b, &["X"]);
            enc.y = first(&mut b, &["Y"]);
            enc.size = first(&mut b, &["Size"]);
            enc.color = first(&mut b, &["Series"]);
            first(&mut b, &["Category"]);
            ChartType::Scatter
        }
        Kind::Table => {
            first(&mut b, &["Values"]);
            ChartType::Table
        }
        Kind::Matrix => {
            let rows = first(&mut b, &["Rows"]);
            let cols = first(&mut b, &["Columns"]);
            let value = first(&mut b, &["Values"]);
            if rows.is_some() && cols.is_some() && value.is_some() {
                enc.x = cols;
                enc.y = rows;
                enc.color = value;
                ChartType::Heatmap
            } else {
                ChartType::Table
            }
        }
    };
    // Anything in a role we did not map still has to be queried.
    for (_, field) in &spec.roles {
        b.add(field);
    }

    let mut filters = Vec::new();
    for f in &spec.filters {
        let (field, scope) = match &f.field {
            FieldRef::Column { column, .. } => (column.clone(), FilterScope::Row),
            other => match b.add(other) {
                Some(out) => (out, FilterScope::Aggregate),
                None => continue,
            },
        };
        if matches!(f.op, FilterOp::Between) && f.values.len() != 2 {
            continue;
        }
        filters.push(Filter {
            field,
            op: f.op,
            values: f.values.clone(),
            scope,
        });
    }
    let sort = spec.sort.as_ref().and_then(|(field, desc)| {
        let by = match field {
            FieldRef::Column { column, .. } => column.clone(),
            other => b.add(other)?,
        };
        Some(Sort {
            by,
            direction: if *desc {
                SortDirection::Desc
            } else {
                SortDirection::Asc
            },
        })
    });

    if b.dims.is_empty() && b.measures.is_empty() {
        warnings.push(format!("visual '{title}' has no fields and was skipped"));
        return None;
    }
    let (dimensions, measures) = (b.dims, b.measures);
    Some(Visual {
        id: ids.claim(&title),
        title,
        dataset,
        chart,
        encodings: if chart == ChartType::Table {
            Encodings::default()
        } else {
            enc
        },
        dimensions,
        measures,
        filters,
        sort,
        limit: None,
    })
}
