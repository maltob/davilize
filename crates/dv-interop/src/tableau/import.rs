//! Reads a Tableau workbook into a Davilize report.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use dv_core::{
    Aggregate, CalculatedField, ChartType, Dashboard, DashboardItem, Dataset, DbKind, Encodings,
    FileFormat, Filter, FilterOp, FilterScope, Join, JoinKey, JoinKind, Measure, ParamType,
    Parameter, Report, Sort, SortDirection, Source, SourceKind, Visual,
};
use roxmltree::{Document, Node};
use serde_json::Value;

use super::{bracket_parts, strip_brackets};
use crate::expr::{self, NameMap};
use crate::util::{UniqueNames, fit_items, sanitize, to_grid};
use crate::{ImportOptions, Imported, InteropError, io_err};

/// Largest single file unpacked from a `.twbx`.
const MAX_UNPACKED_BYTES: u64 = 4 << 30;

struct Ctx<'a> {
    out_dir: &'a Path,
    twb_dir: PathBuf,
    options: &'a ImportOptions<'a>,
    warnings: Vec<String>,
    files: Vec<PathBuf>,
    /// Files unpacked from a `.twbx`: (path inside the archive's `Data/`, path relative to `out_dir`).
    unpacked: Vec<(String, String)>,
    rel_names: UniqueNames,
    /// Absolute or relative file path -> source name, so shared files become one source.
    sources_by_path: HashMap<String, String>,
}

impl Ctx<'_> {
    fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }
}

/// A Tableau datasource that became a dataset.
struct Ds {
    tableau_name: String,
    dataset: String,
    /// Tableau column name -> Davilize field name, for calculated fields.
    names: NameMap,
    /// Calculations that are a plain aggregate: column -> (aggregate, field, alias).
    agg_calcs: HashMap<String, (Aggregate, String, String)>,
}

fn kids<'a, 'b>(node: Node<'a, 'b>, tag: &'static str) -> impl Iterator<Item = Node<'a, 'b>> {
    node.children()
        .filter(move |c| c.is_element() && c.tag_name().name() == tag)
}

pub(crate) fn import(
    path: &Path,
    out_dir: &Path,
    options: &ImportOptions<'_>,
) -> Result<Imported, InteropError> {
    let stem = path.file_stem().map_or_else(
        || "workbook".to_string(),
        |s| s.to_string_lossy().into_owned(),
    );
    let mut ctx = Ctx {
        out_dir,
        twb_dir: path.parent().map(Path::to_path_buf).unwrap_or_default(),
        options,
        warnings: Vec::new(),
        files: Vec::new(),
        unpacked: Vec::new(),
        rel_names: UniqueNames::default(),
        sources_by_path: HashMap::new(),
    };

    let is_packaged = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("twbx"));
    let xml = if is_packaged {
        read_twbx(path, out_dir, &stem, &mut ctx)?
    } else {
        crate::fsx::read_to_string(path).map_err(io_err(path))?
    };

    let doc = Document::parse(&xml)
        .map_err(|e| InteropError::Format(format!("{}: not valid XML ({e})", path.display())))?;
    let root = doc.root_element();
    if root.tag_name().name() != "workbook" {
        return Err(InteropError::Format(format!(
            "{}: not a Tableau workbook",
            path.display()
        )));
    }

    let mut report = Report::new(stem);
    let mut param_names = UniqueNames::default();
    let mut datasources: Vec<Ds> = Vec::new();

    if let Some(list) = kids(root, "datasources").next() {
        for ds in kids(list, "datasource") {
            if ds.attribute("name") == Some("Parameters") {
                import_parameters(ds, &mut report, &mut param_names);
            }
        }
        for ds in kids(list, "datasource") {
            if ds.attribute("name") == Some("Parameters") {
                continue;
            }
            if let Some(info) = import_datasource(ds, &mut ctx, &mut report) {
                datasources.push(info);
            }
        }
    }

    let mut visual_ids = UniqueNames::default();
    let mut sheets: HashMap<String, String> = HashMap::new();
    if let Some(list) = kids(root, "worksheets").next() {
        for ws in kids(list, "worksheet") {
            let name = ws.attribute("name").unwrap_or("Sheet").to_string();
            if let Some(visual) = import_worksheet(ws, &mut ctx, &datasources, &mut visual_ids) {
                sheets.insert(name, visual.id.clone());
                report.visuals.push(visual);
            }
        }
    }

    if let Some(list) = kids(root, "dashboards").next() {
        let mut ids = UniqueNames::default();
        for dash in kids(list, "dashboard") {
            let dashboard = import_dashboard(dash, &sheets, &mut ids, &mut ctx);
            report.dashboards.push(dashboard);
        }
    }

    Ok(Imported {
        report,
        warnings: ctx.warnings,
        files: ctx.files,
    })
}

// ------------------------------------------------------------- packaging

