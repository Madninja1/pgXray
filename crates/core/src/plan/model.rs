use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize)]
pub struct PlanDocument {
    pub root: PlanNode,
    pub execution_time_ms: Option<f64>,
    pub planning_time_ms: Option<f64>,
    pub jit_total_time_ms: Option<f64>,
    pub settings: BTreeMap<String, String>,
    pub node_count: usize,
    pub source_has_analyze: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlanNode {
    pub id: usize,
    pub path: String,
    pub node_type: String,
    pub parent_relationship: Option<String>,
    pub join_type: Option<String>,
    pub strategy: Option<String>,
    pub schema: Option<String>,
    pub relation_name: Option<String>,
    pub alias: Option<String>,
    pub index_name: Option<String>,

    pub startup_cost: Option<f64>,
    pub total_cost: Option<f64>,
    pub plan_rows: Option<f64>,
    pub plan_width: Option<f64>,

    pub actual_startup_time_ms: Option<f64>,
    pub actual_total_time_ms: Option<f64>,
    pub actual_rows: Option<f64>,
    pub actual_loops: Option<f64>,

    pub rows_removed_by_filter: Option<f64>,
    pub rows_removed_by_join_filter: Option<f64>,
    pub rows_removed_by_index_recheck: Option<f64>,

    pub shared_hit_blocks: Option<f64>,
    pub shared_read_blocks: Option<f64>,
    pub shared_dirtied_blocks: Option<f64>,
    pub shared_written_blocks: Option<f64>,
    pub temp_read_blocks: Option<f64>,
    pub temp_written_blocks: Option<f64>,
    pub io_read_time_ms: Option<f64>,
    pub io_write_time_ms: Option<f64>,
    pub wal_records: Option<f64>,
    pub wal_fpi: Option<f64>,
    pub wal_bytes: Option<f64>,

    pub heap_fetches: Option<f64>,
    pub exact_heap_blocks: Option<f64>,
    pub lossy_heap_blocks: Option<f64>,
    pub cache_hits: Option<f64>,
    pub cache_misses: Option<f64>,
    pub cache_evictions: Option<f64>,
    pub cache_overflows: Option<f64>,
    pub hash_buckets: Option<f64>,
    pub hash_batches: Option<f64>,
    pub peak_memory_kb: Option<f64>,
    pub sort_method: Option<String>,
    pub sort_space_type: Option<String>,
    pub sort_space_used_kb: Option<f64>,

    pub workers_planned: Option<f64>,
    pub workers_launched: Option<f64>,
    pub workers_reported: usize,
    pub workers_with_rows: usize,
    pub worker_rows_total: Option<f64>,

    pub filter: Option<String>,
    pub index_cond: Option<String>,
    pub recheck_cond: Option<String>,
    pub join_filter: Option<String>,
    pub hash_cond: Option<String>,
    pub merge_cond: Option<String>,
    pub sort_key: Vec<String>,
    pub output: Vec<String>,

    pub children: Vec<PlanNode>,
}

impl PlanNode {
    pub fn loop_count(&self) -> f64 {
        self.actual_loops.unwrap_or(1.0).max(0.0)
    }

    pub fn actual_rows_total(&self) -> Option<f64> {
        self.actual_rows.map(|rows| rows * self.loop_count())
    }

    /// PostgreSQL reports Actual Total Time as a per-loop average when loops > 1.
    /// Multiplication is useful for finding repeated work, but it is not wall-clock time.
    pub fn repeated_time_ms(&self) -> Option<f64> {
        self.actual_total_time_ms
            .map(|time| time * self.loop_count())
    }

