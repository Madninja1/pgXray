use super::{mapping::PrivacyMapping, sql};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

const DROP_FIELDS: &[&str] = &["Query Identifier"];

const NAME_FIELDS: &[(&str, &str)] = &[
    ("Schema", "schema"),
    ("Relation Name", "table"),
    ("Relation", "table"),
    ("Alias", "alias"),
    ("Index Name", "index"),
    ("CTE Name", "cte"),
    ("Function Name", "function"),
    ("Trigger Name", "object"),
    ("Constraint Name", "constraint"),
    ("Object Name", "object"),
];

const EXPRESSION_FIELDS: &[&str] = &[
    "Output",
    "Filter",
    "Index Cond",
    "Recheck Cond",
    "Join Filter",
    "Hash Cond",
    "Merge Cond",
    "TID Cond",
    "Sort Key",
    "Presorted Key",
    "Group Key",
    "Cache Key",
    "Conflict Filter",
    "Hash Key",
    "Run Condition",
    "Function Call",
    "Table Function Call",
    "Sampling Parameters",
    "Repeatable Seed",
    "Remote SQL",
];

const NAME_LIST_FIELDS: &[(&str, &str)] = &[("Conflict Arbiter Indexes", "index")];

const SAFE_TEXT_FIELDS: &[&str] = &[
    "Node Type",
    "Parent Relationship",
    "Scan Direction",
    "Join Type",
    "Strategy",
    "Partial Mode",
    "Operation",
    "SetOp Command",
    "Sort Method",
    "Sort Space Type",
    "Conflict Resolution",
    "Cache Mode",
    "Custom Plan Provider",
];

const SAFE_SETTING_NAMES: &[&str] = &[
    "seq_page_cost",
    "random_page_cost",
    "cpu_tuple_cost",
    "cpu_index_tuple_cost",
    "cpu_operator_cost",
    "parallel_setup_cost",
    "parallel_tuple_cost",
    "min_parallel_table_scan_size",
    "min_parallel_index_scan_size",
    "effective_cache_size",
    "effective_io_concurrency",
    "maintenance_io_concurrency",
    "default_statistics_target",
    "constraint_exclusion",
    "cursor_tuple_fraction",
    "from_collapse_limit",
    "join_collapse_limit",
    "geqo",
    "geqo_threshold",
    "geqo_effort",
    "geqo_pool_size",
    "geqo_generations",
    "geqo_selection_bias",
    "geqo_seed",
    "work_mem",
    "hash_mem_multiplier",
    "plan_cache_mode",
    "search_path",
    "max_parallel_workers_per_gather",
    "recursive_worktable_factor",
];

#[derive(Default)]
struct ExplainStats {
    query_identifiers_removed: u64,
    custom_setting_names_anonymized: u64,
}

pub fn anonymize(
    input: &str,
    mapping: &mut PrivacyMapping,
) -> Result<(String, Vec<String>), String> {
    let value: Value = serde_json::from_str(input)
        .map_err(|error| format!("EXPLAIN JSON не разбирается: {error}"))?;

    validate_explain_shape(&value)?;

    let mut names = BTreeMap::new();

    survey_names(&value, &mut names, None);

    for (original, kind) in names {
        mapping.identifier(&original, &kind);
    }

    let mut stats = ExplainStats::default();

    let transformed = anonymize_value(&value, mapping, None, &mut stats)?;

    let mut warnings = Vec::new();

    if stats.query_identifiers_removed > 0 {
        warnings.push(format!(
            "Удалено Query Identifier: {}.",
            stats.query_identifiers_removed,
        ));
    }

    if stats.custom_setting_names_anonymized > 0 {
        warnings.push(format!(
            "Анонимизировано custom setting names: {}.",
            stats.custom_setting_names_anonymized,
        ));
    }

    Ok((
        serde_json::to_string_pretty(&transformed).map_err(|error| {
            format!("Не удалось сериализовать анонимизированный EXPLAIN JSON: {error}")
        })? + "\n",
        warnings,
    ))
}