fn read_twbx(
    path: &Path,
    out_dir: &Path,
    stem: &str,
    ctx: &mut Ctx<'_>,
) -> Result<String, InteropError> {
    let file = crate::fsx::open(path).map_err(io_err(path))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| InteropError::Archive(format!("{}: {e}", path.display())))?;
    let mut workbook: Option<String> = None;
    let data_dir_name = format!("{}_data", sanitize(stem));
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| InteropError::Archive(format!("{}: {e}", path.display())))?;
        if entry.is_dir() {
            continue;
        }
        let Some(inner) = entry.enclosed_name() else {
            continue;
        };
        let inner_text = inner.to_string_lossy().replace('\\', "/");
        let lower = inner_text.to_ascii_lowercase();
        if lower.ends_with(".twb") && !inner_text.contains('/') {
            let mut text = String::new();
            entry
                .by_ref()
                .take(MAX_UNPACKED_BYTES)
                .read_to_string(&mut text)
                .map_err(|e| InteropError::Archive(format!("{}: {e}", path.display())))?;
            workbook = Some(text);
        } else if let Some(rest) = inner_text.strip_prefix("Data/") {
            if lower.ends_with(".hyper") || lower.ends_with(".tde") {
                continue; // Tableau extracts cannot be read
            }
            let target = out_dir.join(&data_dir_name).join(rest);
            if let Some(parent) = target.parent() {
                crate::fsx::create_dir_all(parent).map_err(io_err(parent))?;
            }
            let mut out = crate::fsx::create(&target).map_err(io_err(&target))?;
            std::io::copy(&mut entry.by_ref().take(MAX_UNPACKED_BYTES), &mut out)
                .map_err(io_err(&target))?;
            ctx.files.push(target);
            ctx.unpacked
                .push((rest.to_string(), format!("{data_dir_name}/{rest}")));
        }
    }
    workbook.ok_or_else(|| {
        InteropError::Archive(format!("{}: no .twb workbook inside", path.display()))
    })
}

// ------------------------------------------------------------ parameters

fn parse_literal(text: &str, datatype: &str) -> Option<Value> {
    let t = text.trim();
    let unquoted = t
        .strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .or_else(|| t.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')));
    if let Some(inner) = unquoted {
        return Some(Value::String(inner.to_string()));
    }
    if let Some(date) = t.strip_prefix('#').and_then(|t| t.strip_suffix('#')) {
        let day = date.split_whitespace().next().unwrap_or(date);
        return Some(Value::String(day.to_string()));
    }
    match datatype {
        "integer" => t.parse::<i64>().ok().map(Value::from),
        "real" => t.parse::<f64>().ok().and_then(|n| {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                Some(Value::from(n as i64))
            } else {
                serde_json::Number::from_f64(n).map(Value::Number)
            }
        }),
        "boolean" => match t.to_ascii_lowercase().as_str() {
            "true" => Some(Value::Bool(true)),
            "false" => Some(Value::Bool(false)),
            _ => None,
        },
        _ => {
            if let Ok(n) = t.parse::<i64>() {
                Some(Value::from(n))
            } else if let Ok(n) = t.parse::<f64>() {
                serde_json::Number::from_f64(n).map(Value::Number)
            } else {
                Some(Value::String(t.to_string()))
            }
        }
    }
}

fn import_parameters(ds: Node<'_, '_>, report: &mut Report, names: &mut UniqueNames) {
    for col in kids(ds, "column") {
        let internal = strip_brackets(col.attribute("name").unwrap_or("parameter"));
        let caption = col.attribute("caption").unwrap_or(&internal);
        let datatype = col.attribute("datatype").unwrap_or("string");
        let kind = match datatype {
            "integer" => ParamType::Integer,
            "real" => ParamType::Number,
            "boolean" => ParamType::Boolean,
            "date" | "datetime" => ParamType::Date,
            _ => ParamType::String,
        };
        let value = col
            .attribute("value")
            .and_then(|v| parse_literal(v, datatype));
        let mut parameter = Parameter::new(names.claim(caption), value);
        parameter.kind = Some(kind);
        if col.attribute("param-domain-type") == Some("list") {
            let allowed: Vec<Value> = kids(col, "members")
                .flat_map(|m| kids(m, "member"))
                .filter_map(|m| {
                    m.attribute("value")
                        .and_then(|v| parse_literal(v, datatype))
                })
                .collect();
            if !allowed.is_empty() {
                parameter.allowed = Some(allowed);
            }
        }
        if let Some(range) = kids(col, "range").next() {
            parameter.min = range
                .attribute("min")
                .and_then(|v| parse_literal(v, datatype));
            parameter.max = range
                .attribute("max")
                .and_then(|v| parse_literal(v, datatype));
        }
        report.parameters.push(parameter);
    }
}

// ----------------------------------------------------------- datasources

/// A table of a datasource's relation tree.
struct Table<'a, 'b> {
    relation: Node<'a, 'b>,
}

impl Table<'_, '_> {
    fn name(&self) -> String {
        self.relation
            .attribute("name")
            .unwrap_or("table")
            .to_string()
    }
}

