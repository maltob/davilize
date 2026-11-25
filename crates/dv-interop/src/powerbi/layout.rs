//! Report pages and visuals: the PBIR JSON files and the legacy `Report/Layout`.

use std::collections::HashMap;

use dv_core::FilterOp;
use serde_json::{Map, Value, json};

use super::{
    FieldRef, FilterSpec, PageSpec, VisualSpec, aggregate_code, aggregate_from_code,
    aggregate_label,
};

pub(crate) const VISUAL_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/visualContainer/1.0.0/schema.json";
pub(crate) const PAGE_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/page/1.0.0/schema.json";
pub(crate) const PAGES_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/pagesMetadata/1.0.0/schema.json";
pub(crate) const REPORT_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/report/3.0.0/schema.json";
pub(crate) const VERSION_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/versionMetadata/1.0.0/schema.json";

// ----------------------------------------------------------- field JSON

/// The column or measure expression with an entity name as its source.
fn column_expr(table: &str, column: &str, source: &str, by_alias: bool) -> Value {
    let source_ref = if by_alias {
        json!({ "Source": source })
    } else {
        json!({ "Entity": table })
    };
    json!({ "Column": { "Expression": { "SourceRef": source_ref }, "Property": column } })
}

pub(crate) fn field_json(field: &FieldRef) -> Value {
    match field {
        FieldRef::Column { table, column } => column_expr(table, column, "", false),
        FieldRef::Agg { table, column, agg } => json!({
            "Aggregation": {
                "Expression": column_expr(table, column, "", false),
                "Function": aggregate_code(*agg),
            }
        }),
        FieldRef::Measure { table, name } => json!({
            "Measure": { "Expression": { "SourceRef": { "Entity": table } }, "Property": name }
        }),
    }
}

pub(crate) fn query_ref(field: &FieldRef) -> String {
    match field {
        FieldRef::Column { table, column } => format!("{table}.{column}"),
        FieldRef::Agg { table, column, agg } => {
            format!("{}({table}.{column})", aggregate_label(*agg))
        }
        FieldRef::Measure { table, name } => format!("{table}.{name}"),
    }
}

/// Reads a field expression. `aliases` maps source aliases (`From` names) to entities.
pub(crate) fn field_from_json(
    value: &Value,
    aliases: &HashMap<String, String>,
) -> Option<FieldRef> {
    let entity = |expr: &Value| -> Option<String> {
        let source = &expr["Expression"]["SourceRef"];
        if let Some(e) = source.get("Entity").and_then(Value::as_str) {
            return Some(e.to_string());
        }
        let alias = source.get("Source")?.as_str()?;
        Some(
            aliases
                .get(alias)
                .cloned()
                .unwrap_or_else(|| alias.to_string()),
        )
    };
    if let Some(col) = value.get("Column") {
        return Some(FieldRef::Column {
            table: entity(col)?,
            column: col.get("Property")?.as_str()?.to_string(),
        });
    }
    if let Some(agg) = value.get("Aggregation") {
        let col = agg.get("Expression")?.get("Column")?;
        return Some(FieldRef::Agg {
            table: entity(col)?,
            column: col.get("Property")?.as_str()?.to_string(),
            agg: aggregate_from_code(agg.get("Function")?.as_u64()?)?,
        });
    }
    if let Some(m) = value.get("Measure") {
        return Some(FieldRef::Measure {
            table: entity(m)?,
            name: m.get("Property")?.as_str()?.to_string(),
        });
    }
    None
}

// -------------------------------------------------------------- filters

fn literal(value: &Value) -> Value {
    let text = match value {
        Value::String(s) => format!("'{}'", s.replace('\'', "''")),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) if n.is_i64() => format!("{n}L"),
        Value::Number(n) => format!("{n}D"),
        Value::Null => "null".to_string(),
        other => format!("'{other}'"),
    };
    json!({ "Literal": { "Value": text } })
}

