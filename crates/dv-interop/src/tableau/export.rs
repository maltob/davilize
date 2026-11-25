//! Writes a Davilize report as a Tableau workbook.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use dv_core::{
    Aggregate, ChartType, Dashboard, Dataset, FileFormat, Filter, FilterOp, FilterScope, Join,
    JoinKind, Measure, ParamType, Parameter, Report, SortDirection, Source, SourceKind, Visual,
};
use serde_json::Value;

use crate::expr;
use crate::util::{sanitize, xml_escape};
use crate::{Exported, InteropError, io_err};

const VERSION: &str = "18.1";

/// A file table of a datasource.
struct TableSpec {
    connection: String,
    filename: String,
    directory: String,
    source_name: String,
}

impl TableSpec {
    fn relation(&self) -> String {
        format!(
            "<relation connection='{}' name='{}' table='[{}]' type='table' />",
            xml_escape(&self.connection),
            xml_escape(&self.filename),
            xml_escape(&self.filename.replace('.', "#"))
        )
    }

    fn named_connection(&self) -> String {
        format!(
            "<named-connection caption='{dir}' name='{conn}'><connection class='textscan' directory='{dir}' filename='{file}' password='' server='' /></named-connection>",
            dir = xml_escape(&self.directory),
            conn = xml_escape(&self.connection),
            file = xml_escape(&self.filename)
        )
    }
}

struct ExDs {
    dataset: String,
    fed_name: String,
    xml: String,
}

fn strip_verbatim(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    text.strip_prefix("//?/").unwrap_or(&text).to_string()
}

fn find_source<'a>(report: &'a Report, name: &str) -> Option<&'a Source> {
    report.sources.iter().find(|s| s.name == name)
}

struct Exporter<'a> {
    report: &'a Report,
    report_dir: &'a Path,
    packaged: bool,
    warnings: Vec<String>,
    /// (file on disk, path inside the archive)
    data_files: Vec<(PathBuf, String)>,
}