    pub fn display_name(&self) -> String {
        let mut parts = vec![format!("#{} {}", self.id, self.node_type)];

        if let Some(relation) = &self.relation_name {
            if let Some(schema) = &self.schema {
                parts.push(format!("{schema}.{relation}"));
            } else {
                parts.push(relation.clone());
            }
        }

        if let Some(index) = &self.index_name {
            parts.push(format!("index={index}"));
        }

        parts.join(" · ")
    }
}

pub fn parse_explain_json(input: &str) -> Result<PlanDocument, String> {
    let value: Value = serde_json::from_str(input)
        .map_err(|error| format!("EXPLAIN JSON не разбирается: {error}"))?;

    let root_object = match &value {
        Value::Array(values) if values.len() == 1 => values[0]
            .as_object()
            .ok_or("Элемент EXPLAIN FORMAT JSON array должен быть объектом")?,
        Value::Array(values) => {
            return Err(format!(
                "Ожидается ровно один EXPLAIN plan; получено {} элементов",
                values.len()
            ))
        }
        Value::Object(object) => object,
        _ => {
            return Err(crate::i18n::text(
                "Ожидается PostgreSQL EXPLAIN FORMAT JSON object/array",
                "Expected a PostgreSQL EXPLAIN FORMAT JSON object or array",
            )
            .into())
        }
    };

    let plan_value = root_object
        .get("Plan")
        .ok_or("JSON не похож на PostgreSQL EXPLAIN: отсутствует top-level Plan")?;

    validate_numeric_fields(root_object, &["Execution Time", "Planning Time"], "EXPLAIN")?;
    if let Some(jit) = root_object.get("JIT").and_then(Value::as_object) {
        if let Some(timing) = jit.get("Timing").and_then(Value::as_object) {
            validate_numeric_fields(timing, &["Total"], "JIT Timing")?;
        }
    }

    let mut next_id = 1_usize;
    let root = parse_node(plan_value, "0", &mut next_id, 0)?;
    let node_count = next_id.saturating_sub(1);

    let settings = root_object
        .get("Settings")
        .and_then(Value::as_object)
        .map(|object| {
            object
                .iter()
                .map(|(key, value)| (key.clone(), scalar_to_string(value)))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();

    let jit_total_time_ms = root_object
        .get("JIT")
        .and_then(Value::as_object)
        .and_then(|jit| jit.get("Timing"))
        .and_then(Value::as_object)
        .and_then(|timing| number(timing.get("Total")));

    let source_has_analyze = root.actual_total_time_ms.is_some()
        || root.actual_rows.is_some()
        || root_object.get("Execution Time").is_some();

    Ok(PlanDocument {
        root,
        execution_time_ms: number(root_object.get("Execution Time")),
        planning_time_ms: number(root_object.get("Planning Time")),
        jit_total_time_ms,
        settings,
        node_count,
        source_has_analyze,
    })
}

const MAX_PLAN_DEPTH: usize = 96;
const MAX_PLAN_NODES: usize = 100_000;

fn parse_node(
    value: &Value,
    path: &str,
    next_id: &mut usize,
    depth: usize,
) -> Result<PlanNode, String> {
    if depth > MAX_PLAN_DEPTH {
        return Err(format!(
            "Превышена максимальная глубина EXPLAIN plan ({MAX_PLAN_DEPTH})"
        ));
    }
    if *next_id > MAX_PLAN_NODES {
        return Err(format!(
            "Превышено максимальное число узлов EXPLAIN plan ({MAX_PLAN_NODES})"
        ));
    }
    let object = value
        .as_object()
        .ok_or_else(|| format!("Plan node {path} не является JSON object"))?;
    validate_numeric_fields(object, NODE_NUMERIC_FIELDS, path)?;

    let id = *next_id;
    *next_id += 1;

    let node_type = text(object.get("Node Type"))
        .ok_or_else(|| format!("Plan node {path} не содержит Node Type"))?;

    let children = match object.get("Plans") {
        Some(Value::Array(values)) => values
            .iter()
            .enumerate()
            .map(|(index, child)| parse_node(child, &format!("{path}.{index}"), next_id, depth + 1))
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err(format!("Plans у узла {path} должен быть массивом")),
        None => Vec::new(),
    };

    Ok(PlanNode {
        id,
        path: path.to_owned(),
        node_type,
        parent_relationship: text(object.get("Parent Relationship")),
        join_type: text(object.get("Join Type")),
        strategy: text(object.get("Strategy")),
        schema: text(object.get("Schema")),
        relation_name: text(object.get("Relation Name")),
        alias: text(object.get("Alias")),
        index_name: text(object.get("Index Name")),

        startup_cost: number(object.get("Startup Cost")),
        total_cost: number(object.get("Total Cost")),
        plan_rows: number(object.get("Plan Rows")),
        plan_width: number(object.get("Plan Width")),

        actual_startup_time_ms: number(object.get("Actual Startup Time")),
        actual_total_time_ms: number(object.get("Actual Total Time")),
        actual_rows: number(object.get("Actual Rows")),
        actual_loops: number(object.get("Actual Loops")),

        rows_removed_by_filter: number(object.get("Rows Removed by Filter")),
        rows_removed_by_join_filter: number(object.get("Rows Removed by Join Filter")),
        rows_removed_by_index_recheck: number(object.get("Rows Removed by Index Recheck")),

        shared_hit_blocks: number(object.get("Shared Hit Blocks")),
        shared_read_blocks: number(object.get("Shared Read Blocks")),
        shared_dirtied_blocks: number(object.get("Shared Dirtied Blocks")),
        shared_written_blocks: number(object.get("Shared Written Blocks")),
        temp_read_blocks: number(object.get("Temp Read Blocks")),
        temp_written_blocks: number(object.get("Temp Written Blocks")),
        io_read_time_ms: number(object.get("I/O Read Time")),
        io_write_time_ms: number(object.get("I/O Write Time")),
        wal_records: number(object.get("WAL Records")),
        wal_fpi: number(object.get("WAL FPI")),
        wal_bytes: number(object.get("WAL Bytes")),

        heap_fetches: number(object.get("Heap Fetches")),
        exact_heap_blocks: number(object.get("Exact Heap Blocks")),
        lossy_heap_blocks: number(object.get("Lossy Heap Blocks")),
        cache_hits: number(object.get("Cache Hits")),
        cache_misses: number(object.get("Cache Misses")),
        cache_evictions: number(object.get("Cache Evictions")),
        cache_overflows: number(object.get("Cache Overflows")),
        hash_buckets: number(object.get("Hash Buckets")),
        hash_batches: number(object.get("Hash Batches")),
        peak_memory_kb: number(object.get("Peak Memory Usage")),
        sort_method: text(object.get("Sort Method")),
        sort_space_type: text(object.get("Sort Space Type")),
        sort_space_used_kb: number(object.get("Sort Space Used")),

        workers_planned: number(object.get("Workers Planned")),
        workers_launched: number(object.get("Workers Launched")),
        workers_reported: object
            .get("Workers")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0),
        workers_with_rows: object
            .get("Workers")
            .and_then(Value::as_array)
            .map(|workers| {
                workers
                    .iter()
                    .filter(|worker| {
                        worker
                            .as_object()
                            .and_then(|object| number(object.get("Actual Rows")))
                            .unwrap_or(0.0)
                            > 0.0
                    })
                    .count()
            })
            .unwrap_or(0),
        worker_rows_total: object
            .get("Workers")
            .and_then(Value::as_array)
            .map(|workers| {
                workers
                    .iter()
                    .filter_map(|worker| {
                        worker
                            .as_object()
                            .and_then(|object| number(object.get("Actual Rows")))
                    })
                    .sum::<f64>()
            })
            .filter(|value| *value > 0.0),

        filter: text(object.get("Filter")),
        index_cond: text(object.get("Index Cond")),
        recheck_cond: text(object.get("Recheck Cond")),
        join_filter: text(object.get("Join Filter")),
        hash_cond: text(object.get("Hash Cond")),
        merge_cond: text(object.get("Merge Cond")),
        sort_key: string_array(object.get("Sort Key")),
        output: string_array(object.get("Output")),

        children,
    })
}

// Check known numeric metrics before conversion: malformed values must not quietly
// become None and make the analyzer report an incomplete plan as a valid one.
const NODE_NUMERIC_FIELDS: &[&str] = &[
    "Startup Cost",
    "Total Cost",
    "Plan Rows",
    "Plan Width",
    "Actual Startup Time",
    "Actual Total Time",
    "Actual Rows",
    "Actual Loops",
    "Rows Removed by Filter",
    "Rows Removed by Join Filter",
    "Rows Removed by Index Recheck",
    "Shared Hit Blocks",
    "Shared Read Blocks",
    "Shared Dirtied Blocks",
    "Shared Written Blocks",
    "Temp Read Blocks",
    "Temp Written Blocks",
    "I/O Read Time",
    "I/O Write Time",
    "WAL Records",
    "WAL FPI",
    "WAL Bytes",
    "Heap Fetches",
    "Exact Heap Blocks",
    "Lossy Heap Blocks",
    "Cache Hits",
    "Cache Misses",
    "Cache Evictions",
    "Cache Overflows",
    "Hash Buckets",
    "Hash Batches",
    "Peak Memory Usage",
    "Sort Space Used",
    "Workers Planned",
    "Workers Launched",
];

fn validate_numeric_fields(
    object: &serde_json::Map<String, Value>,
    fields: &[&str],
    context: &str,
) -> Result<(), String> {
    for &key in fields {
        if let Some(value) = object.get(key) {
            if number(Some(value))
                .filter(|value| value.is_finite())
                .is_none()
            {
                return Err(format!("Некорректная числовая метрика {key} в {context}"));
            }
        }
    }
    Ok(())
}

fn number(value: Option<&Value>) -> Option<f64> {
    value.and_then(|value| match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    })
}

fn text(value: Option<&Value>) -> Option<String> {
    value.and_then(|value| match value {
        Value::String(text) => Some(text.clone()),
        _ => None,
    })
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn scalar_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null => "null".into(),
        _ => value.to_string(),
    }
}

