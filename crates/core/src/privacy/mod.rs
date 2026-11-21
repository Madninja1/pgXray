mod explain;
mod mapping;
mod result;
mod sql;
mod text;

use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs, path::Path};

pub use mapping::{IdentifierRecord, MappingLock, PrivacyMapping};
pub use result::{
    anonymize_result, anonymize_result_input, anonymize_result_json, deanonymize_result,
    deanonymize_result_input, deanonymize_result_json, ResultPrivacyConfig, ResultPrivacyDefault,
    ResultPrivacyFieldAction, ResultPrivacyNestedFieldConfig, ResultPrivacyPatternRule,
};

const MAX_TEXT_BYTES: usize = 32 * 1024 * 1024;

pub fn engine_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyMode {
    Text,
    TextKnown,
    Sql,
    Explain,
    Result,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyOperation {
    Anonymize,
    Deanonymize,
}

#[derive(Clone, Debug, Serialize)]
pub struct PrivacyMappingInfo {
    pub path: String,
    pub identifiers: usize,
    pub strings: usize,
    pub integers: usize,
    pub floats: usize,
    pub bitstrings: usize,
    pub redactions: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct PrivacyTransformResult {
    pub output: String,
    pub mode: PrivacyMode,
    pub operation: PrivacyOperation,
    pub mapping: PrivacyMappingInfo,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PrivacyMappingEntry {
    pub alias: String,
    pub original: String,
    pub category: String,
    pub kind: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PrivacyMappingEntryKey {
    pub category: String,
    pub original: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PrivacyMappingDeleteResult {
    pub mapping: PrivacyMappingInfo,
    pub deleted: usize,
    pub missing: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct PrivacyPreviewResult {
    pub transform: PrivacyTransformResult,
}

pub fn preview_anonymize(
    mapping_path: &Path,
    mode: PrivacyMode,
    input: &str,
) -> Result<PrivacyPreviewResult, String> {
    if input.len() > MAX_TEXT_BYTES {
        return Err(format!(
            "Privacy input слишком большой: {} bytes; максимум {} bytes",
            input.len(),
            MAX_TEXT_BYTES,
        ));
    }

    if input.contains('\0') {
        return Err(crate::i18n::text(
            "Privacy input содержит NUL byte",
            "Privacy input contains a NUL byte",
        )
        .into());
    }

    let _lock = MappingLock::acquire(mapping_path)?;
    let mut mapping = PrivacyMapping::load(mapping_path)?;

    let (output, warnings) = match mode {
        PrivacyMode::Text => (text::anonymize(input, &mut mapping), Vec::new()),
        PrivacyMode::TextKnown => (text::anonymize_known(input, &mapping), vec![
            crate::i18n::text("Заменены только известные соответствия из mapping (кроме очень коротких значений). Неизвестные секреты могли остаться: проверь результат вручную.", "Only known mappings were replaced (excluding very short values). Unknown secrets may remain: inspect the result manually.").to_owned(),
        ]),
        PrivacyMode::Sql => sql::anonymize(input, &mut mapping)?,
        PrivacyMode::Explain => explain::anonymize(input, &mut mapping)?,

        PrivacyMode::Result => result::anonymize_result_input(input, &mut mapping)?,
    };

    Ok(PrivacyPreviewResult {
        transform: PrivacyTransformResult {
            output,
            mode,
            operation: PrivacyOperation::Anonymize,
            mapping: mapping_info(mapping_path, &mapping),
            warnings,
        },
    })
}

pub fn transform(
    mapping_path: &Path,
    mode: PrivacyMode,
    operation: PrivacyOperation,
    input: &str,
) -> Result<PrivacyTransformResult, String> {
    transform_with_result_config(
        mapping_path,
        mode,
        operation,
        input,
        &ResultPrivacyConfig::default(),
    )
}

pub fn transform_with_result_config(
    mapping_path: &Path,
    mode: PrivacyMode,
    operation: PrivacyOperation,
    input: &str,
    result_config: &ResultPrivacyConfig,
) -> Result<PrivacyTransformResult, String> {
    if input.len() > MAX_TEXT_BYTES {
        return Err(format!(
            "Privacy input слишком большой: {} bytes; максимум {} bytes",
            input.len(),
            MAX_TEXT_BYTES,
        ));
    }

    if input.contains('\0') {
        return Err(crate::i18n::text(
            "Privacy input содержит NUL byte",
            "Privacy input contains a NUL byte",
        )
        .into());
    }

    let _lock = MappingLock::acquire(mapping_path)?;
    let mut mapping = PrivacyMapping::load(mapping_path)?;

    let (output, warnings) = match (mode, operation) {
        (PrivacyMode::Text, PrivacyOperation::Anonymize) => {
            (text::anonymize(input, &mut mapping), Vec::new())
        }
        (PrivacyMode::Text, PrivacyOperation::Deanonymize) => {
            (text::deanonymize(input, &mapping), Vec::new())
        }
        (PrivacyMode::TextKnown, PrivacyOperation::Anonymize) => (
            text::anonymize_known(input, &mapping),
            vec![crate::i18n::text("Заменены только известные соответствия из mapping (кроме очень коротких значений). Неизвестные секреты могли остаться: проверь результат вручную.", "Only known mappings were replaced (excluding very short values). Unknown secrets may remain: inspect the result manually.").to_owned()],
        ),
        (PrivacyMode::TextKnown, PrivacyOperation::Deanonymize) => (
            text::deanonymize_known(input, &mapping),
            Vec::new(),
        ),
        (PrivacyMode::Sql, PrivacyOperation::Anonymize) => sql::anonymize(input, &mut mapping)?,
        (PrivacyMode::Sql, PrivacyOperation::Deanonymize) => {
            (sql::deanonymize(input, &mapping)?, Vec::new())
        }
        (PrivacyMode::Explain, PrivacyOperation::Anonymize) => {
            explain::anonymize(input, &mut mapping)?
        }
        (PrivacyMode::Explain, PrivacyOperation::Deanonymize) => {
            (explain::deanonymize(input, &mapping)?, Vec::new())
        }
        (PrivacyMode::Result, PrivacyOperation::Anonymize) => {
            result::anonymize_result_input_with_config(input, &mut mapping, result_config)?
        }
        (PrivacyMode::Result, PrivacyOperation::Deanonymize) => (
            result::deanonymize_result_input_with_config(input, &mapping, result_config)?,
            Vec::new(),
        ),
    };

    if matches!(operation, PrivacyOperation::Anonymize) && !matches!(mode, PrivacyMode::TextKnown) {
        mapping.save(mapping_path)?;
    }

    Ok(PrivacyTransformResult {
        output,
        mode,
        operation,
        mapping: mapping_info(mapping_path, &mapping),
        warnings,
    })
}

pub fn transform_file(
    mapping_path: &Path,
    mode: PrivacyMode,
    operation: PrivacyOperation,
    input_path: &Path,
) -> Result<PrivacyTransformResult, String> {
    transform_file_with_result_config(
        mapping_path,
        mode,
        operation,
        input_path,
        &ResultPrivacyConfig::default(),
    )
}

pub fn transform_file_with_result_config(
    mapping_path: &Path,
    mode: PrivacyMode,
    operation: PrivacyOperation,
    input_path: &Path,
    result_config: &ResultPrivacyConfig,
) -> Result<PrivacyTransformResult, String> {
    let metadata = fs::metadata(input_path)
        .map_err(|error| format!("Не удалось прочитать privacy input file: {error}"))?;

    if metadata.len() > MAX_TEXT_BYTES as u64 {
        return Err(format!(
            "Privacy input file слишком большой: {} bytes; максимум {} bytes",
            metadata.len(),
            MAX_TEXT_BYTES,
        ));
    }

    let input = fs::read_to_string(input_path)
        .map_err(|error| format!("Privacy input должен быть UTF-8 text: {error}"))?;

    transform_with_result_config(mapping_path, mode, operation, &input, result_config)
}

pub fn reset_mapping(mapping_path: &Path) -> Result<PrivacyMappingInfo, String> {
    let _lock = MappingLock::acquire(mapping_path)?;

    let mut mapping = PrivacyMapping::default();

    mapping.save(mapping_path)?;

    Ok(mapping_info(mapping_path, &mapping))
}

pub fn mapping_entries_for_text(
    mapping_path: &Path,
    text: &str,
) -> Result<Vec<PrivacyMappingEntry>, String> {
    let _lock = MappingLock::acquire(mapping_path)?;
    let mapping = PrivacyMapping::load(mapping_path)?;

    Ok(mapping_entries_for_loaded_mapping(&mapping, text))
}

pub fn list_mapping_entries(
    mapping_path: &Path,
    query: Option<&str>,
    category: Option<&str>,
    limit: usize,
) -> Result<Vec<PrivacyMappingEntry>, String> {
    let _lock = MappingLock::acquire(mapping_path)?;
    let mapping = PrivacyMapping::load(mapping_path)?;
    let category = normalize_mapping_category_filter(category)?;
    let query = query
        .map(|value| value.trim())
        .filter(|value| !value.is_empty());
    let query = query.map(|value| value.to_lowercase());
    let limit = limit.clamp(1, 5_000);

    let entries = all_mapping_entries(&mapping)
        .into_iter()
        .filter(|entry| {
            category
                .as_deref()
                .is_none_or(|value| entry.category == value)
        })
        .filter(|entry| {
            query.as_deref().is_none_or(|query| {
                entry.original.to_lowercase().contains(query)
                    || entry.alias.to_lowercase().contains(query)
                    || entry.category.to_lowercase().contains(query)
                    || entry
                        .kind
                        .as_deref()
                        .is_some_and(|kind| kind.to_lowercase().contains(query))
            })
        })
        .take(limit)
        .collect();

    Ok(entries)
}

pub fn delete_mapping_entries(
    mapping_path: &Path,
    entries: &[PrivacyMappingEntryKey],
) -> Result<PrivacyMappingDeleteResult, String> {
    if entries.is_empty() {
        return Err(crate::i18n::text(
            "Не выбраны mapping entries для удаления",
            "No mapping entries were selected for deletion",
        )
        .into());
    }

    if entries.len() > 10_000 {
        return Err(crate::i18n::text(
            "За одну операцию можно удалить не более 10000 mapping entries",
            "A single operation may delete no more than 10,000 mapping entries",
        )
        .into());
    }

    let keys = entries
        .iter()
        .map(|entry| {
            let category = normalize_mapping_category(&entry.category)?;

            if entry.original.is_empty() {
                return Err(crate::i18n::text(
                    "Original mapping entry не может быть пустым",
                    "Original mapping entry cannot be empty",
                )
                .to_owned());
            }

            Ok((category.to_owned(), entry.original.clone()))
        })
        .collect::<Result<BTreeSet<_>, String>>()?;

    let _lock = MappingLock::acquire(mapping_path)?;
    let mut mapping = PrivacyMapping::load(mapping_path)?;
    let mut deleted = 0;
    let mut missing = 0;

    for (category, original) in keys {
        let removed = match category.as_str() {
            "identifier" => mapping.identifiers.remove(&original).is_some(),
            "string" => mapping.strings.remove(&original).is_some(),
            "integer" => mapping.integers.remove(&original).is_some(),
            "float" => mapping.floats.remove(&original).is_some(),
            "bitstring" => mapping.bitstrings.remove(&original).is_some(),
            "redaction" => mapping.redactions.remove(&original).is_some(),
            _ => unreachable!("validated mapping category"),
        };

        if removed {
            deleted += 1;
        } else {
            missing += 1;
        }
    }

    if deleted > 0 {
        mapping.save(mapping_path)?;
    }

    Ok(PrivacyMappingDeleteResult {
        mapping: mapping_info(mapping_path, &mapping),
        deleted,
        missing,
    })
}

fn all_mapping_entries(mapping: &PrivacyMapping) -> Vec<PrivacyMappingEntry> {
    let mut entries = Vec::new();

    for (original, record) in &mapping.identifiers {
        entries.push(PrivacyMappingEntry {
            alias: record.replacement.clone(),
            original: original.clone(),
            category: "identifier".into(),
            kind: Some(record.kind.clone()),
        });
    }

    for (original, alias) in &mapping.strings {
        entries.push(PrivacyMappingEntry {
            alias: alias.clone(),
            original: original.clone(),
            category: "string".into(),
            kind: None,
        });
    }

    for (original, alias) in &mapping.integers {
        entries.push(PrivacyMappingEntry {
            alias: alias.to_string(),
            original: original.clone(),
            category: "integer".into(),
            kind: None,
        });
    }

    for (original, alias) in &mapping.floats {
        entries.push(PrivacyMappingEntry {
            alias: alias.clone(),
            original: original.clone(),
            category: "float".into(),
            kind: None,
        });
    }

    for (original, alias) in &mapping.bitstrings {
        entries.push(PrivacyMappingEntry {
            alias: alias.clone(),
            original: original.clone(),
            category: "bitstring".into(),
            kind: None,
        });
    }

    for (original, alias) in &mapping.redactions {
        entries.push(PrivacyMappingEntry {
            alias: alias.clone(),
            original: original.clone(),
            category: "redaction".into(),
            kind: None,
        });
    }

    entries.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.original.cmp(&right.original))
            .then_with(|| left.alias.cmp(&right.alias))
    });

    entries
}

fn normalize_mapping_category_filter(category: Option<&str>) -> Result<Option<String>, String> {
    let Some(category) = category
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };

    if category == "all" {
        return Ok(None);
    }

    Ok(Some(normalize_mapping_category(category)?.to_owned()))
}

fn normalize_mapping_category(category: &str) -> Result<&'static str, String> {
    match category.trim() {
        "identifier" => Ok("identifier"),
        "string" => Ok("string"),
        "integer" => Ok("integer"),
        "float" => Ok("float"),
        "bitstring" => Ok("bitstring"),
        "redaction" => Ok("redaction"),
        other => Err(format!("Неизвестная mapping category: {other}")),
    }
}

fn mapping_entries_for_loaded_mapping(
    mapping: &PrivacyMapping,
    text: &str,
) -> Vec<PrivacyMappingEntry> {
    let mut entries = Vec::new();

    for (original, record) in &mapping.identifiers {
        if text.contains(&record.replacement) {
            entries.push(PrivacyMappingEntry {
                alias: record.replacement.clone(),
                original: original.clone(),
                category: "identifier".into(),
                kind: Some(record.kind.clone()),
            });
        }
    }

    for (original, alias) in &mapping.strings {
        if text.contains(alias) {
            entries.push(PrivacyMappingEntry {
                alias: alias.clone(),
                original: original.clone(),
                category: "string".into(),
                kind: None,
            });
        }
    }

    for (original, alias) in &mapping.integers {
        let alias = alias.to_string();
        if text.contains(&alias) {
            entries.push(PrivacyMappingEntry {
                alias,
                original: original.clone(),
                category: "integer".into(),
                kind: None,
            });
        }
    }

    for (original, alias) in &mapping.floats {
        if text.contains(alias) {
            entries.push(PrivacyMappingEntry {
                alias: alias.clone(),
                original: original.clone(),
                category: "float".into(),
                kind: None,
            });
        }
    }

    for (original, alias) in &mapping.bitstrings {
        if text.contains(alias) {
            entries.push(PrivacyMappingEntry {
                alias: alias.clone(),
                original: original.clone(),
                category: "bitstring".into(),
                kind: None,
            });
        }
    }

    for (original, alias) in &mapping.redactions {
        if text.contains(alias) {
            entries.push(PrivacyMappingEntry {
                alias: alias.clone(),
                original: original.clone(),
                category: "redaction".into(),
                kind: None,
            });
        }
    }

    entries.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.alias.cmp(&right.alias))
    });

    entries
}