impl Exporter<'_> {
    fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }

    fn table_for(&mut self, source_name: &str) -> Option<TableSpec> {
        let source = find_source(self.report, source_name)?;
        let SourceKind::File { path, format } = &source.kind else {
            self.warn(format!(
                "source '{source_name}' is a database; reports hold no connection details, so it was not exported"
            ));
            return None;
        };
        if *format != FileFormat::Csv {
            self.warn(format!(
                "source '{source_name}' is {format:?}; Tableau reads only text files here, so it was not exported"
            ));
            return None;
        }
        let absolute = if path.is_absolute() {
            path.clone()
        } else {
            self.report_dir.join(path)
        };
        let filename = absolute.file_name()?.to_string_lossy().into_owned();
        let directory = if self.packaged {
            let dir = format!("Data/{}", sanitize(source_name));
            let entry = format!("{dir}/{filename}");
            if !self.data_files.iter().any(|(_, e)| *e == entry) {
                self.data_files.push((absolute.clone(), entry));
            }
            dir
        } else {
            let abs = crate::fsx::canonicalize(&absolute).unwrap_or(absolute);
            strip_verbatim(abs.parent().unwrap_or(Path::new("")))
        };
        Some(TableSpec {
            connection: format!("textscan.{}", sanitize(source_name)),
            filename,
            directory,
            source_name: source_name.to_string(),
        })
    }

    fn datasource(&mut self, dataset: &Dataset) -> Option<ExDs> {
        // Walk down through datasets built on datasets to the base source.
        let mut chain: Vec<&Dataset> = vec![dataset];
        let mut current = dataset;
        loop {
            if current.sql.is_some() {
                self.warn(format!(
                    "dataset '{}' uses custom SQL, which cannot be exported to a workbook",
                    current.name
                ));
                return None;
            }
            match self
                .report
                .datasets
                .iter()
                .find(|d| d.name == current.source)
            {
                Some(parent) => {
                    chain.push(parent);
                    current = parent;
                }
                None => break,
            }
        }
        chain.reverse(); // base first
        let base = self.table_for(&current.source)?;
        let mut tables = vec![base];
        let mut joins: Vec<(&Join, usize)> = Vec::new();
        for d in &chain {
            for join in &d.joins {
                let with = match self.report.datasets.iter().find(|x| x.name == join.with) {
                    Some(other) if other.sql.is_none() && other.joins.is_empty() => {
                        other.source.clone()
                    }
                    Some(_) => {
                        self.warn(format!(
                            "dataset '{}' joins the derived dataset '{}', which was not exported",
                            dataset.name, join.with
                        ));
                        return None;
                    }
                    None => join.with.clone(),
                };
                let table = self.table_for(&with)?;
                tables.push(table);
                joins.push((join, tables.len() - 1));
            }
        }

        let mut relation = tables[0].relation();
        for (join, index) in &joins {
            let right = &tables[*index];
            let left_table = &tables[0];
            let kind = match join.kind {
                JoinKind::Inner => "inner",
                JoinKind::Left => "left",
                JoinKind::Right => "right",
                JoinKind::Full => "full",
            };
            let equality = |key: &dv_core::JoinKey| {
                format!(
                    "<expression op='='><expression op='[{lt}].[{lc}]' /><expression op='[{rt}].[{rc}]' /></expression>",
                    lt = xml_escape(&left_table.filename),
                    lc = xml_escape(&key.left),
                    rt = xml_escape(&right.filename),
                    rc = xml_escape(&key.right)
                )
            };
            let condition = if join.on.len() == 1 {
                equality(&join.on[0])
            } else {
                format!(
                    "<expression op='AND'>{}</expression>",
                    join.on.iter().map(equality).collect::<String>()
                )
            };
            relation = format!(
                "<relation join='{kind}' type='join'><clause type='join'>{condition}</clause>{relation}{}</relation>",
                right.relation()
            );
        }

        let mut seen = HashSet::new();
        let named: String = tables
            .iter()
            .filter(|t| seen.insert(t.source_name.clone()))
            .map(TableSpec::named_connection)
            .collect();

        let fed_name = format!("federated.{}", sanitize(&dataset.name));
        let mut columns = String::new();
        for d in &chain {
            for field in &d.calculated_fields {
                let formula = match expr::duckdb_to_tableau(&field.expression) {
                    Ok(f) => f,
                    Err(reason) => {
                        self.warn(format!(
                            "calculated field '{}' in '{}' was not exported: {reason}",
                            field.name, dataset.name
                        ));
                        continue;
                    }
                };
                let textual = formula.contains('"') || formula.starts_with("STR(");
                let (datatype, role, ty) = if textual {
                    ("string", "dimension", "nominal")
                } else {
                    ("real", "measure", "quantitative")
                };
                columns.push_str(&format!(
                    "<column caption='{n}' datatype='{datatype}' name='[{n}]' role='{role}' type='{ty}'><calculation class='tableau' formula='{f}' /></column>",
                    n = xml_escape(&field.name),
                    f = xml_escape(&formula)
                ));
            }
        }
        let xml = format!(
            "<datasource caption='{cap}' inline='true' name='{fed}' version='{VERSION}'><connection class='federated'><named-connections>{named}</named-connections>{relation}</connection>{columns}</datasource>",
            cap = xml_escape(&dataset.name),
            fed = xml_escape(&fed_name),
        );
        Some(ExDs {
            dataset: dataset.name.clone(),
            fed_name,
            xml,
        })
    }
}