fn literal_value(node: &Value) -> Option<Value> {
    let text = node.get("Literal")?.get("Value")?.as_str()?;
    if text == "null" {
        return Some(Value::Null);
    }
    if text == "true" || text == "false" {
        return Some(Value::Bool(text == "true"));
    }
    if let Some(inner) = text.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')) {
        return Some(Value::String(inner.replace("''", "'")));
    }
    if let Some(rest) = text.strip_prefix("datetime'") {
        let day = rest.trim_end_matches('\'');
        return Some(Value::String(
            day.split('T').next().unwrap_or(day).to_string(),
        ));
    }
    if let Some(n) = text.strip_suffix('L') {
        return n.parse::<i64>().ok().map(Value::from);
    }
    let n = text.strip_suffix('D').unwrap_or(text);
    n.parse::<f64>()
        .ok()
        .and_then(serde_json::Number::from_f64)
        .map(Value::Number)
}

fn comparison(kind: u8, expr: &Value, value: &Value) -> Value {
    json!({ "Comparison": { "ComparisonKind": kind, "Left": expr, "Right": literal(value) } })
}

/// The filter definition for a field and condition, with the source alias `t`.
pub(crate) fn filter_definition(filter: &FilterSpec) -> Option<(Value, &'static str)> {
    let table = filter.field.table().to_string();
    let target = match &filter.field {
        FieldRef::Column { column, .. } => column_expr(&table, column, "t", true),
        FieldRef::Agg { column, agg, .. } => json!({
            "Aggregation": {
                "Expression": column_expr(&table, column, "t", true),
                "Function": aggregate_code(*agg),
            }
        }),
        FieldRef::Measure { name, .. } => json!({
            "Measure": { "Expression": { "SourceRef": { "Source": "t" } }, "Property": name }
        }),
    };
    let (condition, kind) = match filter.op {
        FilterOp::Eq | FilterOp::In => (
            json!({ "In": {
                "Expressions": [target],
                "Values": filter.values.iter().map(|v| vec![literal(v)]).collect::<Vec<_>>(),
            } }),
            "Categorical",
        ),
        FilterOp::Ne => (
            json!({ "Not": { "Expression": { "In": {
                "Expressions": [target],
                "Values": filter.values.iter().map(|v| vec![literal(v)]).collect::<Vec<_>>(),
            } } } }),
            "Categorical",
        ),
        FilterOp::Gt | FilterOp::Ge | FilterOp::Lt | FilterOp::Le => {
            let k = match filter.op {
                FilterOp::Gt => 1,
                FilterOp::Ge => 2,
                FilterOp::Lt => 3,
                _ => 4,
            };
            (comparison(k, &target, filter.values.first()?), "Advanced")
        }
        FilterOp::Between => (
            json!({ "And": {
                "Left": comparison(2, &target, filter.values.first()?),
                "Right": comparison(4, &target, filter.values.get(1)?),
            } }),
            "Advanced",
        ),
        FilterOp::IsNull => (comparison(0, &target, &Value::Null), "Advanced"),
        FilterOp::IsNotNull => (
            json!({ "Not": { "Expression": comparison(0, &target, &Value::Null) } }),
            "Advanced",
        ),
    };
    Some((
        json!({
            "Version": 2,
            "From": [{ "Name": "t", "Entity": table, "Type": 0 }],
            "Where": [{ "Condition": condition }],
        }),
        kind,
    ))
}

fn condition_to_filter(
    cond: &Value,
    aliases: &HashMap<String, String>,
) -> Option<(FieldRef, FilterOp, Vec<Value>)> {
    if let Some(inner) = cond.get("In") {
        let field = field_from_json(inner.get("Expressions")?.get(0)?, aliases)?;
        let values: Vec<Value> = inner
            .get("Values")?
            .as_array()?
            .iter()
            .filter_map(|row| literal_value(row.get(0)?))
            .collect();
        let op = if values.len() == 1 {
            FilterOp::Eq
        } else {
            FilterOp::In
        };
        return Some((field, op, values));
    }
    if let Some(not) = cond.get("Not") {
        let (field, op, values) = condition_to_filter(not.get("Expression")?, aliases)?;
        return match op {
            FilterOp::Eq | FilterOp::In => Some((field, FilterOp::Ne, values)),
            FilterOp::IsNull => Some((field, FilterOp::IsNotNull, values)),
            _ => None,
        };
    }
    if let Some(c) = cond.get("Comparison") {
        let field = field_from_json(c.get("Left")?, aliases)?;
        let value = literal_value(c.get("Right")?)?;
        let op = match c.get("ComparisonKind")?.as_u64()? {
            0 if value.is_null() => FilterOp::IsNull,
            0 => FilterOp::Eq,
            1 => FilterOp::Gt,
            2 => FilterOp::Ge,
            3 => FilterOp::Lt,
            4 => FilterOp::Le,
            _ => return None,
        };
        let values = if value.is_null() { vec![] } else { vec![value] };
        return Some((field, op, values));
    }
    if let Some(and) = cond.get("And") {
        let (f1, o1, v1) = condition_to_filter(and.get("Left")?, aliases)?;
        let (f2, o2, v2) = condition_to_filter(and.get("Right")?, aliases)?;
        if f1 == f2 && o1 == FilterOp::Ge && o2 == FilterOp::Le {
            return Some((
                f1,
                FilterOp::Between,
                vec![v1.into_iter().next()?, v2.into_iter().next()?],
            ));
        }
    }
    None
}