pub fn deanonymize(input: &str, mapping: &PrivacyMapping) -> Result<String, String> {
    let value: Value = serde_json::from_str(input)
        .map_err(|error| format!("Анонимизированный EXPLAIN JSON не разбирается: {error}"))?;

    validate_explain_shape(&value)?;

    let transformed = deanonymize_value(&value, mapping, None)?;

    Ok(serde_json::to_string_pretty(&transformed).map_err(|error| {
        format!("Не удалось сериализовать деанонимизированный EXPLAIN JSON: {error}")
    })? + "\n")
}

fn validate_explain_shape(value: &Value) -> Result<(), String> {
    let validate_root = |root: &Value| {
        root.as_object()
            .and_then(|object| object.get("Plan"))
            .is_some_and(Value::is_object)
    };

    match value {
        Value::Array(values) if !values.is_empty() && values.iter().all(validate_root) => Ok(()),
        Value::Object(_) if validate_root(value) => Ok(()),
        _ => Err(
            crate::i18n::text("JSON не похож на PostgreSQL EXPLAIN FORMAT JSON: каждый top-level элемент должен содержать объект Plan", "JSON does not resemble PostgreSQL EXPLAIN FORMAT JSON: each top-level element must contain a Plan object").into(),
        ),
    }
}

fn survey_names(value: &Value, names: &mut BTreeMap<String, String>, parent_key: Option<&str>) {
    match value {
        Value::String(text) => {
            if let Some((_, kind)) = NAME_FIELDS
                .iter()
                .find(|(name, _)| Some(*name) == parent_key)
            {
                if !is_safe_schema(text) {
                    register_name(names, text, kind);
                }
            } else if let Some((_, kind)) = NAME_LIST_FIELDS
                .iter()
                .find(|(name, _)| Some(*name) == parent_key)
            {
                register_name(names, text, kind);
            }
        }

        Value::Array(values) => {
            for value in values {
                survey_names(value, names, parent_key);
            }
        }

        Value::Object(object) => {
            for (key, value) in object {
                survey_names(value, names, Some(key));
            }
        }

        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn register_name(names: &mut BTreeMap<String, String>, original: &str, kind: &str) {
    match names.get(original) {
        Some(current) if name_kind_priority(current) >= name_kind_priority(kind) => {}
        _ => {
            names.insert(original.to_owned(), kind.to_owned());
        }
    }
}

fn name_kind_priority(kind: &str) -> u8 {
    match kind {
        "database" => 100,
        "schema" => 95,
        "table" => 90,
        "index" => 85,
        "function" => 80,
        "constraint" => 75,
        "cte" => 65,
        "alias" => 55,
        _ => 10,
    }
}

fn anonymize_value(
    value: &Value,
    mapping: &mut PrivacyMapping,
    parent_key: Option<&str>,
    stats: &mut ExplainStats,
) -> Result<Value, String> {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(value.clone()),

        Value::String(text) => anonymize_string_value(parent_key, text, mapping),

        Value::Array(values) => {
            let mut result = Vec::with_capacity(values.len());

            for value in values {
                result.push(anonymize_value(value, mapping, parent_key, stats)?);
            }

            Ok(Value::Array(result))
        }

        Value::Object(object) => {
            if parent_key == Some("Settings") {
                return anonymize_settings(object, mapping, stats);
            }

            let mut result = Map::new();

            for (key, value) in object {
                if DROP_FIELDS.contains(&key.as_str()) {
                    stats.query_identifiers_removed =
                        stats.query_identifiers_removed.saturating_add(1);

                    continue;
                }

                // EXPLAIN JSON keys are part of the PostgreSQL/PEV2 protocol.
                // Never rename them: PEV2 versions may add support for fields
                // newer than pgXray's own allow-list.
                result.insert(
                    key.clone(),
                    anonymize_value(value, mapping, Some(key), stats)?,
                );
            }

            Ok(Value::Object(result))
        }
    }
}

fn deanonymize_value(
    value: &Value,
    mapping: &PrivacyMapping,
    parent_key: Option<&str>,
) -> Result<Value, String> {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(value.clone()),

        Value::String(text) => deanonymize_string_value(parent_key, text, mapping),

        Value::Array(values) => {
            let mut result = Vec::with_capacity(values.len());

            for value in values {
                result.push(deanonymize_value(value, mapping, parent_key)?);
            }

            Ok(Value::Array(result))
        }

        Value::Object(object) => {
            if parent_key == Some("Settings") {
                return deanonymize_settings(object, mapping);
            }

            let mut result = Map::new();

            for (key, value) in object {
                result.insert(key.clone(), deanonymize_value(value, mapping, Some(key))?);
            }

            Ok(Value::Object(result))
        }
    }
}