struct FlatJoin {
    kind: JoinKind,
    /// ((table, column), (table, column)) per equality.
    keys: Vec<((String, String), (String, String))>,
}

fn flatten<'a, 'b>(
    relation: Node<'a, 'b>,
    tables: &mut Vec<Table<'a, 'b>>,
    joins: &mut Vec<FlatJoin>,
) -> Result<(), String> {
    match relation.attribute("type") {
        Some("join") => {
            let parts: Vec<Node> = relation
                .children()
                .filter(|c| c.is_element() && c.tag_name().name() == "relation")
                .collect();
            let [left, right] = parts.as_slice() else {
                return Err("a join without two sides".into());
            };
            flatten(*left, tables, joins)?;
            if right.attribute("type") == Some("join") {
                return Err("a join nested on its right side".into());
            }
            if right.attribute("type") == Some("text") {
                return Err("custom SQL".into());
            }
            tables.push(Table { relation: *right });
            let kind = match relation.attribute("join") {
                Some("left") => JoinKind::Left,
                Some("right") => JoinKind::Right,
                Some("full") => JoinKind::Full,
                _ => JoinKind::Inner,
            };
            let mut keys = Vec::new();
            for clause in kids(relation, "clause") {
                for expr in clause.descendants().filter(|n| {
                    n.is_element()
                        && n.tag_name().name() == "expression"
                        && n.attribute("op") == Some("=")
                }) {
                    let sides: Vec<Vec<String>> = kids(expr, "expression")
                        .map(|e| bracket_parts(e.attribute("op").unwrap_or("")))
                        .collect();
                    if let [a, b] = sides.as_slice()
                        && a.len() == 2
                        && b.len() == 2
                    {
                        keys.push(((a[0].clone(), a[1].clone()), (b[0].clone(), b[1].clone())));
                    }
                }
            }
            joins.push(FlatJoin { kind, keys });
            Ok(())
        }
        Some("text") => Err("custom SQL".into()),
        Some("table") | None => {
            tables.push(Table { relation });
            Ok(())
        }
        Some(other) => Err(format!("a {other} relation")),
    }
}

enum Location {
    File { path: String, format: FileFormat },
    Database { kind: DbKind, connection: String },
}

fn normalise(path: &str) -> String {
    path.replace('\\', "/").trim_end_matches('/').to_string()
}

/// Works out where a text file connection's data is.
fn locate_file(ctx: &mut Ctx<'_>, directory: &str, filename: &str) -> String {
    let dir = normalise(directory);
    let wanted = if dir.is_empty() {
        filename.to_string()
    } else {
        format!("{dir}/{filename}")
    };
    // A packaged workbook carries its data files.
    if let Some((_, rel)) = ctx
        .unpacked
        .iter()
        .find(|(inner, _)| wanted.ends_with(inner.as_str()) || inner.as_str() == wanted)
    {
        return rel.clone();
    }
    if let Some((_, rel)) = ctx
        .unpacked
        .iter()
        .find(|(inner, _)| inner.rsplit('/').next() == Some(filename))
    {
        return rel.clone();
    }
    let candidate = {
        let p = PathBuf::from(&wanted);
        if crate::util::is_absolute_path(&wanted) {
            p
        } else {
            ctx.twb_dir.join(p)
        }
    };
    if !crate::fsx::exists(&candidate) {
        ctx.warn(format!(
            "data file {} was not found; the source points to it anyway",
            candidate.display()
        ));
    }
    match candidate.strip_prefix(ctx.out_dir) {
        Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
        Err(_) => candidate.to_string_lossy().replace('\\', "/"),
    }
}

fn location_for(
    ctx: &mut Ctx<'_>,
    relation: Node<'_, '_>,
    connection: Node<'_, '_>,
) -> Result<Location, String> {
    let class = connection.attribute("class").unwrap_or("");
    match class {
        "textscan" => {
            let filename = connection
                .attribute("filename")
                .filter(|f| !f.is_empty())
                .or_else(|| relation.attribute("name"))
                .ok_or("a text file connection without a file name")?;
            let directory = connection.attribute("directory").unwrap_or("");
            Ok(Location::File {
                path: locate_file(ctx, directory, filename),
                format: FileFormat::Csv,
            })
        }
        "sqlite" | "sqlite3" => Ok(Location::Database {
            kind: DbKind::Sqlite,
            connection: sanitize(connection.attribute("dbname").unwrap_or("sqlite")),
        }),
        "postgres" => Ok(Location::Database {
            kind: DbKind::Postgres,
            connection: sanitize(connection.attribute("dbname").unwrap_or("postgres")),
        }),
        "hyper" | "dataengine" => Err("a Tableau extract, which cannot be read".into()),
        "" => Err("a connection without a type".into()),
        other => Err(format!("a {other} connection, which is not supported")),
    }
}