/// Reads a `filter` object (`From` + `Where`) with its field expression.
fn filter_from_json(entry: &Value, warnings: &mut Vec<String>) -> Vec<FilterSpec> {
    let Some(def) = entry.get("filter") else {
        return Vec::new();
    };
    let aliases: HashMap<String, String> = def
        .get("From")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|f| {
            Some((
                f.get("Name")?.as_str()?.to_string(),
                f.get("Entity")?.as_str()?.to_string(),
            ))
        })
        .collect();
    let mut out = Vec::new();
    for w in def
        .get("Where")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match condition_to_filter(&w["Condition"], &aliases) {
            Some((field, op, values)) => out.push(FilterSpec { field, op, values }),
            None => warnings.push("a filter of an unsupported kind was skipped".to_string()),
        }
    }
    out
}

// ----------------------------------------------------------------- PBIR

fn sort_json(sort: &(FieldRef, bool)) -> Value {
    json!({
        "sort": [{
            "field": field_json(&sort.0),
            "direction": if sort.1 { "Descending" } else { "Ascending" },
        }],
        "isDefaultSort": true,
    })
}

fn title_object(title: &str) -> Value {
    json!({ "title": [{ "properties": {
        "show": { "expr": { "Literal": { "Value": "true" } } },
        "text": { "expr": { "Literal": { "Value": format!("'{}'", title.replace('\'', "''")) } } },
    } }] })
}

/// The contents of a PBIR `visual.json` file.
pub(crate) fn visual_to_pbir(spec: &VisualSpec, z: u32) -> Value {
    let mut state = Map::new();
    for (role, field) in &spec.roles {
        let entry = state
            .entry(role.clone())
            .or_insert_with(|| json!({ "projections": [] }));
        entry["projections"].as_array_mut().unwrap().push(json!({
            "field": field_json(field),
            "queryRef": query_ref(field),
            "nativeQueryRef": query_ref(field),
        }));
    }
    let mut query = json!({ "queryState": Value::Object(state) });
    if let Some(sort) = &spec.sort {
        query["sortDefinition"] = sort_json(sort);
    }
    let mut visual = json!({
        "visualType": spec.visual_type,
        "query": query,
        "drillFilterOtherVisuals": true,
    });
    if let Some(title) = &spec.title {
        visual["visualContainerObjects"] = title_object(title);
    }
    let mut out = json!({
        "$schema": VISUAL_SCHEMA,
        "name": spec.name,
        "position": {
            "x": spec.x, "y": spec.y, "z": z, "height": spec.height, "width": spec.width,
        },
        "visual": visual,
    });
    let filters: Vec<Value> = spec
        .filters
        .iter()
        .enumerate()
        .filter_map(|(i, f)| {
            let (def, kind) = filter_definition(f)?;
            Some(json!({
                "name": format!("filter{i}"),
                "field": field_json(&f.field),
                "type": kind,
                "filter": def,
            }))
        })
        .collect();
    if !filters.is_empty() {
        out["filterConfig"] = json!({ "filters": filters });
    }
    out
}

fn title_from(container: &Value) -> Option<String> {
    let text = container
        .get("title")?
        .get(0)?
        .get("properties")?
        .get("text")?
        .get("expr")?;
    literal_value(text).and_then(|v| v.as_str().map(str::to_string))
}