pub fn walk_nodes<'a>(root: &'a PlanNode, output: &mut Vec<&'a PlanNode>) {
    output.push(root);

    for child in &root.children {
        walk_nodes(child, output);
    }
}

pub fn all_nodes(document: &PlanDocument) -> Vec<&PlanNode> {
    let mut output = Vec::with_capacity(document.node_count);
    walk_nodes(&document.root, &mut output);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_multiple_plans_in_one_array() {
        let source = r#"[{"Plan":{"Node Type":"Seq Scan"}},{"Plan":{"Node Type":"Index Scan"}}]"#;
        assert!(parse_explain_json(source)
            .unwrap_err()
            .contains("ровно один"));
    }

    #[test]
    fn rejects_invalid_children_and_numeric_metrics() {
        for source in [
            r#"[{"Plan":{"Node Type":"Seq Scan","Plans":{}}}]"#,
            r#"[{"Plan":{"Node Type":"Seq Scan","Actual Rows":"invalid"}}]"#,
            r#"[{"Plan":{"Node Type":"Seq Scan"}},"oops"]"#,
            r#"[{"Plan":{"Node Type":"Seq Scan"},"Execution Time":"invalid"}]"#,
        ] {
            assert!(
                parse_explain_json(source).is_err(),
                "unexpectedly accepted: {source}"
            );
        }
    }

    #[test]
    fn parses_nested_parallel_plan() {
        let source = r#"[{"Plan":{"Node Type":"Gather","Workers Planned":2,"Workers Launched":2,"Plans":[{"Node Type":"Seq Scan","Parallel Aware":true,"Actual Rows":100,"Actual Loops":3}]},"Execution Time":12.5}]"#;
        let plan = parse_explain_json(source).unwrap();
        assert_eq!(plan.node_count, 2);
        assert_eq!(plan.root.children[0].actual_rows_total(), Some(300.0));
    }

    #[test]
    fn parses_minimal_plan() {
        let source = r#"[{"Plan":{"Node Type":"Seq Scan","Relation Name":"orders","Plan Rows":10,"Actual Rows":8,"Actual Loops":1},"Execution Time":1.25}]"#;
        let plan = parse_explain_json(source).unwrap();

        assert_eq!(plan.node_count, 1);
        assert_eq!(plan.root.node_type, "Seq Scan");
        assert_eq!(plan.root.relation_name.as_deref(), Some("orders"));
        assert_eq!(plan.execution_time_ms, Some(1.25));
        assert!(plan.source_has_analyze);
    }
}