fn import_datasource(ds: Node<'_, '_>, ctx: &mut Ctx<'_>, report: &mut Report) -> Option<Ds> {
    let tableau_name = ds.attribute("name")?.to_string();
    let label = ds
        .attribute("caption")
        .unwrap_or_else(|| {
            tableau_name
                .strip_prefix("federated.")
                .unwrap_or(&tableau_name)
        })
        .to_string();
    let Some(conn) = kids(ds, "connection").next() else {
        ctx.warn(format!(
            "datasource '{label}' has no connection and was skipped"
        ));
        return None;
    };
    let named: HashMap<&str, Node> = kids(conn, "named-connections")
        .flat_map(|n| kids(n, "named-connection"))
        .filter_map(|nc| Some((nc.attribute("name")?, kids(nc, "connection").next()?)))
        .collect();
    let Some(root_relation) = kids(conn, "relation").next() else {
        ctx.warn(format!(
            "datasource '{label}' has no table (a published or extract-only datasource) and was skipped"
        ));
        return None;
    };

    let mut tables = Vec::new();
    let mut joins = Vec::new();
    if let Err(reason) = flatten(root_relation, &mut tables, &mut joins) {
        ctx.warn(format!(
            "datasource '{label}' uses {reason} and was skipped"
        ));
        return None;
    }

    // One source per table.
    let mut source_names = Vec::new();
    let mut database_table: Option<String> = None;
    for table in &tables {
        let connection = table
            .relation
            .attribute("connection")
            .and_then(|c| named.get(c).copied())
            .unwrap_or(conn);
        match location_for(ctx, table.relation, connection) {
            Ok(Location::File { path, format }) => {
                let name = match ctx.sources_by_path.get(&path) {
                    Some(existing) => existing.clone(),
                    None => {
                        let name = ctx.rel_names.claim(&table.name());
                        report.sources.push(Source {
                            name: name.clone(),
                            kind: SourceKind::File {
                                path: PathBuf::from(&path),
                                format,
                            },
                        });
                        ctx.sources_by_path.insert(path, name.clone());
                        name
                    }
                };
                source_names.push(name);
            }
            Ok(Location::Database { kind, connection }) => {
                let key = format!("db:{kind:?}:{connection}");
                let name = match ctx.sources_by_path.get(&key) {
                    Some(existing) => existing.clone(),
                    None => {
                        let name = ctx.rel_names.claim(&connection);
                        report.sources.push(Source {
                            name: name.clone(),
                            kind: SourceKind::Database {
                                kind,
                                connection: connection.clone(),
                            },
                        });
                        ctx.sources_by_path.insert(key, name.clone());
                        name
                    }
                };
                let table_name = bracket_parts(table.relation.attribute("table").unwrap_or(""))
                    .pop()
                    .unwrap_or_else(|| table.name());
                database_table = Some(table_name);
                source_names.push(name);
            }
            Err(reason) => {
                ctx.warn(format!(
                    "datasource '{label}' uses {reason} and was skipped"
                ));
                return None;
            }
        }
    }

    let dataset_name = ctx.rel_names.claim(&label);
    let mut dataset = Dataset {
        name: dataset_name.clone(),
        source: source_names[0].clone(),
        sql: None,
        joins: Vec::new(),
        calculated_fields: Vec::new(),
    };
    if let Some(table) = database_table {
        if tables.len() > 1 {
            ctx.warn(format!(
                "datasource '{label}' joins database tables, which is not imported"
            ));
            return None;
        }
        dataset.sql = Some(format!(
            "SELECT * FROM \"{}\".\"{}\"",
            dataset.source,
            table.replace('"', "\"\"")
        ));
    } else {
        for (index, join) in joins.iter().enumerate() {
            let right = &tables[index + 1];
            let right_name = right.name();
            let keys: Vec<JoinKey> = join
                .keys
                .iter()
                .map(|((t1, c1), (t2, c2))| {
                    if *t2 == right_name || *t1 != right_name {
                        JoinKey {
                            left: c1.clone(),
                            right: c2.clone(),
                        }
                    } else {
                        JoinKey {
                            left: c2.clone(),
                            right: c1.clone(),
                        }
                    }
                })
                .collect();
            if keys.is_empty() {
                ctx.warn(format!(
                    "a join in datasource '{label}' has no equality condition and was skipped"
                ));
                continue;
            }
            dataset.joins.push(Join {
                with: source_names[index + 1].clone(),
                kind: join.kind,
                on: keys,
            });
        }
    }

    let mut info = Ds {
        tableau_name,
        dataset: dataset_name,
        names: NameMap::new(),
        agg_calcs: HashMap::new(),
    };
    import_calculations(ds, &label, ctx, &mut dataset, &mut info);
    report.datasets.push(dataset);
    Some(info)
}

struct Calc {
    internal: String,
    field: String,
    formula: String,
}