/// Reads a PBIR `visual.json`. Visuals without a data query (text boxes, images) give `None`.
pub(crate) fn visual_from_pbir(root: &Value, warnings: &mut Vec<String>) -> Option<VisualSpec> {
    let visual = root.get("visual")?;
    let visual_type = visual.get("visualType")?.as_str()?.to_string();
    let aliases = HashMap::new();
    let mut roles = Vec::new();
    for (role, body) in visual.get("query")?.get("queryState")?.as_object()?.iter() {
        for p in body
            .get("projections")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(field) = p.get("field").and_then(|f| field_from_json(f, &aliases)) {
                roles.push((role.clone(), field));
            }
        }
    }
    let sort = visual
        .get("query")
        .and_then(|q| q.get("sortDefinition"))
        .and_then(|s| s.get("sort"))
        .and_then(|s| s.get(0))
        .and_then(|s| {
            Some((
                field_from_json(s.get("field")?, &aliases)?,
                s.get("direction")?.as_str()? == "Descending",
            ))
        });
    let pos = root.get("position");
    let num = |k: &str| {
        pos.and_then(|p| p.get(k))
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
    };
    let mut filters = Vec::new();
    for entry in root
        .get("filterConfig")
        .and_then(|f| f.get("filters"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        filters.extend(filter_from_json(entry, warnings));
    }
    Some(VisualSpec {
        name: root.get("name")?.as_str()?.to_string(),
        visual_type,
        title: visual.get("visualContainerObjects").and_then(title_from),
        roles,
        x: num("x"),
        y: num("y"),
        width: num("width"),
        height: num("height"),
        filters,
        sort,
    })
}

// --------------------------------------------------------------- legacy

/// One entry of a legacy section's `visualContainers`.
pub(crate) fn visual_to_legacy(spec: &VisualSpec, z: u32) -> Value {
    let tables: Vec<String> = {
        let mut t: Vec<String> = Vec::new();
        for (_, f) in &spec.roles {
            if !t.iter().any(|x| x == f.table()) {
                t.push(f.table().to_string());
            }
        }
        t
    };
    let alias = |table: &str| format!("t{}", tables.iter().position(|t| t == table).unwrap_or(0));
    let aliased = |field: &FieldRef| -> Value {
        let a = alias(field.table());
        match field {
            FieldRef::Column { column, .. } => column_expr(field.table(), column, &a, true),
            FieldRef::Agg { column, agg, .. } => json!({ "Aggregation": {
                "Expression": column_expr(field.table(), column, &a, true),
                "Function": aggregate_code(*agg),
            } }),
            FieldRef::Measure { name, .. } => json!({ "Measure": {
                "Expression": { "SourceRef": { "Source": a } }, "Property": name,
            } }),
        }
    };
    let mut select = Vec::new();
    let mut projections = Map::new();
    for (role, field) in &spec.roles {
        let name = query_ref(field);
        let mut item = aliased(field);
        item["Name"] = json!(name);
        if !select.iter().any(|s: &Value| s["Name"] == item["Name"]) {
            select.push(item);
        }
        projections
            .entry(role.clone())
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .unwrap()
            .push(json!({ "queryRef": name }));
    }
    let mut prototype = json!({
        "Version": 2,
        "From": tables.iter().map(|t| json!({ "Name": alias(t), "Entity": t, "Type": 0 })).collect::<Vec<_>>(),
        "Select": select,
    });
    if let Some((field, desc)) = &spec.sort {
        prototype["OrderBy"] = json!([{
            "Direction": if *desc { 2 } else { 1 },
            "Expression": aliased(field),
        }]);
    }
    let mut single = json!({
        "visualType": spec.visual_type,
        "projections": Value::Object(projections),
        "prototypeQuery": prototype,
        "drillFilterOtherVisuals": true,
    });
    if let Some(title) = &spec.title {
        single["vcObjects"] = title_object(title);
    }
    let position = json!({
        "x": spec.x, "y": spec.y, "z": z, "width": spec.width, "height": spec.height,
    });
    let config = json!({
        "name": spec.name,
        "layouts": [{ "id": 0, "position": position }],
        "singleVisual": single,
    });
    let filters: Vec<Value> = spec
        .filters
        .iter()
        .enumerate()
        .filter_map(|(i, f)| {
            let (def, kind) = filter_definition(f)?;
            Some(json!({
                "name": format!("filter{i}"),
                "expression": field_json(&f.field),
                "filter": def,
                "type": kind,
            }))
        })
        .collect();
    json!({
        "x": spec.x, "y": spec.y, "z": z, "width": spec.width, "height": spec.height,
        "config": config.to_string(),
        "filters": Value::Array(filters).to_string(),
        "tabOrder": z,
    })
}