fn anonymize_string_value(
    key: Option<&str>,
    value: &str,
    mapping: &mut PrivacyMapping,
) -> Result<Value, String> {
    if let Some((_, kind)) = NAME_FIELDS.iter().find(|(name, _)| Some(*name) == key) {
        return Ok(Value::String(if is_safe_schema(value) {
            value.to_owned()
        } else {
            mapping.identifier(value, kind)
        }));
    }

    if let Some((_, kind)) = NAME_LIST_FIELDS.iter().find(|(name, _)| Some(*name) == key) {
        return Ok(Value::String(mapping.identifier(value, kind)));
    }

    if key.is_some_and(|key| EXPRESSION_FIELDS.contains(&key)) {
        return Ok(Value::String(sql::anonymize_fragment(value, mapping)?));
    }

    if key.is_some_and(|key| SAFE_TEXT_FIELDS.contains(&key)) {
        return Ok(Value::String(value.to_owned()));
    }

    if mapping.reverse_string(value).is_some() {
        return Ok(Value::String(value.to_owned()));
    }

    Ok(Value::String(mapping.string_value(value)))
}

fn deanonymize_string_value(
    key: Option<&str>,
    value: &str,
    mapping: &PrivacyMapping,
) -> Result<Value, String> {
    if key.is_some_and(|key| EXPRESSION_FIELDS.contains(&key)) {
        return Ok(Value::String(sql::deanonymize_fragment(value, mapping)?));
    }

    if NAME_FIELDS.iter().any(|(name, _)| Some(*name) == key)
        || NAME_LIST_FIELDS.iter().any(|(name, _)| Some(*name) == key)
    {
        return Ok(Value::String(
            mapping
                .reverse_identifier(value)
                .unwrap_or(value)
                .to_owned(),
        ));
    }

    Ok(Value::String(
        mapping.reverse_string(value).unwrap_or(value).to_owned(),
    ))
}

fn anonymize_settings(
    object: &Map<String, Value>,
    mapping: &mut PrivacyMapping,
    stats: &mut ExplainStats,
) -> Result<Value, String> {
    let mut result = Map::new();

    for (key, value) in object {
        if is_safe_setting_name(key) {
            result.insert(
                key.clone(),
                match value {
                    Value::String(text) if is_safe_setting_value(text) => {
                        Value::String(text.clone())
                    }

                    Value::String(text) => Value::String(mapping.string_value(text)),

                    _ => value.clone(),
                },
            );
        } else {
            stats.custom_setting_names_anonymized =
                stats.custom_setting_names_anonymized.saturating_add(1);

            let key_alias = mapping.identifier(key, "setting");

            result.insert(key_alias, anonymize_value(value, mapping, None, stats)?);
        }
    }

    Ok(Value::Object(result))
}

fn deanonymize_settings(
    object: &Map<String, Value>,
    mapping: &PrivacyMapping,
) -> Result<Value, String> {
    let mut result = Map::new();

    for (key, value) in object {
        let original_key = mapping.reverse_identifier(key).unwrap_or(key).to_owned();

        result.insert(original_key, deanonymize_value(value, mapping, None)?);
    }

    Ok(Value::Object(result))
}

fn is_safe_setting_name(key: &str) -> bool {
    // Prefix matching is unsafe: custom GUC names can mimic known settings.
    SAFE_SETTING_NAMES.contains(&key)
}

fn is_safe_setting_value(value: &str) -> bool {
    let value = value.trim();

    if matches!(
        value.to_ascii_lowercase().as_str(),
        "on" | "off"
            | "true"
            | "false"
            | "yes"
            | "no"
            | "auto"
            | "partition"
            | "force_custom_plan"
            | "force_generic_plan"
    ) {
        return true;
    }

    let mut seen_digit = false;

    for character in value.chars() {
        if character.is_ascii_digit() {
            seen_digit = true;

            continue;
        }

        if character.is_ascii_whitespace()
            || matches!(
                character,
                '+' | '-'
                    | '.'
                    | 'B'
                    | 'k'
                    | 'M'
                    | 'G'
                    | 'T'
                    | 'u'
                    | 'm'
                    | 's'
                    | 'i'
                    | 'n'
                    | 'h'
                    | 'd'
            )
        {
            continue;
        }

        return false;
    }

    seen_digit
}