fn import_calculations(
    ds: Node<'_, '_>,
    label: &str,
    ctx: &mut Ctx<'_>,
    dataset: &mut Dataset,
    info: &mut Ds,
) {
    let mut field_names = UniqueNames::default();
    let mut calcs: Vec<Calc> = Vec::new();
    for col in kids(ds, "column") {
        let Some(calc) = kids(col, "calculation").next() else {
            continue;
        };
        if calc.attribute("class") != Some("tableau") {
            continue;
        }
        let Some(formula) = calc.attribute("formula").filter(|f| !f.trim().is_empty()) else {
            continue;
        };
        let internal = strip_brackets(col.attribute("name").unwrap_or(""));
        if internal.is_empty() {
            continue;
        }
        let caption = col.attribute("caption").unwrap_or(&internal);
        let field = field_names.claim(caption);
        info.names.insert(internal.clone(), field.clone());
        calcs.push(Calc {
            internal,
            field,
            formula: formula.to_string(),
        });
    }

    let internals: HashSet<&str> = calcs.iter().map(|c| c.internal.as_str()).collect();
    let mut translated: Vec<(usize, String)> = Vec::new();
    let mut failed: HashSet<String> = HashSet::new();
    for (index, calc) in calcs.iter().enumerate() {
        if let Some((agg, raw)) = expr::simple_tableau_aggregate(&calc.formula) {
            let field = info.names.get(&raw).cloned().unwrap_or(raw);
            info.agg_calcs
                .insert(calc.internal.clone(), (agg, field, calc.field.clone()));
            continue;
        }
        let depends_on_failed = expr::tableau_references(&calc.formula)
            .iter()
            .any(|r| failed.contains(r));
        let result = if depends_on_failed {
            Err("depends on a calculation that could not be imported".to_string())
        } else {
            expr::tableau_to_duckdb(&calc.formula, &info.names).and_then(|text| {
                match ctx.options.check_expression {
                    Some(check) => check(&calc.field, &text).map(|()| text),
                    None => Ok(text),
                }
            })
        };
        match result {
            Ok(text) => translated.push((index, text)),
            Err(reason) => {
                failed.insert(calc.internal.clone());
                ctx.warn(format!(
                    "calculated field '{}' in '{label}' was not imported: {reason}",
                    calc.field
                ));
            }
        }
    }

    // Later fields may use earlier ones, so order by dependency.
    let mut placed: HashSet<String> = HashSet::new();
    let mut pending = translated;
    while !pending.is_empty() {
        let before = pending.len();
        let mut rest = Vec::new();
        for (index, text) in pending {
            let calc = &calcs[index];
            let ready = expr::tableau_references(&calc.formula).iter().all(|r| {
                r == &calc.internal || !internals.contains(r.as_str()) || placed.contains(r)
            });
            if ready {
                placed.insert(calc.internal.clone());
                dataset.calculated_fields.push(CalculatedField {
                    name: calc.field.clone(),
                    expression: text,
                    description: Some(format!("Imported from Tableau: {}", calc.formula.trim())),
                });
            } else {
                rest.push((index, text));
            }
        }
        if rest.len() == before {
            for (index, _) in rest {
                ctx.warn(format!(
                    "calculated field '{}' in '{label}' refers to itself in a loop and was not imported",
                    calcs[index].field
                ));
            }
            break;
        }
        pending = rest;
    }
}

// ------------------------------------------------------------ worksheets

#[derive(Debug, Clone)]
enum Use {
    Dim(String),
    DatePart(String),
    Agg(Aggregate, String, Option<String>),
    Skip(String),
}

struct Instance {
    column: String,
    derivation: String,
}

fn instances(view: Node<'_, '_>) -> HashMap<String, Instance> {
    let mut map = HashMap::new();
    for dep in kids(view, "datasource-dependencies") {
        for ci in kids(dep, "column-instance") {
            let (Some(name), Some(column)) = (ci.attribute("name"), ci.attribute("column")) else {
                continue;
            };
            map.insert(
                strip_brackets(name),
                Instance {
                    column: strip_brackets(column),
                    derivation: ci.attribute("derivation").unwrap_or("None").to_string(),
                },
            );
        }
    }
    map
}

fn derivation_from_prefix(prefix: &str) -> &'static str {
    match prefix {
        "sum" => "Sum",
        "avg" => "Avg",
        "min" => "Min",
        "max" => "Max",
        "cnt" => "Count",
        "ctd" => "CountD",
        "none" | "attr" => "None",
        "usr" => "User",
        "yr" | "tyr" | "mn" | "tmn" | "qr" | "tqr" | "dy" | "tdy" | "wk" | "twk" | "hr" | "mi"
        | "sc" => "Year",
        _ => "Unknown",
    }
}