fn param_literal(value: &Value, kind: Option<ParamType>) -> String {
    match value {
        Value::String(s) => {
            if kind == Some(ParamType::Date) {
                format!("#{s}#")
            } else {
                format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
            }
        }
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

fn parameters_xml(params: &[Parameter]) -> String {
    if params.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "<datasource hasconnection='false' inline='true' name='Parameters' version='{VERSION}'><aliases enabled='yes' />"
    );
    for p in params {
        let kind = p.kind.or_else(|| {
            p.default.as_ref().map(|d| match d {
                Value::Bool(_) => ParamType::Boolean,
                Value::Number(n) if n.is_i64() => ParamType::Integer,
                Value::Number(_) => ParamType::Number,
                _ => ParamType::String,
            })
        });
        let datatype = match kind {
            Some(ParamType::Integer) => "integer",
            Some(ParamType::Number) => "real",
            Some(ParamType::Boolean) => "boolean",
            Some(ParamType::Date) => "date",
            _ => "string",
        };
        let literal = p
            .default
            .as_ref()
            .map(|v| param_literal(v, kind))
            .unwrap_or_default();
        let domain = if p.allowed.is_some() {
            "list"
        } else if p.min.is_some() || p.max.is_some() {
            "range"
        } else {
            "any"
        };
        out.push_str(&format!(
            "<column caption='{n}' datatype='{datatype}' name='[{n}]' param-domain-type='{domain}' role='measure' type='quantitative' value='{v}'><calculation class='tableau' formula='{v}' />",
            n = xml_escape(&p.name),
            v = xml_escape(&literal)
        ));
        if let Some(allowed) = &p.allowed {
            out.push_str("<members>");
            for a in allowed {
                out.push_str(&format!(
                    "<member value='{}' />",
                    xml_escape(&param_literal(a, kind))
                ));
            }
            out.push_str("</members>");
        } else if p.min.is_some() || p.max.is_some() {
            out.push_str("<range");
            if let Some(max) = &p.max {
                out.push_str(&format!(" max='{}'", xml_escape(&param_literal(max, kind))));
            }
            if let Some(min) = &p.min {
                out.push_str(&format!(" min='{}'", xml_escape(&param_literal(min, kind))));
            }
            out.push_str(" />");
        }
        out.push_str("</column>");
    }
    out.push_str("</datasource>");
    out
}

#[derive(Clone)]
struct Entry {
    instance: String,
    column: String,
    derivation: &'static str,
    measure: bool,
}

fn agg_parts(agg: Aggregate) -> (&'static str, &'static str) {
    match agg {
        Aggregate::Sum => ("sum", "Sum"),
        Aggregate::Avg => ("avg", "Avg"),
        Aggregate::Min => ("min", "Min"),
        Aggregate::Max => ("max", "Max"),
        Aggregate::Count => ("cnt", "Count"),
        Aggregate::CountDistinct => ("ctd", "CountD"),
    }
}

fn dim_entry(field: &str) -> Entry {
    Entry {
        instance: format!("none:{field}:nk"),
        column: field.to_string(),
        derivation: "None",
        measure: false,
    }
}

fn measure_entry(m: &Measure) -> Entry {
    let (prefix, derivation) = agg_parts(m.agg);
    Entry {
        instance: format!("{prefix}:{}:qk", m.field),
        column: m.field.clone(),
        derivation,
        measure: true,
    }
}

fn literal(value: &Value) -> String {
    match value {
        Value::String(s) => format!("\"{s}\""),
        other => other.to_string(),
    }
}

fn is_number(value: &Value) -> bool {
    value.is_number()
}

fn filter_xml(filter: &Filter, fed: &str, entry: &Entry, warnings: &mut Vec<String>) -> String {
    let column = format!("[{fed}].[{}]", entry.instance);
    let level = format!("[{}]", entry.column);
    let member = |v: &Value| {
        format!(
            "<groupfilter function='member' level='{}' member='{}' />",
            xml_escape(&level),
            xml_escape(&literal(v))
        )
    };
    let categorical = |inner: String| {
        format!(
            "<filter class='categorical' column='{}'>{inner}</filter>",
            xml_escape(&column)
        )
    };
    if filter
        .values
        .iter()
        .any(|v| dv_core::param_ref(v).ok().flatten().is_some())
    {
        warnings.push(format!(
            "the filter on '{}' uses a parameter, which was not exported",
            filter.field
        ));
        return String::new();
    }
    match filter.op {
        FilterOp::Eq | FilterOp::In if !filter.values.is_empty() => {
            if filter.values.len() == 1 {
                categorical(member(&filter.values[0]))
            } else {
                categorical(format!(
                    "<groupfilter function='union'>{}</groupfilter>",
                    filter.values.iter().map(member).collect::<String>()
                ))
            }
        }
        FilterOp::Ne if !filter.values.is_empty() => categorical(format!(
            "<groupfilter function='except'><groupfilter function='level-members' level='{}' />{}</groupfilter>",
            xml_escape(&level),
            filter.values.iter().map(member).collect::<String>()
        )),
        FilterOp::IsNull => categorical(format!(
            "<groupfilter function='member' level='{}' member='%null%' />",
            xml_escape(&level)
        )),
        FilterOp::IsNotNull => categorical(format!(
            "<groupfilter function='except'><groupfilter function='level-members' level='{0}' /><groupfilter function='member' level='{0}' member='%null%' /></groupfilter>",
            xml_escape(&level)
        )),
        FilterOp::Between if filter.values.len() == 2 && filter.values.iter().all(is_number) => {
            format!(
                "<filter class='quantitative' column='{}' included-values='in-range'><min>{}</min><max>{}</max></filter>",
                xml_escape(&column),
                filter.values[0],
                filter.values[1]
            )
        }
        FilterOp::Ge | FilterOp::Gt | FilterOp::Le | FilterOp::Lt
            if filter.values.len() == 1 && is_number(&filter.values[0]) =>
        {
            if matches!(filter.op, FilterOp::Gt | FilterOp::Lt) {
                warnings.push(format!(
                    "the strict comparison on '{}' became inclusive in Tableau",
                    filter.field
                ));
            }
            let tag = if matches!(filter.op, FilterOp::Ge | FilterOp::Gt) {
                "min"
            } else {
                "max"
            };
            format!(
                "<filter class='quantitative' column='{}' included-values='in-range'><{tag}>{}</{tag}></filter>",
                xml_escape(&column),
                filter.values[0]
            )
        }
        _ => {
            warnings.push(format!(
                "the {:?} filter on '{}' could not be exported",
                filter.op, filter.field
            ));
            String::new()
        }
    }
}

fn worksheet_xml(visual: &Visual, sheet: &str, ds: &ExDs, warnings: &mut Vec<String>) -> String {
    let fed = &ds.fed_name;
    let measure_by_out: HashMap<String, &Measure> = visual
        .measures
        .iter()
        .map(|m| (m.output_name(), m))
        .collect();
    let entry_for = |name: &str| -> Entry {
        match measure_by_out.get(name) {
            Some(m) => measure_entry(m),
            None => dim_entry(name),
        }
    };
    let reference = |e: &Entry| format!("[{fed}].[{}]", e.instance);

    let mut deps: Vec<Entry> = Vec::new();
    let add_dep = |e: &Entry, deps: &mut Vec<Entry>| {
        if !deps.iter().any(|d| d.instance == e.instance) {
            deps.push(e.clone());
        }
    };

    let table_chart = visual.chart == ChartType::Table;
    let (cols, rows): (Vec<Entry>, Vec<Entry>) = if table_chart {
        (
            Vec::new(),
            visual.dimensions.iter().map(|d| dim_entry(d)).collect(),
        )
    } else {
        (
            visual.encodings.x.iter().map(|n| entry_for(n)).collect(),
            visual.encodings.y.iter().map(|n| entry_for(n)).collect(),
        )
    };
    let mut channels = String::new();
    let mut covered: HashSet<String> = HashSet::new();
    for e in cols.iter().chain(rows.iter()) {
        add_dep(e, &mut deps);
        covered.insert(e.instance.clone());
    }
    for (tag, name) in [
        ("color", &visual.encodings.color),
        ("size", &visual.encodings.size),
    ] {
        if let Some(name) = name {
            let e = entry_for(name);
            add_dep(&e, &mut deps);
            covered.insert(e.instance.clone());
            channels.push_str(&format!(
                "<{tag} column='{}' />",
                xml_escape(&reference(&e))
            ));
        }
    }
    let leftovers: Vec<Entry> = visual
        .dimensions
        .iter()
        .map(|d| dim_entry(d))
        .chain(visual.measures.iter().map(measure_entry))
        .filter(|e| !covered.contains(&e.instance))
        .collect();
    let extra_tag = if table_chart { "text" } else { "lod" };
    let mut extra_measures = 0;
    for e in &leftovers {
        add_dep(e, &mut deps);
        let tag = if e.measure { extra_tag } else { "lod" };
        if e.measure && table_chart {
            extra_measures += 1;
        }
        channels.push_str(&format!("<{tag} column='{}' />", xml_escape(&reference(e))));
    }
    if extra_measures > 1 {
        warnings.push(format!(
            "table '{}' has several measures; Tableau normally shows them with Measure Values",
            visual.title
        ));
    }

    let mark = match visual.chart {
        ChartType::Bar => "Bar",
        ChartType::Line => "Line",
        ChartType::Scatter => "Circle",
        ChartType::Table => "Text",
        ChartType::Heatmap => "Square",
    };

    let shelf = |entries: &[Entry]| -> String {
        let refs: Vec<String> = entries.iter().map(&reference).collect();
        match refs.len() {
            0 => String::new(),
            1 => refs[0].clone(),
            _ => format!("({})", refs.join(" * ")),
        }
    };

    let mut filters = String::new();
    for filter in &visual.filters {
        let entry = match filter.scope {
            FilterScope::Row => dim_entry(&filter.field),
            FilterScope::Aggregate => entry_for(&filter.field),
        };
        let xml = filter_xml(filter, fed, &entry, warnings);
        if !xml.is_empty() {
            add_dep(&entry, &mut deps);
            filters.push_str(&xml);
        }
    }
    if visual.limit.is_some() {
        warnings.push(format!(
            "the row limit of '{}' was not exported",
            visual.title
        ));
    }
    let mut sort = String::new();
    if let Some(s) = &visual.sort {
        let by = entry_for(&s.by);
        let column = if by.measure {
            visual.dimensions.first().map(|d| dim_entry(d))
        } else {
            Some(by.clone())
        };
        if let Some(column) = column {
            add_dep(&by, &mut deps);
            sort = format!(
                "<computed-sort column='{}' direction='{}' using='{}' />",
                xml_escape(&reference(&column)),
                if s.direction == SortDirection::Desc {
                    "DESC"
                } else {
                    "ASC"
                },
                xml_escape(&reference(&by))
            );
        }
    }

    let mut dep_xml = String::new();
    for e in &deps {
        let (datatype, role, ty) = if e.measure {
            ("real", "measure", "quantitative")
        } else {
            ("string", "dimension", "nominal")
        };
        dep_xml.push_str(&format!(
            "<column datatype='{datatype}' name='[{c}]' role='{role}' type='{ty}' /><column-instance column='[{c}]' derivation='{d}' name='[{i}]' pivot='key' type='{ty}' />",
            c = xml_escape(&e.column),
            d = e.derivation,
            i = xml_escape(&e.instance)
        ));
    }

    format!(
        "<worksheet name='{name}'><table><view><datasources><datasource caption='{cap}' name='{fed}' /></datasources><datasource-dependencies datasource='{fed}'>{dep_xml}</datasource-dependencies>{filters}{sort}<aggregation value='true' /></view><style /><panes><pane selection-relaxation-option='selection-relaxation-allow'><view><breakdown value='auto' /></view><mark class='{mark}' /><encodings>{channels}</encodings></pane></panes><rows>{rows}</rows><cols>{cols}</cols></table></worksheet>",
        name = xml_escape(sheet),
        cap = xml_escape(&ds.dataset),
        fed = xml_escape(fed),
        rows = xml_escape(&shelf(&rows)),
        cols = xml_escape(&shelf(&cols)),
    )
}

fn dashboard_xml(dashboard: &Dashboard, sheets: &HashMap<String, String>) -> String {
    let width = 1000u32;
    let rows = dashboard
        .items
        .iter()
        .map(|i| i.y + i.h)
        .max()
        .unwrap_or(1)
        .max(1);
    let height = rows * dashboard.row_height;
    let mut id = 2;
    let mut zones = String::new();
    for item in &dashboard.items {
        let Some(sheet) = sheets.get(&item.visual) else {
            continue;
        };
        let unit = |v: u32, total: u32| (u64::from(v) * 100_000 / u64::from(total.max(1))) as u32;
        zones.push_str(&format!(
            "<zone h='{}' id='{id}' name='{}' w='{}' x='{}' y='{}' />",
            unit(item.h, rows),
            xml_escape(sheet),
            unit(item.w, dashboard.columns),
            unit(item.x, dashboard.columns),
            unit(item.y, rows),
        ));
        id += 1;
    }
    format!(
        "<dashboard name='{}'><style /><size maxheight='{height}' maxwidth='{width}' minheight='{height}' minwidth='{width}' /><zones><zone h='100000' id='1' type-v2='layout-basic' w='100000' x='0' y='0'>{zones}</zone></zones></dashboard>",
        xml_escape(&dashboard.title)
    )
}

pub(crate) fn export(
    report: &Report,
    report_dir: &Path,
    packaged: bool,
    out: &Path,
) -> Result<Exported, InteropError> {
    let mut exporter = Exporter {
        report,
        report_dir,
        packaged,
        warnings: Vec::new(),
        data_files: Vec::new(),
    };
    let mut datasources: Vec<ExDs> = Vec::new();
    for dataset in &report.datasets {
        if let Some(ds) = exporter.datasource(dataset) {
            datasources.push(ds);
        }
    }

    let mut sheet_names: HashMap<String, String> = HashMap::new();
    let mut used_titles: HashSet<String> = HashSet::new();
    let mut worksheets = String::new();
    let mut warnings = Vec::new();
    for visual in &report.visuals {
        let Some(ds) = datasources.iter().find(|d| d.dataset == visual.dataset) else {
            warnings.push(format!(
                "visual '{}' was skipped because its dataset could not be exported",
                visual.id
            ));
            continue;
        };
        let mut sheet = visual.title.clone();
        if !used_titles.insert(sheet.clone()) {
            sheet = format!("{} ({})", visual.title, visual.id);
            used_titles.insert(sheet.clone());
        }
        worksheets.push_str(&worksheet_xml(visual, &sheet, ds, &mut warnings));
        sheet_names.insert(visual.id.clone(), sheet);
    }
    let mut dashboards = String::new();
    for dashboard in &report.dashboards {
        if !dashboard.filters.is_empty() {
            warnings.push(format!(
                "the shared filters of dashboard '{}' were not exported",
                dashboard.id
            ));
        }
        dashboards.push_str(&dashboard_xml(dashboard, &sheet_names));
    }
    warnings.splice(0..0, exporter.warnings.drain(..));

    let xml = format!(
        "<?xml version='1.0' encoding='utf-8' ?>\n<workbook original-version='{VERSION}' source-platform='win' version='{VERSION}' xmlns:user='http://www.tableausoftware.com/xml/user'><datasources>{}{}</datasources><worksheets>{worksheets}</worksheets>{}</workbook>\n",
        parameters_xml(&report.parameters),
        datasources
            .iter()
            .map(|d| d.xml.as_str())
            .collect::<String>(),
        if dashboards.is_empty() {
            String::new()
        } else {
            format!("<dashboards>{dashboards}</dashboards>")
        }
    );

    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        crate::fsx::create_dir_all(parent).map_err(io_err(parent))?;
    }
    if packaged {
        let file = crate::fsx::create(out).map_err(io_err(out))?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let stem = out.file_stem().map_or_else(
            || "workbook".to_string(),
            |s| s.to_string_lossy().into_owned(),
        );
        let archive_err = |e: zip::result::ZipError| InteropError::Archive(e.to_string());
        zip.start_file(format!("{stem}.twb"), options)
            .map_err(archive_err)?;
        zip.write_all(xml.as_bytes()).map_err(io_err(out))?;
        for (source, entry) in &exporter.data_files {
            let bytes = crate::fsx::read(source).map_err(io_err(source))?;
            zip.start_file(entry.as_str(), options)
                .map_err(archive_err)?;
            zip.write_all(&bytes).map_err(io_err(out))?;
        }
        zip.finish().map_err(archive_err)?;
    } else {
        crate::fsx::write(out, xml).map_err(io_err(out))?;
    }
    Ok(Exported {
        files: vec![out.to_path_buf()],
        warnings,
    })
}