fn is_safe_schema(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "public" | "pg_catalog" | "information_schema"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_unknown_protocol_keys() {
        let source = r#"[{"Plan":{"Node Type":"Seq Scan","Relation Name":"orders","Future PEV2 Field":"Display Enum"}}]"#;

        let mut mapping = PrivacyMapping::default();

        let (output, _) = anonymize(source, &mut mapping).unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();

        assert!(value[0]["Plan"].get("Future PEV2 Field").is_some(),);
    }

    #[test]
    fn rejects_invalid_plan_shapes_in_both_directions() {
        for source in [
            r#"[{"Plan":{"Node Type":"Result"}},{}]"#,
            r#"[{"Plan":{"Node Type":"Result"}},{"Plan":"invalid"}]"#,
            r#"{"Plan":null}"#,
            r#"[]"#,
        ] {
            let mut mapping = PrivacyMapping::default();
            assert!(
                anonymize(source, &mut mapping).is_err(),
                "accepted {source}"
            );
            assert!(deanonymize(source, &mapping).is_err(), "accepted {source}");
        }
    }

    #[test]
    fn custom_setting_with_known_prefix_is_anonymized() {
        let source =
            r#"[{"Plan":{"Node Type":"Result"},"Settings":{"enable_private_mode":"secret"}}]"#;
        let mut mapping = PrivacyMapping::default();
        let (output, warnings) = anonymize(source, &mut mapping).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert!(value[0]["Settings"].get("enable_private_mode").is_none());
        assert_eq!(warnings.len(), 1);
        let restored = deanonymize(&output, &mapping).unwrap();
        let restored_value: Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(
            restored_value[0]["Settings"]["enable_private_mode"],
            "secret"
        );
    }

    #[test]
    fn preserves_plan_metrics() {
        let source = r#"[{"Plan":{"Node Type":"Seq Scan","Relation Name":"orders","Plan Rows":1000,"Actual Rows":42,"Filter":"customer_id = 'secret'"}}]"#;

        let mut mapping = PrivacyMapping::default();

        let (output, _) = anonymize(source, &mut mapping).unwrap();

        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value[0]["Plan"]["Plan Rows"], 1000,);

        assert_eq!(value[0]["Plan"]["Actual Rows"], 42,);

        assert_ne!(value[0]["Plan"]["Relation Name"], "orders",);
    }

    #[test]
    fn preserves_explain_object_field_order() {
        let source = r#"[{
            "Plan": {
                "Node Type": "Seq Scan",
                "Parallel Aware": false,
                "Relation Name": "orders",
                "Alias": "orders",
                "Startup Cost": 0.0,
                "Total Cost": 10.0,
                "Plan Rows": 5
            }
        }]"#;

        let mut mapping = PrivacyMapping::default();

        let (output, _) = anonymize(source, &mut mapping).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        let keys: Vec<&str> = value[0]["Plan"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();

        assert_eq!(
            keys,
            vec![
                "Node Type",
                "Parallel Aware",
                "Relation Name",
                "Alias",
                "Startup Cost",
                "Total Cost",
                "Plan Rows",
            ]
        );
    }

    #[test]
    fn deanonymize_does_not_rename_regular_explain_keys() {
        let mut mapping = PrivacyMapping::default();
        let alias = mapping.identifier("private_setting", "setting");

        let source = format!(r#"[{{"Plan":{{"{alias}":"value"}}}}]"#);

        let output = deanonymize(&source, &mapping).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert!(value[0]["Plan"].get(&alias).is_some());
        assert!(value[0]["Plan"].get("private_setting").is_none());
    }

    #[test]
    fn deanonymize_restores_custom_setting_keys() {
        let mut mapping = PrivacyMapping::default();
        let alias = mapping.identifier("private.custom_setting", "setting");

        let source =
            format!(r#"[{{"Plan":{{"Node Type":"Result"}},"Settings":{{"{alias}":42}}}}]"#);

        let output = deanonymize(&source, &mapping).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value[0]["Settings"]["private.custom_setting"], 42,);
    }
}