fn resolve(instance_name: &str, map: &HashMap<String, Instance>, ds: &Ds) -> Use {
    let (column, derivation) = match map.get(instance_name) {
        Some(i) => (i.column.clone(), i.derivation.clone()),
        None => {
            let mut parts = instance_name.split(':');
            let prefix = parts.next().unwrap_or("");
            match (parts.next(), derivation_from_prefix(prefix)) {
                (Some(column), d) if d != "Unknown" => (column.to_string(), d.to_string()),
                _ => (instance_name.to_string(), "None".to_string()),
            }
        }
    };
    if column.starts_with(':') {
        return Use::Skip("Measure Names and Measure Values".into());
    }
    if let Some((agg, field, alias)) = ds.agg_calcs.get(&column) {
        return Use::Agg(*agg, field.clone(), Some(alias.clone()));
    }
    let field = ds.names.get(&column).cloned().unwrap_or(column);
    match derivation.as_str() {
        "Sum" => Use::Agg(Aggregate::Sum, field, None),
        "Avg" => Use::Agg(Aggregate::Avg, field, None),
        "Min" => Use::Agg(Aggregate::Min, field, None),
        "Max" => Use::Agg(Aggregate::Max, field, None),
        "Count" => Use::Agg(Aggregate::Count, field, None),
        "CountD" => Use::Agg(Aggregate::CountDistinct, field, None),
        "None" | "Attr" | "" => Use::Dim(field),
        "User" => Use::Skip(format!("a table calculation on '{field}'")),
        d if d.contains("Year")
            || d.contains("Month")
            || d.contains("Day")
            || d.contains("Quarter")
            || d.contains("Week")
            || d.contains("Hour") =>
        {
            Use::DatePart(field)
        }
        d => Use::Skip(format!("the '{d}' aggregation of '{field}'")),
    }
}

fn measure_for(agg: Aggregate, field: &str, alias: Option<String>) -> Measure {
    let default = format!("{}_{}", agg.as_str(), field);
    let alias = alias.or_else(|| {
        let safe = sanitize(&default);
        (safe != default).then_some(safe)
    });
    Measure {
        field: field.to_string(),
        agg,
        alias,
    }
}

#[derive(Default)]
struct Builder {
    dims: Vec<String>,
    measures: Vec<Measure>,
    date_parts: bool,
}

impl Builder {
    /// Registers a field and returns the name a channel should use for it.
    fn add(&mut self, used: &Use) -> Option<String> {
        match used {
            Use::Dim(f) => {
                if !self.dims.contains(f) {
                    self.dims.push(f.clone());
                }
                Some(f.clone())
            }
            Use::DatePart(f) => {
                self.date_parts = true;
                self.add(&Use::Dim(f.clone()))
            }
            Use::Agg(agg, f, alias) => {
                let m = measure_for(*agg, f, alias.clone());
                let out = m.output_name();
                if !self.measures.iter().any(|x| x.output_name() == out) {
                    self.measures.push(m);
                }
                Some(out)
            }
            Use::Skip(_) => None,
        }
    }

    fn knows(&self, name: &str) -> bool {
        self.dims.iter().any(|d| d == name) || self.measures.iter().any(|m| m.output_name() == name)
    }
}

/// The last `[...]` of each `[a].[b]` group in a shelf or column reference.
fn shelf_refs(text: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        let mut parts = Vec::new();
        let mut cursor = &rest[start..];
        loop {
            let found = bracket_parts(cursor);
            let Some(first) = found.first() else {
                break;
            };
            parts.push(first.clone());
            let consumed = cursor.find(']').map_or(cursor.len(), |i| i + 1);
            cursor = &cursor[consumed..];
            if cursor.starts_with(".[") {
                cursor = &cursor[1..];
            } else {
                break;
            }
        }
        if let Some(last) = parts.pop() {
            refs.push(last);
        }
        rest = cursor;
    }
    refs
}