pub fn mapping_info_from_path(mapping_path: &Path) -> Result<PrivacyMappingInfo, String> {
    let _lock = MappingLock::acquire(mapping_path)?;

    let mapping = PrivacyMapping::load(mapping_path)?;

    Ok(mapping_info(mapping_path, &mapping))
}

fn mapping_info(mapping_path: &Path, mapping: &PrivacyMapping) -> PrivacyMappingInfo {
    PrivacyMappingInfo {
        path: mapping_path.to_string_lossy().into_owned(),
        identifiers: mapping.identifiers.len(),
        strings: mapping.strings.len(),
        integers: mapping.integers.len(),
        floats: mapping.floats.len(),
        bitstrings: mapping.bitstrings.len(),
        redactions: mapping.redactions.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_mapping(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        let directory = std::env::temp_dir().join(format!(
            "pgxray-privacy-{name}-{}-{nonce}",
            std::process::id(),
        ));

        fs::create_dir_all(&directory).unwrap();

        directory.join("mapping.json")
    }

    #[test]
    fn text_round_trip_uses_one_mapping() {
        let mapping = temp_mapping("text");

        let source = "server=db-prod user=alice\n";

        let anonymous = transform(
            &mapping,
            PrivacyMode::Text,
            PrivacyOperation::Anonymize,
            source,
        )
        .unwrap();

        assert_ne!(anonymous.output, source,);

        let restored = transform(
            &mapping,
            PrivacyMode::Text,
            PrivacyOperation::Deanonymize,
            &anonymous.output,
        )
        .unwrap();

        assert_eq!(restored.output, source,);

        let _ = fs::remove_dir_all(mapping.parent().unwrap());
    }

    #[test]
    fn known_text_mode_preserves_message_without_creating_mapping_entries() {
        let path = temp_mapping("text-known");
        let mut seed = PrivacyMapping::default();
        let alias = seed.identifier("secret_orders", "table");
        seed.save(&path).unwrap();
        let before = fs::read(&path).unwrap();

        let message = "ERROR: relation \"secret_orders\" does not exist. Unknown: new_secret";
        let anonymous = transform(
            &path,
            PrivacyMode::TextKnown,
            PrivacyOperation::Anonymize,
            message,
        )
        .unwrap();
        assert_eq!(
            anonymous.output,
            format!("ERROR: relation \"{alias}\" does not exist. Unknown: new_secret"),
        );
        assert!(!anonymous.warnings.is_empty());
        assert_eq!(fs::read(&path).unwrap(), before);

        let restored = transform(
            &path,
            PrivacyMode::TextKnown,
            PrivacyOperation::Deanonymize,
            &anonymous.output,
        )
        .unwrap();
        assert_eq!(restored.output, message);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn sql_round_trip_preserves_structure() {
        let mapping = temp_mapping("sql");

        let source = "SELECT customer_id FROM private.orders WHERE customer_id = 42;";

        let anonymous = transform(
            &mapping,
            PrivacyMode::Sql,
            PrivacyOperation::Anonymize,
            source,
        )
        .unwrap();

        assert!(anonymous.output.contains("table_",),);

        assert!(!anonymous.output.contains("orders",),);

        let restored = transform(
            &mapping,
            PrivacyMode::Sql,
            PrivacyOperation::Deanonymize,
            &anonymous.output,
        )
        .unwrap();

        assert_eq!(restored.output, source,);

        let _ = fs::remove_dir_all(mapping.parent().unwrap());
    }

    #[test]
    fn explain_keeps_metrics_and_restores_names() {
        let mapping = temp_mapping("explain");

        let source = r#"[{"Plan":{"Node Type":"Seq Scan","Relation Name":"orders","Plan Rows":100,"Actual Rows":7,"Filter":"customer_id = 42"}}]"#;

        let anonymous = transform(
            &mapping,
            PrivacyMode::Explain,
            PrivacyOperation::Anonymize,
            source,
        )
        .unwrap();

        let anonymous_json: serde_json::Value = serde_json::from_str(&anonymous.output).unwrap();

        assert_eq!(anonymous_json[0]["Plan"]["Plan Rows"], 100,);

        assert_eq!(anonymous_json[0]["Plan"]["Actual Rows"], 7,);

        assert_ne!(anonymous_json[0]["Plan"]["Relation Name"], "orders",);

        let restored = transform(
            &mapping,
            PrivacyMode::Explain,
            PrivacyOperation::Deanonymize,
            &anonymous.output,
        )
        .unwrap();

        let restored_json: serde_json::Value = serde_json::from_str(&restored.output).unwrap();

        assert_eq!(restored_json[0]["Plan"]["Relation Name"], "orders",);

        let _ = fs::remove_dir_all(mapping.parent().unwrap());
    }

    #[test]
    fn result_column_stats_is_classified_automatically() {
        let mapping = temp_mapping("column-stats");

        let source = r#"[{
            "schema":"public",
            "table":"orders",
            "column":"customer_id",
            "null_frac":0.01,
            "n_distinct":12543,
            "most_common_vals":[123],
            "most_common_freqs":[0.012],
            "histogram_bounds":[1],
            "correlation":0.72
        }]"#;

        let anonymous = transform(
            &mapping,
            PrivacyMode::Result,
            PrivacyOperation::Anonymize,
            source,
        )
        .unwrap();

        let anonymous_json: serde_json::Value = serde_json::from_str(&anonymous.output).unwrap();

        assert_eq!(anonymous_json[0]["schema"], "public");
        assert_eq!(anonymous_json[0]["table"], "table_001");
        assert_eq!(anonymous_json[0]["column"], "column_001");
        assert_eq!(anonymous_json[0]["null_frac"], 0.01);

        let restored = transform(
            &mapping,
            PrivacyMode::Result,
            PrivacyOperation::Deanonymize,
            &anonymous.output,
        )
        .unwrap();

        let restored_json: serde_json::Value = serde_json::from_str(&restored.output).unwrap();
        let source_json: serde_json::Value = serde_json::from_str(source).unwrap();

        assert_eq!(restored_json, source_json);

        let _ = fs::remove_dir_all(mapping.parent().unwrap());
    }

    #[test]
    fn text_mode_reuses_mapping_created_by_sql_mode() {
        let mapping = temp_mapping("text-reuse");
        let sql = "SELECT customer_id FROM private.orders;";

        transform(&mapping, PrivacyMode::Sql, PrivacyOperation::Anonymize, sql).unwrap();

        let sql_mapping = PrivacyMapping::load(&mapping).unwrap();
        let table_alias = sql_mapping
            .existing_alias_for_original("orders")
            .expect("SQL mode should map table name");
        let column_alias = sql_mapping
            .existing_alias_for_original("customer_id")
            .expect("SQL mode should map selected identifier");

        let text = transform(
            &mapping,
            PrivacyMode::Text,
            PrivacyOperation::Anonymize,
            "orders\ncustomer_id",
        )
        .unwrap();

        assert_eq!(text.output, format!("{table_alias}\n{column_alias}"));

        let loaded = PrivacyMapping::load(&mapping).unwrap();
        assert!(loaded.strings.is_empty());

        let restored = transform(
            &mapping,
            PrivacyMode::Text,
            PrivacyOperation::Deanonymize,
            &text.output,
        )
        .unwrap();

        assert_eq!(restored.output, "orders\ncustomer_id");

        let _ = fs::remove_dir_all(mapping.parent().unwrap());
    }

    #[test]
    fn mapping_entries_can_be_searched_and_deleted_in_batch() {
        let mapping = temp_mapping("mapping-manager");
        let mut stored = PrivacyMapping::default();
        stored.identifier("orders", "table");
        stored.identifier("customer_id", "column");
        stored.string_value("secret");
        stored.save(&mapping).unwrap();

        let found = list_mapping_entries(&mapping, Some("orders"), None, 100).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].original, "orders");
        assert_eq!(found[0].category, "identifier");

        let result = delete_mapping_entries(
            &mapping,
            &[
                PrivacyMappingEntryKey {
                    category: "identifier".into(),
                    original: "orders".into(),
                },
                PrivacyMappingEntryKey {
                    category: "string".into(),
                    original: "secret".into(),
                },
            ],
        )
        .unwrap();

        assert_eq!(result.deleted, 2);
        assert_eq!(result.missing, 0);
        assert_eq!(result.mapping.identifiers, 1);
        assert_eq!(result.mapping.strings, 0);

        let remaining = list_mapping_entries(&mapping, None, None, 100).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].original, "customer_id");

        let mut reloaded = PrivacyMapping::load(&mapping).unwrap();
        assert_eq!(reloaded.string_value("new-secret"), "value_002");

        let _ = fs::remove_dir_all(mapping.parent().unwrap());
    }
}