fn parse_embedded(value: &Value) -> Option<Value> {
    match value {
        Value::String(s) => serde_json::from_str(s).ok(),
        Value::Object(_) | Value::Array(_) => Some(value.clone()),
        _ => None,
    }
}

/// Reads a legacy visual container. Containers without a data query give `None`.
pub(crate) fn visual_from_legacy(
    container: &Value,
    warnings: &mut Vec<String>,
) -> Option<VisualSpec> {
    let config = parse_embedded(container.get("config")?)?;
    let single = config.get("singleVisual")?;
    let visual_type = single.get("visualType")?.as_str()?.to_string();
    let query = single.get("prototypeQuery")?;
    let aliases: HashMap<String, String> = query
        .get("From")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|f| {
            Some((
                f.get("Name")?.as_str()?.to_string(),
                f.get("Entity")?.as_str()?.to_string(),
            ))
        })
        .collect();
    let by_name: HashMap<String, FieldRef> = query
        .get("Select")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| {
            Some((
                s.get("Name")?.as_str()?.to_string(),
                field_from_json(s, &aliases)?,
            ))
        })
        .collect();
    let mut roles = Vec::new();
    for (role, items) in single.get("projections")?.as_object()? {
        for item in items.as_array().into_iter().flatten() {
            if let Some(field) = item
                .get("queryRef")
                .and_then(Value::as_str)
                .and_then(|r| by_name.get(r))
            {
                roles.push((role.clone(), field.clone()));
            }
        }
    }
    let sort = query.get("OrderBy").and_then(|o| o.get(0)).and_then(|o| {
        Some((
            field_from_json(o.get("Expression")?, &aliases)?,
            o.get("Direction")?.as_u64()? == 2,
        ))
    });
    let num = |k: &str| container.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    let mut filters = Vec::new();
    if let Some(Value::Array(list)) = container.get("filters").and_then(parse_embedded) {
        for entry in &list {
            filters.extend(filter_from_json(entry, warnings));
        }
    }
    Some(VisualSpec {
        name: config
            .get("name")
            .and_then(Value::as_str)
            .map_or_else(|| format!("visual{}", num("z")), str::to_string),
        visual_type,
        title: single.get("vcObjects").and_then(title_from),
        roles,
        x: num("x"),
        y: num("y"),
        width: num("width"),
        height: num("height"),
        filters,
        sort,
    })
}

/// The whole legacy `Report/Layout` document.
pub(crate) fn legacy_layout(pages: &[PageSpec]) -> Value {
    let sections: Vec<Value> = pages
        .iter()
        .enumerate()
        .map(|(i, page)| {
            json!({
                "name": page.name,
                "displayName": page.display_name,
                "filters": "[]",
                "ordinal": i,
                "visualContainers": page
                    .visuals
                    .iter()
                    .enumerate()
                    .map(|(z, v)| visual_to_legacy(v, z as u32))
                    .collect::<Vec<_>>(),
                "config": "{}",
                "displayOption": 1,
                "width": page.width,
                "height": page.height,
            })
        })
        .collect();
    json!({
        "id": 0,
        "resourcePackages": [],
        "sections": sections,
        "config": "{\"version\":\"5.55\",\"themeCollection\":{\"baseTheme\":{\"name\":\"CY24SU10\",\"version\":\"5.61\",\"type\":2}}}",
        "layoutOptimization": 0,
    })
}

pub(crate) fn pages_from_legacy(layout: &Value, warnings: &mut Vec<String>) -> Vec<PageSpec> {
    layout
        .get("sections")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|s| {
            let visuals = s
                .get("visualContainers")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|c| visual_from_legacy(c, warnings))
                .collect();
            let name = s.get("name").and_then(Value::as_str).unwrap_or("page");
            PageSpec {
                name: name.to_string(),
                display_name: s
                    .get("displayName")
                    .and_then(Value::as_str)
                    .unwrap_or(name)
                    .to_string(),
                width: s.get("width").and_then(Value::as_f64).unwrap_or(1280.0),
                height: s.get("height").and_then(Value::as_f64).unwrap_or(720.0),
                visuals,
            }
        })
        .collect()
}