fn outside_brackets(text: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in text.chars() {
        match c {
            '[' => depth += 1,
            ']' if depth > 0 => depth -= 1,
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

fn filter_value(text: &str) -> Option<Value> {
    let t = text.trim();
    if t == "%null%" || t.is_empty() {
        return None;
    }
    parse_literal(t, "")
}

fn import_worksheet(
    ws: Node<'_, '_>,
    ctx: &mut Ctx<'_>,
    datasources: &[Ds],
    ids: &mut UniqueNames,
) -> Option<Visual> {
    let name = ws.attribute("name").unwrap_or("Sheet").to_string();
    let table = kids(ws, "table").next()?;
    let view = kids(table, "view").next()?;
    let ds = kids(view, "datasources")
        .flat_map(|d| kids(d, "datasource"))
        .filter_map(|d| d.attribute("name"))
        .filter(|n| *n != "Parameters")
        .find_map(|n| datasources.iter().find(|d| d.tableau_name == n));
    let Some(ds) = ds else {
        ctx.warn(format!(
            "sheet '{name}' uses a datasource that was not imported and was skipped"
        ));
        return None;
    };
    let map = instances(view);
    let mut b = Builder::default();
    let mut skipped: Vec<String> = Vec::new();
    let mut take = |text: &str, b: &mut Builder| -> Vec<String> {
        shelf_refs(text)
            .iter()
            .filter_map(|r| {
                let used = resolve(r, &map, ds);
                if let Use::Skip(why) = &used {
                    skipped.push(why.clone());
                }
                b.add(&used)
            })
            .collect()
    };

    let cols_text = kids(table, "cols")
        .next()
        .and_then(|n| n.text())
        .unwrap_or("");
    let rows_text = kids(table, "rows")
        .next()
        .and_then(|n| n.text())
        .unwrap_or("");
    for text in [cols_text, rows_text] {
        if outside_brackets(text).contains('/') {
            ctx.warn(format!(
                "sheet '{name}' uses a dual axis; the second axis was merged into the first"
            ));
        }
    }
    let cols = take(cols_text, &mut b);
    let rows = take(rows_text, &mut b);
    let cols_have_measure = cols
        .iter()
        .any(|c| b.measures.iter().any(|m| &m.output_name() == c));
    let rows_have_measure = rows
        .iter()
        .any(|c| b.measures.iter().any(|m| &m.output_name() == c));

    let pane = kids(table, "panes").flat_map(|p| kids(p, "pane")).next();
    let mark = pane
        .and_then(|p| kids(p, "mark").next())
        .and_then(|m| m.attribute("class"))
        .unwrap_or("Automatic")
        .to_ascii_lowercase();

    let mut encodings = Encodings::default();
    if let Some(enc) = pane.and_then(|p| kids(p, "encodings").next()) {
        for channel in enc.children().filter(|c| c.is_element()) {
            let Some(column) = channel.attribute("column") else {
                continue;
            };
            let tag = channel.tag_name().name().to_string();
            for out in take(column, &mut b) {
                match tag.as_str() {
                    "color" if encodings.color.is_none() => encodings.color = Some(out),
                    "size" if encodings.size.is_none() => encodings.size = Some(out),
                    _ => {}
                }
            }
        }
    }

    let has_measure = !b.measures.is_empty();
    let chart = match mark.as_str() {
        "bar" | "gantt bar" => ChartType::Bar,
        "line" => ChartType::Line,
        "circle" | "shape" | "cross" => ChartType::Scatter,
        "text" => ChartType::Table,
        "square" => {
            if cols.is_empty() || rows.is_empty() {
                ChartType::Table
            } else {
                ChartType::Heatmap
            }
        }
        "area" => {
            ctx.warn(format!(
                "sheet '{name}' is an area chart, imported as a line chart"
            ));
            ChartType::Line
        }
        "pie" => {
            ctx.warn(format!(
                "sheet '{name}' is a pie chart, imported as a bar chart"
            ));
            ChartType::Bar
        }
        "automatic" | "" => {
            if cols_have_measure && rows_have_measure {
                ChartType::Scatter
            } else if has_measure && (!cols.is_empty() || !rows.is_empty()) {
                if b.date_parts {
                    ChartType::Line
                } else {
                    ChartType::Bar
                }
            } else {
                ChartType::Table
            }
        }
        other => {
            ctx.warn(format!(
                "sheet '{name}' uses the '{other}' mark, which has no equivalent; imported as a table"
            ));
            ChartType::Table
        }
    };

    if chart != ChartType::Table {
        encodings.x = cols.first().cloned();
        encodings.y = rows.first().cloned();
    }
    let chart = if chart == ChartType::Heatmap {
        if encodings.color.is_none() {
            encodings.color = b.measures.first().map(Measure::output_name);
        }
        if encodings.color.is_none() {
            ChartType::Table
        } else {
            ChartType::Heatmap
        }
    } else {
        chart
    };
    if chart == ChartType::Table {
        encodings = Encodings::default();
    }

    for why in skipped.iter().collect::<HashSet<_>>() {
        ctx.warn(format!("sheet '{name}': skipped {why}"));
    }

    let mut filters: Vec<Filter> = Vec::new();
    for f in kids(view, "filter") {
        let Some(column) = f.attribute("column") else {
            continue;
        };
        let Some(inst) = shelf_refs(column).pop() else {
            continue;
        };
        let used = resolve(&inst, &map, ds);
        let (field, scope) = match &used {
            Use::Dim(field) => (field.clone(), FilterScope::Row),
            Use::Agg(..) => match b.add(&used) {
                Some(out) => (out, FilterScope::Aggregate),
                None => continue,
            },
            Use::DatePart(field) => {
                ctx.warn(format!(
                    "sheet '{name}': a filter on a date part of '{field}' was skipped"
                ));
                continue;
            }
            Use::Skip(why) => {
                ctx.warn(format!("sheet '{name}': a filter on {why} was skipped"));
                continue;
            }
        };
        match f.attribute("class") {
            Some("categorical") => {
                let Some(root) = kids(f, "groupfilter").next() else {
                    continue;
                };
                let members: Vec<Value> = root
                    .descendants()
                    .filter(|n| {
                        n.is_element()
                            && n.tag_name().name() == "groupfilter"
                            && n.attribute("function") == Some("member")
                    })
                    .filter_map(|n| n.attribute("member").and_then(filter_value))
                    .collect();
                match root.attribute("function") {
                    Some("union") | Some("member") if !members.is_empty() => {
                        let op = if members.len() == 1 {
                            FilterOp::Eq
                        } else {
                            FilterOp::In
                        };
                        filters.push(Filter {
                            field,
                            op,
                            values: members,
                            scope,
                        });
                    }
                    Some("except") if !members.is_empty() => {
                        for member in members {
                            filters.push(Filter {
                                field: field.clone(),
                                op: FilterOp::Ne,
                                values: vec![member],
                                scope,
                            });
                        }
                    }
                    Some("level-members") => {}
                    other => ctx.warn(format!(
                        "sheet '{name}': a {} filter on '{field}' was skipped",
                        other.unwrap_or("custom")
                    )),
                }
            }
            Some("quantitative") => {
                let text = |tag: &'static str| {
                    kids(f, tag)
                        .next()
                        .and_then(|n| n.text())
                        .and_then(filter_value)
                };
                match (text("min"), text("max")) {
                    (Some(lo), Some(hi)) => filters.push(Filter {
                        field,
                        op: FilterOp::Between,
                        values: vec![lo, hi],
                        scope,
                    }),
                    (Some(lo), None) => filters.push(Filter {
                        field,
                        op: FilterOp::Ge,
                        values: vec![lo],
                        scope,
                    }),
                    (None, Some(hi)) => filters.push(Filter {
                        field,
                        op: FilterOp::Le,
                        values: vec![hi],
                        scope,
                    }),
                    (None, None) => {}
                }
            }
            other => ctx.warn(format!(
                "sheet '{name}': a {} filter was skipped",
                other.unwrap_or("custom")
            )),
        }
    }

    let sort = kids(view, "computed-sort")
        .next()
        .map(|s| {
            (
                s.attribute("using").or(s.attribute("column")),
                s.attribute("direction"),
            )
        })
        .or_else(|| {
            kids(view, "shelf-sort-v2").next().map(|s| {
                (
                    s.attribute("measure-to-sort-by")
                        .or(s.attribute("dimension-to-sort")),
                    s.attribute("direction"),
                )
            })
        })
        .and_then(|(by, direction)| {
            let inst = shelf_refs(by?).pop()?;
            let used = resolve(&inst, &map, ds);
            let out = match &used {
                Use::Dim(f) | Use::DatePart(f) => f.clone(),
                Use::Agg(agg, f, alias) => measure_for(*agg, f, alias.clone()).output_name(),
                Use::Skip(_) => return None,
            };
            b.knows(&out).then(|| Sort {
                by: out,
                direction: if direction.is_some_and(|d| d.eq_ignore_ascii_case("DESC")) {
                    SortDirection::Desc
                } else {
                    SortDirection::Asc
                },
            })
        });

    if b.dims.is_empty() && b.measures.is_empty() {
        ctx.warn(format!("sheet '{name}' has no fields and was skipped"));
        return None;
    }

    Some(Visual {
        id: ids.claim(&name),
        title: name,
        dataset: ds.dataset.clone(),
        chart,
        encodings,
        dimensions: b.dims,
        measures: b.measures,
        filters,
        sort,
        limit: None,
    })
}

// ------------------------------------------------------------ dashboards

fn import_dashboard(
    dash: Node<'_, '_>,
    sheets: &HashMap<String, String>,
    ids: &mut UniqueNames,
    ctx: &mut Ctx<'_>,
) -> Dashboard {
    let name = dash.attribute("name").unwrap_or("Dashboard").to_string();
    let mut dashboard = Dashboard::new(ids.claim(&name), name.clone());
    let height = kids(dash, "size")
        .next()
        .and_then(|s| {
            s.attribute("maxheight")
                .or(s.attribute("minheight"))
                .and_then(|h| h.parse::<f64>().ok())
        })
        .unwrap_or(800.0);
    let rows = ((height / f64::from(dashboard.row_height)).round() as u32).max(1);
    let num = |zone: Node<'_, '_>, key: &str| {
        zone.attribute(key)
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    let mut seen = HashSet::new();
    for zone in dash
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "zone")
    {
        let Some(sheet) = zone.attribute("name") else {
            continue;
        };
        let Some(visual) = sheets.get(sheet) else {
            continue;
        };
        if !seen.insert(visual.clone()) {
            continue;
        }
        dashboard.items.push(DashboardItem {
            visual: visual.clone(),
            x: to_grid(num(zone, "x"), 100_000.0, dashboard.columns),
            y: to_grid(num(zone, "y"), 100_000.0, rows),
            w: to_grid(num(zone, "w"), 100_000.0, dashboard.columns).max(1),
            h: to_grid(num(zone, "h"), 100_000.0, rows).max(1),
        });
    }
    fit_items(&mut dashboard);
    let missing = dash
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "zone")
        .filter(|z| {
            z.attribute("type-v2")
                .is_some_and(|t| t == "filter" || t == "param")
        })
        .count();
    if missing > 0 {
        ctx.warn(format!(
            "dashboard '{name}' has {missing} filter or parameter control(s), which are not imported"
        ));
    }
    dashboard
}
