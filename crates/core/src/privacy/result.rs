use super::{mapping::PrivacyMapping, sql};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultPrivacyDefault {
    #[default]
    Auto,
    Preserve,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResultPrivacyFieldAction {
    Auto,
    Preserve,
    Value,
    Schema,
    Table,
    Column,
    Columns,
    Index,
    Database,
    Role,
    Application,
    Address,
    Tablespace,
    Sql,
    SqlFragment,
    Nested(ResultPrivacyNestedFieldConfig),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResultPrivacyPatternRule {
    pub pattern: String,
    pub action: ResultPrivacyFieldAction,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResultPrivacyNestedFieldConfig {
    /// Parse a string field as JSON before applying nested rules. This is useful
    /// for JSON/JSONB values printed by psql as a single text cell.
    #[serde(default)]
    pub json: bool,

    /// Default policy for fields inside this block.
    #[serde(default)]
    pub default: ResultPrivacyDefault,

    /// Action for keys of the current JSON object. Omitted means preserve keys.
    #[serde(default)]
    pub keys: Option<Box<ResultPrivacyFieldAction>>,

    /// Exact named fields inside the current JSON object.
    #[serde(default)]
    pub fields: BTreeMap<String, ResultPrivacyFieldAction>,

    /// Regex rules for field names. Exact `fields` rules have higher priority.
    #[serde(default)]
    pub patterns: Vec<ResultPrivacyPatternRule>,

    /// Policy for every immediate object value / array item when no exact or
    /// regex rule matches. Useful for dynamic maps such as tables keyed by name.
    #[serde(default)]
    pub values: Option<Box<ResultPrivacyFieldAction>>,
}

impl Default for ResultPrivacyNestedFieldConfig {
    fn default() -> Self {
        Self {
            json: false,
            default: ResultPrivacyDefault::Preserve,
            keys: None,
            fields: BTreeMap::new(),
            patterns: Vec::new(),
            values: None,
        }
    }
}

impl Serialize for ResultPrivacyFieldAction {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Nested(config) => config.serialize(serializer),
            action => serializer.serialize_str(action.as_str()),
        }
    }
}

impl<'de> Deserialize<'de> for ResultPrivacyFieldAction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;

        match value {
            Value::String(value) => Self::from_str(&value).ok_or_else(|| {
                serde::de::Error::custom(format!("Неизвестное result privacy action: {value:?}"))
            }),
            Value::Object(_) => serde_json::from_value::<ResultPrivacyNestedFieldConfig>(value)
                .map(Self::Nested)
                .map_err(serde::de::Error::custom),
            _ => Err(serde::de::Error::custom(
                "Result privacy field action должен быть строкой или object config",
            )),
        }
    }
}

impl ResultPrivacyFieldAction {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Preserve => "preserve",
            Self::Value => "value",
            Self::Schema => "schema",
            Self::Table => "table",
            Self::Column => "column",
            Self::Columns => "columns",
            Self::Index => "index",
            Self::Database => "database",
            Self::Role => "role",
            Self::Application => "application",
            Self::Address => "address",
            Self::Tablespace => "tablespace",
            Self::Sql => "sql",
            Self::SqlFragment => "sql_fragment",
            Self::Nested(_) => "nested",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "auto" => Self::Auto,
            "preserve" => Self::Preserve,
            "value" => Self::Value,
            "schema" => Self::Schema,
            "table" => Self::Table,
            "column" => Self::Column,
            "columns" => Self::Columns,
            "index" => Self::Index,
            "database" => Self::Database,
            "role" => Self::Role,
            "application" => Self::Application,
            "address" => Self::Address,
            "tablespace" => Self::Tablespace,
            "sql" => Self::Sql,
            "sql_fragment" => Self::SqlFragment,
            _ => return None,
        })
    }

    fn nested(&self) -> Option<&ResultPrivacyNestedFieldConfig> {
        match self {
            Self::Nested(config) => Some(config),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResultPrivacyConfig {
    #[serde(default)]
    pub default: ResultPrivacyDefault,
    /// Optional rule for outer JSON object keys (or keys of JSON array rows).
    /// Does not affect psql column headers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<Box<ResultPrivacyFieldAction>>,
    #[serde(default)]
    pub fields: BTreeMap<String, ResultPrivacyFieldAction>,
    #[serde(default)]
    pub patterns: Vec<ResultPrivacyPatternRule>,
}

impl ResultPrivacyConfig {
    pub fn normalized(self) -> Result<Self, String> {
        let mut field_count = 0_usize;
        if let Some(keys) = self.keys.as_deref() {
            validate_object_key_action(keys)?;
        }
        let fields = normalize_field_map(self.fields, 0, &mut field_count)?;
        let patterns = normalize_pattern_rules(self.patterns, 0, &mut field_count)?;

        Ok(Self {
            default: self.default,
            keys: self.keys,
            fields,
            patterns,
        })
    }
}

fn normalize_field_map(
    source: BTreeMap<String, ResultPrivacyFieldAction>,
    depth: usize,
    field_count: &mut usize,
) -> Result<BTreeMap<String, ResultPrivacyFieldAction>, String> {
    if depth > 32 {
        return Err(crate::i18n::text(
            "Result privacy policy содержит больше 32 уровней вложенности",
            "Result privacy policy exceeds 32 nesting levels",
        )
        .into());
    }

    if source.len() > 256 {
        return Err(crate::i18n::text(
            "Result privacy policy содержит больше 256 fields на одном уровне",
            "Result privacy policy exceeds 256 fields at one level",
        )
        .into());
    }

    let mut fields = BTreeMap::new();

    for (field, action) in source {
        *field_count = field_count.saturating_add(1);
        if *field_count > 2048 {
            return Err(crate::i18n::text(
                "Result privacy policy содержит больше 2048 field overrides",
                "Result privacy policy exceeds 2048 field overrides",
            )
            .into());
        }

        if field.len() > 128 {
            return Err(format!(
                "Имя result privacy field слишком длинное: {field:?}"
            ));
        }

        let normalized = normalize_field_name(&field);

        if normalized.is_empty() {
            return Err(format!("Некорректное имя result privacy field: {field:?}"));
        }

        let action = normalize_field_action(action, depth + 1, field_count)?;

        if fields.insert(normalized.clone(), action).is_some() {
            return Err(format!(
                "Result privacy policy содержит дублирующееся поле после нормализации: {normalized}"
            ));
        }
    }

    Ok(fields)
}

fn normalize_field_action(
    action: ResultPrivacyFieldAction,
    depth: usize,
    field_count: &mut usize,
) -> Result<ResultPrivacyFieldAction, String> {
    let ResultPrivacyFieldAction::Nested(config) = action else {
        return Ok(action);
    };

    if let Some(keys) = config.keys.as_deref() {
        validate_object_key_action(keys)?;
    }

    let keys = config
        .keys
        .map(|action| normalize_field_action(*action, depth + 1, field_count).map(Box::new))
        .transpose()?;
    let fields = normalize_field_map(config.fields, depth, field_count)?;
    let patterns = normalize_pattern_rules(config.patterns, depth, field_count)?;
    let values = config
        .values
        .map(|action| normalize_field_action(*action, depth + 1, field_count).map(Box::new))
        .transpose()?;

    Ok(ResultPrivacyFieldAction::Nested(
        ResultPrivacyNestedFieldConfig {
            json: config.json,
            default: config.default,
            keys,
            fields,
            patterns,
            values,
        },
    ))
}

fn normalize_pattern_rules(
    source: Vec<ResultPrivacyPatternRule>,
    depth: usize,
    field_count: &mut usize,
) -> Result<Vec<ResultPrivacyPatternRule>, String> {
    if source.len() > 256 {
        return Err(crate::i18n::text(
            "Result privacy policy содержит больше 256 regex rules на одном уровне",
            "Result privacy policy exceeds 256 regex rules at one level",
        )
        .into());
    }

    let mut patterns = Vec::with_capacity(source.len());
    for rule in source {
        *field_count = field_count.saturating_add(1);
        if *field_count > 2048 {
            return Err(crate::i18n::text(
                "Result privacy policy содержит больше 2048 field overrides",
                "Result privacy policy exceeds 2048 field overrides",
            )
            .into());
        }
        if rule.pattern.len() > 512 {
            return Err(crate::i18n::text(
                "Result privacy regex pattern длиннее 512 символов",
                "Result privacy regex pattern exceeds 512 characters",
            )
            .into());
        }
        Regex::new(&rule.pattern).map_err(|error| {
            format!(
                "Некорректный result privacy regex {:?}: {error}",
                rule.pattern
            )
        })?;
        patterns.push(ResultPrivacyPatternRule {
            pattern: rule.pattern,
            action: normalize_field_action(rule.action, depth + 1, field_count)?,
        });
    }
    Ok(patterns)
}

fn validate_object_key_action(action: &ResultPrivacyFieldAction) -> Result<(), String> {
    if matches!(
        action,
        ResultPrivacyFieldAction::Preserve
            | ResultPrivacyFieldAction::Value
            | ResultPrivacyFieldAction::Schema
            | ResultPrivacyFieldAction::Table
            | ResultPrivacyFieldAction::Column
            | ResultPrivacyFieldAction::Index
            | ResultPrivacyFieldAction::Database
            | ResultPrivacyFieldAction::Role
            | ResultPrivacyFieldAction::Application
            | ResultPrivacyFieldAction::Address
            | ResultPrivacyFieldAction::Tablespace
    ) {
        return Ok(());
    }

    Err(format!(
        "Result privacy object keys не поддерживают action {:?}; используйте preserve, value или одиночный identifier action",
        action.as_str(),
    ))
}

#[derive(Clone, Debug)]
struct PsqlAlignedTable {
    columns: Vec<String>,
    rows: Vec<Vec<Value>>,
    footer_row_count: Option<usize>,
    timing_line: Option<String>,
}

#[derive(Clone, Debug)]
struct PsqlExpandedTable {
    columns: Vec<String>,
    rows: Vec<Vec<Value>>,
    footer_row_count: Option<usize>,
    timing_line: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResultPrivacyKind {
    Safe,
    SafeArray,
    String,
    SensitiveValue,
    Identifier(&'static str),
    IdentifierArray(&'static str),
    Sql,
    SqlFragment,
    Nested,
}

#[derive(Clone, Copy, Debug)]
struct FieldDecision {
    policy: ResultPrivacyKind,
    fallback_sensitive: bool,
}

#[derive(Default)]
struct ResultTransformStats {
    fallback_sensitive_fields: BTreeSet<String>,
    redacted_sql_fields: BTreeSet<String>,
    redacted_sql_fragment_fields: BTreeSet<String>,
    sql_warnings: Vec<String>,
}

impl ResultTransformStats {
    fn warnings(self) -> Vec<String> {
        let mut warnings = Vec::new();

        if !self.fallback_sensitive_fields.is_empty() {
            warnings.push(format!(
                "Неизвестные string fields анонимизированы как sensitive values: {}.",
                self.fallback_sensitive_fields
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(", "),
            ));
        }

        if !self.redacted_sql_fields.is_empty() {
            warnings.push(format!(
                "SQL fields не удалось безопасно разобрать; значения полностью redacted: {}.",
                self.redacted_sql_fields
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(", "),
            ));
        }

        if !self.redacted_sql_fragment_fields.is_empty() {
            warnings.push(format!(
                "SQL fragment fields не удалось безопасно разобрать; значения полностью redacted: {}.",
                self.redacted_sql_fragment_fields
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(", "),
            ));
        }

        warnings.extend(self.sql_warnings);
        warnings
    }
}

pub fn anonymize_result(
    input: &Value,
    mapping: &mut PrivacyMapping,
) -> Result<(Value, Vec<String>), String> {
    anonymize_result_with_config(input, mapping, &ResultPrivacyConfig::default())
}

pub fn anonymize_result_with_config(
    input: &Value,
    mapping: &mut PrivacyMapping,
    config: &ResultPrivacyConfig,
) -> Result<(Value, Vec<String>), String> {
    validate_result_root(input)?;
    let config = config.clone().normalized()?;
    survey_identifiers(input, mapping, &config)?;

    let mut stats = ResultTransformStats::default();
    let output = transform_anonymize_root(input, mapping, &mut stats, &config)?;

    Ok((output, stats.warnings()))
}

pub fn deanonymize_result(input: &Value, mapping: &PrivacyMapping) -> Result<Value, String> {
    deanonymize_result_with_config(input, mapping, &ResultPrivacyConfig::default())
}

pub fn deanonymize_result_with_config(
    input: &Value,
    mapping: &PrivacyMapping,
    config: &ResultPrivacyConfig,
) -> Result<Value, String> {
    validate_result_root(input)?;
    let config = config.clone().normalized()?;

    transform_deanonymize_root(input, mapping, &config)
}

pub fn anonymize_result_json(
    input: &str,
    mapping: &mut PrivacyMapping,
) -> Result<(String, Vec<String>), String> {
    anonymize_result_json_with_config(input, mapping, &ResultPrivacyConfig::default())
}

pub fn anonymize_result_json_with_config(
    input: &str,
    mapping: &mut PrivacyMapping,
    config: &ResultPrivacyConfig,
) -> Result<(String, Vec<String>), String> {
    let value: Value = serde_json::from_str(input)
        .map_err(|error| format!("Diagnostic result JSON не разбирается: {error}"))?;

    let (output, warnings) = anonymize_result_with_config(&value, mapping, config)?;
    let output = serde_json::to_string_pretty(&output).map_err(|error| {
        format!("Не удалось сериализовать анонимизированный diagnostic result: {error}")
    })?;

    Ok((format!("{output}\n"), warnings))
}

pub fn deanonymize_result_json(input: &str, mapping: &PrivacyMapping) -> Result<String, String> {
    deanonymize_result_json_with_config(input, mapping, &ResultPrivacyConfig::default())
}

pub fn deanonymize_result_json_with_config(
    input: &str,
    mapping: &PrivacyMapping,
    config: &ResultPrivacyConfig,
) -> Result<String, String> {
    let value: Value = serde_json::from_str(input)
        .map_err(|error| format!("Anonymous diagnostic result JSON не разбирается: {error}"))?;

    let output = deanonymize_result_with_config(&value, mapping, config)?;
    let output = serde_json::to_string_pretty(&output).map_err(|error| {
        format!("Не удалось сериализовать deanonymized diagnostic result: {error}")
    })?;

    Ok(format!("{output}\n"))
}

pub fn anonymize_result_input(
    input: &str,
    mapping: &mut PrivacyMapping,
) -> Result<(String, Vec<String>), String> {
    anonymize_result_input_with_config(input, mapping, &ResultPrivacyConfig::default())
}

pub fn anonymize_result_input_with_config(
    input: &str,
    mapping: &mut PrivacyMapping,
    config: &ResultPrivacyConfig,
) -> Result<(String, Vec<String>), String> {
    let config = config.clone().normalized()?;

    match serde_json::from_str::<Value>(input) {
        Ok(_) => anonymize_result_json_with_config(input, mapping, &config),
        Err(json_error) => match parse_psql_expanded_table(input) {
            Ok(table) => anonymize_psql_expanded_table(&table, mapping, &config),
            Err(expanded_error) => {
                let table = parse_psql_aligned_table(input).map_err(|aligned_error| {
                    format!(
                        "Diagnostic result не распознан как JSON, psql expanded (\\gx) или psql aligned table. JSON: {json_error}; expanded: {expanded_error}; aligned: {aligned_error}"
                    )
                })?;

                anonymize_psql_aligned_table(&table, mapping, &config)
            }
        },
    }
}

pub fn deanonymize_result_input(input: &str, mapping: &PrivacyMapping) -> Result<String, String> {
    deanonymize_result_input_with_config(input, mapping, &ResultPrivacyConfig::default())
}

pub fn deanonymize_result_input_with_config(
    input: &str,
    mapping: &PrivacyMapping,
    config: &ResultPrivacyConfig,
) -> Result<String, String> {
    let config = config.clone().normalized()?;

    match serde_json::from_str::<Value>(input) {
        Ok(_) => deanonymize_result_json_with_config(input, mapping, &config),
        Err(json_error) => match parse_psql_expanded_table(input) {
            Ok(table) => deanonymize_psql_expanded_table(&table, mapping, &config),
            Err(expanded_error) => {
                let table = parse_psql_aligned_table(input).map_err(|aligned_error| {
                    format!(
                        "Anonymous diagnostic result не распознан как JSON, psql expanded (\\gx) или psql aligned table. JSON: {json_error}; expanded: {expanded_error}; aligned: {aligned_error}"
                    )
                })?;

                deanonymize_psql_aligned_table(&table, mapping, &config)
            }
        },
    }
}

fn parse_psql_expanded_table(input: &str) -> Result<PsqlExpandedTable, String> {
    let lines = input.lines().collect::<Vec<_>>();
    let mut index = 0_usize;

    while lines.get(index).is_some_and(|line| line.trim().is_empty()) {
        index = index.saturating_add(1);
    }

    if lines
        .get(index)
        .and_then(|line| parse_psql_footer(line.trim()))
        == Some(0)
    {
        let timing_line = parse_psql_trailing_metadata(&lines[index + 1..])?;

        return Ok(PsqlExpandedTable {
            columns: Vec::new(),
            rows: Vec::new(),
            footer_row_count: Some(0),
            timing_line,
        });
    }

    if !lines
        .get(index)
        .is_some_and(|line| parse_psql_expanded_record_marker(line).is_some())
    {
        return Err("первая содержательная строка не похожа на -[ RECORD N ]-".into());
    }

    let mut columns = Vec::<String>::new();
    let mut rows = Vec::<Vec<Value>>::new();
    let mut footer_row_count = None;
    let mut timing_line = None;
    let mut current_fields = Vec::<(String, String)>::new();

    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();

        if trimmed.is_empty() {
            index = index.saturating_add(1);
            continue;
        }

        if parse_psql_expanded_record_marker(line).is_some() {
            if !current_fields.is_empty() {
                push_psql_expanded_record(&mut columns, &mut rows, &mut current_fields)?;
            }

            index = index.saturating_add(1);
            continue;
        }

        if let Some(count) = parse_psql_footer(trimmed) {
            if !current_fields.is_empty() {
                push_psql_expanded_record(&mut columns, &mut rows, &mut current_fields)?;
            }

            footer_row_count = Some(count);
            index = index.saturating_add(1);
            break;
        }

        if is_psql_timing_line(trimmed) {
            if !current_fields.is_empty() {
                push_psql_expanded_record(&mut columns, &mut rows, &mut current_fields)?;
            }

            timing_line = Some(trimmed.to_owned());
            index = index.saturating_add(1);
            break;
        }

        let Some((left, right)) = line.split_once('|') else {
            return Err(format!(
                "строка expanded output не содержит separator |: {line:?}"
            ));
        };

        let field = left.trim();
        let value = right.trim().to_owned();

        if field.is_empty() {
            let Some((_, previous_value)) = current_fields.last_mut() else {
                return Err("continuation line встретилась до первого field".into());
            };

            // В стандартном ascii linestyle psql помечает реальный newline
            // знаком `+` в правом краю предыдущей physical line.
            if previous_value.ends_with('+') {
                previous_value.pop();
            }

            previous_value.push('\n');
            previous_value.push_str(&value);
        } else {
            current_fields.push((field.to_owned(), value));
        }

        index = index.saturating_add(1);
    }

    if !current_fields.is_empty() {
        push_psql_expanded_record(&mut columns, &mut rows, &mut current_fields)?;
    }

    if timing_line.is_none() {
        timing_line = parse_psql_trailing_metadata(&lines[index..])?;
        index = lines.len();
    } else {
        while lines.get(index).is_some_and(|line| line.trim().is_empty()) {
            index = index.saturating_add(1);
        }
    }

    if index < lines.len() {
        return Err("после expanded result обнаружены неожиданные строки".into());
    }

    if rows.is_empty() {
        return Err("expanded result не содержит records".into());
    }

    if let Some(expected) = footer_row_count {
        if expected != rows.len() {
            return Err(format!(
                "footer сообщает {expected} row(s), но разобрано {} expanded record(s)",
                rows.len(),
            ));
        }
    }

    Ok(PsqlExpandedTable {
        columns,
        rows,
        footer_row_count,
        timing_line,
    })
}

fn parse_psql_expanded_record_marker(line: &str) -> Option<usize> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("-[ RECORD ")?;
    let (number, suffix) = rest.split_once(" ]")?;

    // psql растягивает expanded header под ширину результата и вставляет `+`
    // в позициях разделителей, например `-[ RECORD 1 ]-------+-----------`.
    if suffix.is_empty()
        || !suffix
            .chars()
            .all(|character| matches!(character, '-' | '+'))
    {
        return None;
    }

    number.trim().parse::<usize>().ok()
}

fn push_psql_expanded_record(
    columns: &mut Vec<String>,
    rows: &mut Vec<Vec<Value>>,
    fields: &mut Vec<(String, String)>,
) -> Result<(), String> {
    if fields.is_empty() {
        return Err("expanded record не содержит fields".into());
    }

    if columns.is_empty() {
        columns.extend(fields.iter().map(|(field, _)| field.clone()));
    } else {
        if fields.len() != columns.len() {
            return Err(format!(
                "expanded record содержит {} field(s), ожидалось {}",
                fields.len(),
                columns.len(),
            ));
        }

        for (index, (field, _)) in fields.iter().enumerate() {
            if field != &columns[index] {
                return Err(format!(
                    "expanded record field #{} равен {field:?}, ожидалось {:?}",
                    index + 1,
                    columns[index],
                ));
            }
        }
    }

    rows.push(
        fields
            .drain(..)
            .map(|(_, value)| parse_psql_cell(value))
            .collect(),
    );

    Ok(())
}

fn parse_psql_aligned_table(input: &str) -> Result<PsqlAlignedTable, String> {
    let mut lines = input.lines().collect::<Vec<_>>();

    while lines.first().is_some_and(|line| line.trim().is_empty()) {
        lines.remove(0);
    }

    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }

    if lines
        .first()
        .and_then(|line| parse_psql_footer(line.trim()))
        == Some(0)
    {
        let timing_line = parse_psql_trailing_metadata(&lines[1..])?;

        return Ok(PsqlAlignedTable {
            columns: Vec::new(),
            rows: Vec::new(),
            footer_row_count: Some(0),
            timing_line,
        });
    }

    if lines.len() < 2 {
        return Err("слишком мало строк для psql aligned table".into());
    }

    let separator = lines[1];
    let widths = psql_separator_widths(separator)?;
    let columns = split_psql_fixed_line(lines[0], &widths)?;

    if columns.iter().any(|column| column.is_empty()) {
        return Err("psql table содержит колонку без имени".into());
    }

    let mut rows = Vec::new();
    let mut footer_row_count = None;
    let mut timing_line = None;
    let mut index = 2_usize;

    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();

        if let Some(count) = parse_psql_footer(trimmed) {
            footer_row_count = Some(count);
            index = index.saturating_add(1);
            break;
        }

        if is_psql_timing_line(trimmed) {
            timing_line = Some(trimmed.to_owned());
            index = index.saturating_add(1);
            break;
        }

        let cells = split_psql_fixed_line(line, &widths)?;
        rows.push(cells.into_iter().map(parse_psql_cell).collect());
        index = index.saturating_add(1);
    }

    if timing_line.is_none() {
        timing_line = parse_psql_trailing_metadata(&lines[index..])?;
    } else {
        parse_psql_trailing_metadata(&lines[index..])?;
    }

    if let Some(expected) = footer_row_count {
        if expected != rows.len() {
            return Err(format!(
                "footer сообщает {expected} row(s), но разобрано {} физических строк; wrapped/multiline psql output пока не поддерживается",
                rows.len(),
            ));
        }
    }

    Ok(PsqlAlignedTable {
        columns,
        rows,
        footer_row_count,
        timing_line,
    })
}

fn psql_separator_widths(line: &str) -> Result<Vec<usize>, String> {
    let line = line.trim_end();

    if line.is_empty()
        || !line.contains('-')
        || !line.chars().all(|character| matches!(character, '-' | '+'))
    {
        return Err("вторая строка не похожа на separator psql aligned table".into());
    }

    let widths = line
        .split('+')
        .map(|segment| segment.chars().count())
        .collect::<Vec<_>>();

    if widths.contains(&0) {
        return Err("psql separator содержит пустой segment".into());
    }

    Ok(widths)
}

fn parse_psql_footer(line: &str) -> Option<usize> {
    let inner = line.strip_prefix('(')?.strip_suffix(')')?;

    let mut parts = inner.split_whitespace();
    let count = parts.next()?.parse::<usize>().ok()?;
    let unit = parts.next()?;

    if !matches!(unit, "row" | "rows") || parts.next().is_some() {
        return None;
    }

    Some(count)
}

fn is_psql_timing_line(line: &str) -> bool {
    line.strip_prefix("Time:")
        .is_some_and(|value| !value.trim().is_empty())
}

fn parse_psql_trailing_metadata(lines: &[&str]) -> Result<Option<String>, String> {
    let mut timing_line = None;

    for line in lines {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            continue;
        }

        if is_psql_timing_line(trimmed) && timing_line.is_none() {
            timing_line = Some(trimmed.to_owned());
            continue;
        }

        return Err(format!(
            "после psql result обнаружена неожиданная строка: {line:?}"
        ));
    }

    Ok(timing_line)
}

fn split_psql_fixed_line(line: &str, widths: &[usize]) -> Result<Vec<String>, String> {
    let characters = line.chars().collect::<Vec<_>>();
    let mut cursor = 0_usize;
    let mut cells = Vec::with_capacity(widths.len());

    for (index, width) in widths.iter().copied().enumerate() {
        let end = cursor.saturating_add(width);

        if index + 1 < widths.len() && characters.len() <= end {
            return Err(format!(
                "psql row слишком короткая перед колонкой #{}; возможно, output был wrapped",
                index + 1,
            ));
        }

        let available_end = end.min(characters.len());
        let mut cell = characters[cursor..available_end].iter().collect::<String>();

        if available_end < end {
            cell.push_str(&" ".repeat(end - available_end));
        }

        cells.push(cell.trim().to_owned());
        cursor = end;

        if index + 1 < widths.len() {
            if characters.get(cursor) != Some(&'|') {
                return Err(format!(
                    "psql row не содержит separator | после колонки #{}; возможно, output был wrapped",
                    index + 1,
                ));
            }

            cursor = cursor.saturating_add(1);
        }
    }

    if characters[cursor.min(characters.len())..]
        .iter()
        .any(|character| !character.is_whitespace())
    {
        return Err("psql row содержит данные за последней колонкой".into());
    }

    Ok(cells)
}

fn parse_psql_cell(cell: String) -> Value {
    if cell == "∅" {
        return Value::Null;
    }

    if cell == "t" {
        return Value::Bool(true);
    }

    if cell == "f" {
        return Value::Bool(false);
    }

    if let Ok(value) = cell.parse::<i64>() {
        return Value::Number(value.into());
    }

    if let Ok(value) = cell.parse::<u64>() {
        return Value::Number(value.into());
    }

    if cell.contains('.') || cell.contains('e') || cell.contains('E') {
        if let Ok(value) = cell.parse::<f64>() {
            if let Some(value) = serde_json::Number::from_f64(value) {
                return Value::Number(value);
            }
        }
    }

    Value::String(cell)
}

fn anonymize_psql_expanded_table(
    table: &PsqlExpandedTable,
    mapping: &mut PrivacyMapping,
    config: &ResultPrivacyConfig,
) -> Result<(String, Vec<String>), String> {
    let aligned = PsqlAlignedTable {
        columns: table.columns.clone(),
        rows: table.rows.clone(),
        footer_row_count: table.footer_row_count,
        timing_line: table.timing_line.clone(),
    };

    survey_psql_identifiers(&aligned, mapping, config)?;

    let mut stats = ResultTransformStats::default();
    let rows = transform_psql_rows(&table.columns, &table.rows, mapping, &mut stats, config)?;

    Ok((
        render_psql_expanded_table(
            &table.columns,
            &rows,
            table.footer_row_count.is_some(),
            table.timing_line.as_deref(),
        )?,
        stats.warnings(),
    ))
}

fn deanonymize_psql_expanded_table(
    table: &PsqlExpandedTable,
    mapping: &PrivacyMapping,
    config: &ResultPrivacyConfig,
) -> Result<String, String> {
    let rows = deanonymize_psql_rows(&table.columns, &table.rows, mapping, config)?;

    render_psql_expanded_table(
        &table.columns,
        &rows,
        table.footer_row_count.is_some(),
        table.timing_line.as_deref(),
    )
}

fn transform_psql_rows(
    columns: &[String],
    rows: &[Vec<Value>],
    mapping: &mut PrivacyMapping,
    stats: &mut ResultTransformStats,
    config: &ResultPrivacyConfig,
) -> Result<Vec<Vec<Value>>, String> {
    let mut transformed_rows = Vec::with_capacity(rows.len());

    for row in rows {
        let mut transformed = Vec::with_capacity(row.len());

        for (index, value) in row.iter().enumerate() {
            let field = &columns[index];
            let decision = field_policy(field, value, config);

            if decision.fallback_sensitive {
                stats.fallback_sensitive_fields.insert(field.clone());
            }

            transformed.push(anonymize_field(
                field,
                value,
                decision.policy,
                mapping,
                stats,
                config,
                resolve_root_field_action(field, config).and_then(|action| action.nested()),
            )?);
        }

        transformed_rows.push(transformed);
    }

    Ok(transformed_rows)
}

fn deanonymize_psql_rows(
    columns: &[String],
    rows: &[Vec<Value>],
    mapping: &PrivacyMapping,
    config: &ResultPrivacyConfig,
) -> Result<Vec<Vec<Value>>, String> {
    let mut transformed_rows = Vec::with_capacity(rows.len());

    for row in rows {
        let mut transformed = Vec::with_capacity(row.len());

        for (index, value) in row.iter().enumerate() {
            let field = &columns[index];
            let decision = field_policy(field, value, config);

            transformed.push(deanonymize_field(
                field,
                value,
                decision.policy,
                mapping,
                config,
                resolve_root_field_action(field, config).and_then(|action| action.nested()),
            )?);
        }

        transformed_rows.push(transformed);
    }

    Ok(transformed_rows)
}

fn render_psql_expanded_table(
    columns: &[String],
    rows: &[Vec<Value>],
    had_footer: bool,
    timing_line: Option<&str>,
) -> Result<String, String> {
    let field_width = columns
        .iter()
        .map(|column| column.chars().count())
        .max()
        .unwrap_or(0);
    let mut output = String::new();

    for (row_index, row) in rows.iter().enumerate() {
        output.push_str(&format!("-[ RECORD {} ]-\n", row_index + 1));

        for (column_index, column) in columns.iter().enumerate() {
            let text = psql_expanded_cell_text(&row[column_index])?;
            let mut lines = text.split('\n');
            let first = lines.next().unwrap_or("");

            output.push_str(&format!(
                "{column:<field_width$} | {first}\n",
                field_width = field_width,
            ));

            for continuation in lines {
                output.push_str(&format!(
                    "{empty:<field_width$} | {continuation}\n",
                    empty = "",
                    field_width = field_width,
                ));
            }
        }
    }

    if had_footer {
        output.push_str(&format!(
            "({} {})\n",
            rows.len(),
            if rows.len() == 1 { "row" } else { "rows" },
        ));
    }

    if let Some(timing_line) = timing_line {
        output.push_str(timing_line);
        output.push('\n');
    }

    Ok(output)
}

fn psql_expanded_cell_text(value: &Value) -> Result<String, String> {
    match value {
        Value::Null => Ok("∅".into()),
        Value::Bool(true) => Ok("t".into()),
        Value::Bool(false) => Ok("f".into()),
        Value::Number(value) => Ok(value.to_string()),
        Value::String(value) => Ok(value.clone()),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).map_err(|error| {
            format!("Не удалось сериализовать psql expanded result cell: {error}")
        }),
    }
}

fn anonymize_psql_aligned_table(
    table: &PsqlAlignedTable,
    mapping: &mut PrivacyMapping,
    config: &ResultPrivacyConfig,
) -> Result<(String, Vec<String>), String> {
    survey_psql_identifiers(table, mapping, config)?;

    let mut stats = ResultTransformStats::default();
    let rows = transform_psql_rows(&table.columns, &table.rows, mapping, &mut stats, config)?;

    Ok((
        render_psql_aligned_table(
            &table.columns,
            &rows,
            table.footer_row_count.is_some(),
            table.timing_line.as_deref(),
        )?,
        stats.warnings(),
    ))
}

fn deanonymize_psql_aligned_table(
    table: &PsqlAlignedTable,
    mapping: &PrivacyMapping,
    config: &ResultPrivacyConfig,
) -> Result<String, String> {
    let rows = deanonymize_psql_rows(&table.columns, &table.rows, mapping, config)?;

    render_psql_aligned_table(
        &table.columns,
        &rows,
        table.footer_row_count.is_some(),
        table.timing_line.as_deref(),
    )
}

fn survey_psql_identifiers(
    table: &PsqlAlignedTable,
    mapping: &mut PrivacyMapping,
    config: &ResultPrivacyConfig,
) -> Result<(), String> {
    for row in &table.rows {
        for (index, value) in row.iter().enumerate() {
            let field = &table.columns[index];
            let action = resolve_root_field_action(field, config);
            survey_value_with_action(field, value, action, mapping, config, None)?;
        }
    }

    Ok(())
}

fn render_psql_aligned_table(
    columns: &[String],
    rows: &[Vec<Value>],
    had_footer: bool,
    timing_line: Option<&str>,
) -> Result<String, String> {
    if columns.is_empty() && rows.is_empty() {
        let mut output = String::new();

        if had_footer {
            output.push_str("(0 rows)\n");
        }

        if let Some(timing_line) = timing_line {
            output.push_str(timing_line);
            output.push('\n');
        }

        return Ok(output);
    }

    let text_rows = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(psql_cell_text)
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut widths = columns
        .iter()
        .map(|column| column.chars().count())
        .collect::<Vec<_>>();

    for row in &text_rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.chars().count());
        }
    }

    let numeric_columns = (0..columns.len())
        .map(|index| {
            rows.iter().any(|row| row[index].is_number())
                && rows.iter().all(|row| {
                    row[index].is_number() || row[index].as_str().is_some_and(str::is_empty)
                })
        })
        .collect::<Vec<_>>();

    let mut output = String::new();

    output.push_str(&render_psql_row(
        columns,
        &widths,
        &vec![false; columns.len()],
    ));
    output.push('\n');
    output.push_str(
        &widths
            .iter()
            .map(|width| "-".repeat(*width))
            .collect::<Vec<_>>()
            .join("-+-"),
    );
    output.push('\n');

    for row in &text_rows {
        output.push_str(&render_psql_row(row, &widths, &numeric_columns));
        output.push('\n');
    }

    if had_footer {
        output.push_str(&format!(
            "({} {})\n",
            rows.len(),
            if rows.len() == 1 { "row" } else { "rows" },
        ));
    }

    if let Some(timing_line) = timing_line {
        output.push_str(timing_line);
        output.push('\n');
    }

    Ok(output)
}

fn render_psql_row(cells: &[String], widths: &[usize], right_align: &[bool]) -> String {
    cells
        .iter()
        .enumerate()
        .map(|(index, cell)| {
            let padding = widths[index].saturating_sub(cell.chars().count());

            if right_align[index] {
                format!("{}{}", " ".repeat(padding), cell)
            } else {
                format!("{}{}", cell, " ".repeat(padding))
            }
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn psql_cell_text(value: &Value) -> Result<String, String> {
    match value {
        Value::Null => Ok("∅".into()),
        Value::Bool(true) => Ok("t".into()),
        Value::Bool(false) => Ok("f".into()),
        Value::Number(value) => Ok(value.to_string()),
        Value::String(value) => {
            if value.contains('\n') || value.contains('\r') {
                return Err(
                    "psql aligned output с multiline cell пока не поддерживается; используйте JSON для такого результата"
                        .into(),
                );
            }

            Ok(value.clone())
        }
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value)
            .map_err(|error| format!("Не удалось сериализовать psql result cell: {error}")),
    }
}

fn validate_result_root(input: &Value) -> Result<(), String> {
    if matches!(input, Value::Array(_) | Value::Object(_)) {
        return Ok(());
    }

    Err("Diagnostic result должен быть JSON object или array".into())
}

fn field_policy(field: &str, value: &Value, config: &ResultPrivacyConfig) -> FieldDecision {
    let action = resolve_root_field_action(field, config);
    field_policy_from_action(action, field, value, config)
}

fn resolve_root_field_action<'a>(
    field: &str,
    config: &'a ResultPrivacyConfig,
) -> Option<&'a ResultPrivacyFieldAction> {
    let normalized = normalize_field_name(field);
    if let Some(action) = config.fields.get(&normalized) {
        return Some(action);
    }
    for rule in &config.patterns {
        if Regex::new(&rule.pattern).is_ok_and(|pattern| pattern.is_match(field)) {
            return Some(&rule.action);
        }
    }
    None
}

fn field_policy_from_action(
    action: Option<&ResultPrivacyFieldAction>,
    field: &str,
    value: &Value,
    config: &ResultPrivacyConfig,
) -> FieldDecision {
    if let Some(action) = action {
        return field_action_policy(action, field, value);
    }

    match config.default {
        ResultPrivacyDefault::Auto => generic_field_policy(field, value),
        ResultPrivacyDefault::Preserve => preserve_default_policy(value),
    }
}

fn field_policy_in_scope(
    action: Option<&ResultPrivacyFieldAction>,
    field: &str,
    value: &Value,
    config: &ResultPrivacyConfig,
    scope: Option<&ResultPrivacyNestedFieldConfig>,
) -> FieldDecision {
    if let Some(action) = action {
        return field_action_policy(action, field, value);
    }

    match scope.map(|scope| scope.default).unwrap_or(config.default) {
        ResultPrivacyDefault::Auto => generic_field_policy(field, value),
        ResultPrivacyDefault::Preserve => preserve_default_policy(value),
    }
}

fn resolve_field_action<'a>(
    field: &str,
    config: &'a ResultPrivacyConfig,
    scope: Option<&'a ResultPrivacyNestedFieldConfig>,
) -> Option<&'a ResultPrivacyFieldAction> {
    let normalized = normalize_field_name(field);

    if let Some(scope) = scope {
        if let Some(action) = scope.fields.get(&normalized) {
            return Some(action);
        }

        for rule in &scope.patterns {
            if Regex::new(&rule.pattern).is_ok_and(|pattern| pattern.is_match(field)) {
                return Some(&rule.action);
            }
        }

        if let Some(action) = scope.values.as_deref() {
            return Some(action);
        }

        return None;
    }

    resolve_root_field_action(field, config)
}

fn field_action_policy(
    action: &ResultPrivacyFieldAction,
    field: &str,
    value: &Value,
) -> FieldDecision {
    let policy = match action {
        ResultPrivacyFieldAction::Auto => return generic_field_policy(field, value),
        ResultPrivacyFieldAction::Preserve => ResultPrivacyKind::Safe,
        ResultPrivacyFieldAction::Value => ResultPrivacyKind::SensitiveValue,
        ResultPrivacyFieldAction::Schema => ResultPrivacyKind::Identifier("schema"),
        ResultPrivacyFieldAction::Table => ResultPrivacyKind::Identifier("table"),
        ResultPrivacyFieldAction::Column => ResultPrivacyKind::Identifier("column"),
        ResultPrivacyFieldAction::Columns => ResultPrivacyKind::IdentifierArray("column"),
        ResultPrivacyFieldAction::Index => ResultPrivacyKind::Identifier("index"),
        ResultPrivacyFieldAction::Database => ResultPrivacyKind::Identifier("database"),
        ResultPrivacyFieldAction::Role => ResultPrivacyKind::Identifier("role"),
        ResultPrivacyFieldAction::Application => ResultPrivacyKind::Identifier("application"),
        ResultPrivacyFieldAction::Address => ResultPrivacyKind::Identifier("address"),
        ResultPrivacyFieldAction::Tablespace => ResultPrivacyKind::Identifier("tablespace"),
        ResultPrivacyFieldAction::Sql => ResultPrivacyKind::Sql,
        ResultPrivacyFieldAction::SqlFragment => ResultPrivacyKind::SqlFragment,
        ResultPrivacyFieldAction::Nested(_) => ResultPrivacyKind::Nested,
    };

    FieldDecision {
        policy,
        fallback_sensitive: false,
    }
}

fn preserve_default_policy(value: &Value) -> FieldDecision {
    FieldDecision {
        policy: if contains_object_value(value) {
            ResultPrivacyKind::Nested
        } else {
            ResultPrivacyKind::Safe
        },
        fallback_sensitive: false,
    }
}

fn generic_field_policy(field: &str, value: &Value) -> FieldDecision {
    let normalized = normalize_field_name(field);

    let policy = match normalized.as_str() {
        "query" | "query_text" | "query_string" | "statement" | "statement_text" | "sql"
        | "sql_text" | "index_definition" => Some(ResultPrivacyKind::Sql),

        "predicate" | "filter" | "condition" | "where_clause" | "index_cond" | "recheck_cond"
        | "join_filter" | "hash_cond" | "merge_cond" => Some(ResultPrivacyKind::SqlFragment),

        "schema" | "schemaname" | "schema_name" => Some(ResultPrivacyKind::Identifier("schema")),

        "table" | "tablename" | "table_name" | "relname" | "relation" | "relation_name" => {
            Some(ResultPrivacyKind::Identifier("table"))
        }

        "index" | "indexname" | "index_name" => Some(ResultPrivacyKind::Identifier("index")),

        "column" | "column_name" | "attname" => Some(ResultPrivacyKind::Identifier("column")),

        "columns" | "column_names" | "attnames" => {
            Some(ResultPrivacyKind::IdentifierArray("column"))
        }

        "database" | "datname" | "database_name" => Some(ResultPrivacyKind::Identifier("database")),

        "user" | "username" | "user_name" | "usename" | "role" | "rolname" | "role_name" => {
            Some(ResultPrivacyKind::Identifier("role"))
        }

        "application" | "application_name" => Some(ResultPrivacyKind::Identifier("application")),

        "client_addr" | "client_address" | "client_host" | "client_hostname" | "host"
        | "hostname" | "server_addr" | "server_address" => {
            Some(ResultPrivacyKind::Identifier("address"))
        }

        "tablespace" | "tablespace_name" | "spcname" => {
            Some(ResultPrivacyKind::Identifier("tablespace"))
        }

        "state" | "wait_event" | "wait_event_type" | "backend_type" | "locktype" | "lock_type"
        | "mode" | "sync_state" | "command" | "status" | "phase" | "backend_start"
        | "xact_start" | "query_start" | "state_change" | "stats_since" | "stats_reset"
        | "last_vacuum" | "last_autovacuum" | "last_analyze" | "last_autoanalyze"
        | "last_seq_scan" | "last_idx_scan" | "last_archived_time" | "last_failed_time"
        | "write_lag" | "flush_lag" | "replay_lag" => Some(ResultPrivacyKind::Safe),

        "pid" | "leader_pid" | "dbid" | "userid" | "queryid" | "query_id" | "relid"
        | "indexrelid" | "classid" | "objid" | "objsubid" | "backend_xid" | "backend_xmin"
        | "most_common_vals" | "histogram_bounds" | "most_common_elems" => {
            Some(ResultPrivacyKind::SensitiveValue)
        }

        _ if normalized == "id"
            || normalized == "ids"
            || normalized.ends_with("_id")
            || normalized.ends_with("_ids")
            || normalized.ends_with("_oid")
            || normalized.ends_with("_oids")
            || normalized.ends_with("_code")
            || normalized.ends_with("_number")
            || normalized == "internal_account" =>
        {
            Some(ResultPrivacyKind::SensitiveValue)
        }

        _ => None,
    };

    if let Some(policy) = policy {
        return FieldDecision {
            policy,
            fallback_sensitive: false,
        };
    }

    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => FieldDecision {
            policy: ResultPrivacyKind::Safe,
            fallback_sensitive: false,
        },

        Value::String(_) => FieldDecision {
            policy: ResultPrivacyKind::String,
            fallback_sensitive: true,
        },

        Value::Object(_) => FieldDecision {
            policy: ResultPrivacyKind::Nested,
            fallback_sensitive: false,
        },

        Value::Array(values) => {
            if values
                .iter()
                .all(|value| matches!(value, Value::Null | Value::Bool(_) | Value::Number(_)))
            {
                FieldDecision {
                    policy: ResultPrivacyKind::SafeArray,
                    fallback_sensitive: false,
                }
            } else if values.iter().any(|value| matches!(value, Value::Object(_))) {
                FieldDecision {
                    policy: ResultPrivacyKind::Nested,
                    fallback_sensitive: false,
                }
            } else {
                FieldDecision {
                    policy: ResultPrivacyKind::SensitiveValue,
                    fallback_sensitive: values.iter().any(contains_string_value),
                }
            }
        }
    }
}

fn normalize_field_name(field: &str) -> String {
    let mut normalized = String::with_capacity(field.len());
    let mut previous_underscore = false;

    for character in field.trim().chars() {
        let mapped = if character.is_ascii_alphanumeric() {
            character.to_ascii_lowercase()
        } else {
            '_'
        };

        if mapped == '_' {
            if previous_underscore {
                continue;
            }

            previous_underscore = true;
        } else {
            previous_underscore = false;
        }

        normalized.push(mapped);
    }

    normalized.trim_matches('_').to_owned()
}

fn contains_object_value(value: &Value) -> bool {
    match value {
        Value::Object(_) => true,
        Value::Array(values) => values.iter().any(contains_object_value),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}

fn contains_string_value(value: &Value) -> bool {
    match value {
        Value::String(_) => true,
        Value::Array(values) => values.iter().any(contains_string_value),
        Value::Object(values) => values.values().any(contains_string_value),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

fn survey_identifiers(
    input: &Value,
    mapping: &mut PrivacyMapping,
    config: &ResultPrivacyConfig,
) -> Result<(), String> {
    survey_identifiers_nested(input, mapping, config, None)
}

fn survey_identifiers_nested(
    value: &Value,
    mapping: &mut PrivacyMapping,
    config: &ResultPrivacyConfig,
    scope: Option<&ResultPrivacyNestedFieldConfig>,
) -> Result<(), String> {
    match value {
        Value::Array(values) => {
            if let Some(each) = scope.and_then(|scope| scope.values.as_deref()) {
                for value in values {
                    survey_value_with_action("<item>", value, Some(each), mapping, config, scope)?;
                }
            } else {
                for value in values {
                    survey_identifiers_nested(value, mapping, config, scope)?;
                }
            }
        }

        Value::Object(object) => {
            if let Some(key_action) = scope.and_then(|scope| scope.keys.as_deref()) {
                for field in object.keys() {
                    survey_object_key(field, key_action, mapping)?;
                }
            }

            for (field, value) in object {
                let action = resolve_field_action(field, config, scope);
                survey_value_with_action(field, value, action, mapping, config, scope)?;
            }
        }

        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }

    Ok(())
}

fn survey_value_with_action(
    field: &str,
    value: &Value,
    action: Option<&ResultPrivacyFieldAction>,
    mapping: &mut PrivacyMapping,
    config: &ResultPrivacyConfig,
    scope: Option<&ResultPrivacyNestedFieldConfig>,
) -> Result<(), String> {
    let decision = field_policy_in_scope(action, field, value, config, scope);

    match decision.policy {
        ResultPrivacyKind::Identifier(identifier_kind) => {
            if let Some(value) = value.as_str() {
                if identifier_kind != "schema" || !is_safe_schema(value) {
                    mapping.identifier(value, identifier_kind);
                }
            }
        }

        ResultPrivacyKind::IdentifierArray(identifier_kind) => {
            survey_identifier_array_value(field, value, identifier_kind, mapping)?;
        }

        ResultPrivacyKind::Nested => {
            let nested_scope = action.and_then(|action| action.nested());
            if nested_scope.is_some_and(|scope| scope.json) {
                match value {
                    Value::Null => {}
                    Value::String(value) => {
                        let parsed: Value = serde_json::from_str(value).map_err(|error| {
                            format!("Diagnostic result field {field:?} помечен как JSON, но не разбирается: {error}")
                        })?;
                        survey_identifiers_nested(&parsed, mapping, config, nested_scope)?;
                    }
                    Value::Object(_) | Value::Array(_) => {
                        survey_identifiers_nested(value, mapping, config, nested_scope)?;
                    }
                    _ => return Err(format!(
                        "Diagnostic result field {field:?} помечен как JSON и должен быть string/object/array/null"
                    )),
                }
            } else {
                survey_identifiers_nested(value, mapping, config, nested_scope)?;
            }
        }

        ResultPrivacyKind::Safe
        | ResultPrivacyKind::SafeArray
        | ResultPrivacyKind::String
        | ResultPrivacyKind::SensitiveValue
        | ResultPrivacyKind::Sql
        | ResultPrivacyKind::SqlFragment => {}
    }

    Ok(())
}

fn survey_object_key(
    key: &str,
    action: &ResultPrivacyFieldAction,
    mapping: &mut PrivacyMapping,
) -> Result<(), String> {
    match action {
        ResultPrivacyFieldAction::Preserve => {}
        ResultPrivacyFieldAction::Value => {
            anonymize_string_value(key, mapping);
        }
        ResultPrivacyFieldAction::Schema => {
            if !is_safe_schema(key) {
                mapping.identifier(key, "schema");
            }
        }
        ResultPrivacyFieldAction::Table => {
            mapping.identifier(key, "table");
        }
        ResultPrivacyFieldAction::Column => {
            mapping.identifier(key, "column");
        }
        ResultPrivacyFieldAction::Index => {
            mapping.identifier(key, "index");
        }
        ResultPrivacyFieldAction::Database => {
            mapping.identifier(key, "database");
        }
        ResultPrivacyFieldAction::Role => {
            mapping.identifier(key, "role");
        }
        ResultPrivacyFieldAction::Application => {
            mapping.identifier(key, "application");
        }
        ResultPrivacyFieldAction::Address => {
            mapping.identifier(key, "address");
        }
        ResultPrivacyFieldAction::Tablespace => {
            mapping.identifier(key, "tablespace");
        }
        _ => validate_object_key_action(action)?,
    }

    Ok(())
}

fn transform_anonymize_root(
    input: &Value,
    mapping: &mut PrivacyMapping,
    stats: &mut ResultTransformStats,
    config: &ResultPrivacyConfig,
) -> Result<Value, String> {
    // Apply root `keys` only at the outer object (or each array row).
    // Nested fields use their own optional `keys` action.
    let root_scope = ResultPrivacyNestedFieldConfig {
        default: config.default,
        keys: config.keys.clone(),
        fields: config.fields.clone(),
        patterns: config.patterns.clone(),
        ..ResultPrivacyNestedFieldConfig::default()
    };
    let root_scope_ref = config.keys.as_ref().map(|_| &root_scope);
    match input {
        Value::Array(values) => values
            .iter()
            .map(|value| transform_anonymize_nested(value, mapping, stats, config, root_scope_ref))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),

        Value::Object(_) => {
            transform_anonymize_nested(input, mapping, stats, config, root_scope_ref)
        }

        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            unreachable!("validate_result_root rejected scalar diagnostic result")
        }
    }
}

fn transform_anonymize_nested(
    value: &Value,
    mapping: &mut PrivacyMapping,
    stats: &mut ResultTransformStats,
    config: &ResultPrivacyConfig,
    scope: Option<&ResultPrivacyNestedFieldConfig>,
) -> Result<Value, String> {
    match value {
        Value::Object(object) => {
            let mut transformed = Map::new();
            let key_action = scope.and_then(|scope| scope.keys.as_deref());

            for (field, value) in object {
                let action = resolve_field_action(field, config, scope);
                let decision = field_policy_in_scope(action, field, value, config, scope);

                if decision.fallback_sensitive {
                    stats.fallback_sensitive_fields.insert(field.clone());
                }

                let output_field = match key_action {
                    Some(action) => anonymize_object_key(field, action, mapping)?,
                    None => field.clone(),
                };
                let output_value = anonymize_field(
                    field,
                    value,
                    decision.policy,
                    mapping,
                    stats,
                    config,
                    action.and_then(|action| action.nested()),
                )?;

                if transformed
                    .insert(output_field.clone(), output_value)
                    .is_some()
                {
                    return Err(format!(
                        "После анонимизации два JSON key преобразовались в один key {output_field:?}"
                    ));
                }
            }

            Ok(Value::Object(transformed))
        }

        Value::Array(values) => {
            if let Some(each) = scope.and_then(|scope| scope.values.as_deref()) {
                values
                    .iter()
                    .map(|value| {
                        let decision = field_action_policy(each, "<item>", value);
                        anonymize_field(
                            "<item>",
                            value,
                            decision.policy,
                            mapping,
                            stats,
                            config,
                            each.nested(),
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(Value::Array)
            } else {
                values
                    .iter()
                    .map(|value| transform_anonymize_nested(value, mapping, stats, config, scope))
                    .collect::<Result<Vec<_>, _>>()
                    .map(Value::Array)
            }
        }

        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            if scope.is_some() {
                return Err(
                    "Nested result privacy config можно применять только к JSON object или array"
                        .into(),
                );
            }

            match config.default {
                ResultPrivacyDefault::Auto => anonymize_sensitive_value(value, mapping),
                ResultPrivacyDefault::Preserve => Ok(value.clone()),
            }
        }
    }
}

fn transform_deanonymize_root(
    input: &Value,
    mapping: &PrivacyMapping,
    config: &ResultPrivacyConfig,
) -> Result<Value, String> {
    let root_scope = ResultPrivacyNestedFieldConfig {
        default: config.default,
        keys: config.keys.clone(),
        fields: config.fields.clone(),
        patterns: config.patterns.clone(),
        ..ResultPrivacyNestedFieldConfig::default()
    };
    let root_scope_ref = config.keys.as_ref().map(|_| &root_scope);
    match input {
        Value::Array(values) => values
            .iter()
            .map(|value| transform_deanonymize_nested(value, mapping, config, root_scope_ref))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),

        Value::Object(_) => transform_deanonymize_nested(input, mapping, config, root_scope_ref),

        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            unreachable!("validate_result_root rejected scalar diagnostic result")
        }
    }
}

fn transform_deanonymize_nested(
    value: &Value,
    mapping: &PrivacyMapping,
    config: &ResultPrivacyConfig,
    scope: Option<&ResultPrivacyNestedFieldConfig>,
) -> Result<Value, String> {
    match value {
        Value::Object(object) => {
            let mut transformed = Map::new();
            let key_action = scope.and_then(|scope| scope.keys.as_deref());

            for (field, value) in object {
                let original_field = match key_action {
                    Some(action) => deanonymize_object_key(field, action, mapping)?,
                    None => field.clone(),
                };
                let action = resolve_field_action(&original_field, config, scope);
                let decision = field_policy_in_scope(action, &original_field, value, config, scope);
                let output_value = deanonymize_field(
                    &original_field,
                    value,
                    decision.policy,
                    mapping,
                    config,
                    action.and_then(|action| action.nested()),
                )?;

                if transformed
                    .insert(original_field.clone(), output_value)
                    .is_some()
                {
                    return Err(format!(
                        "После deanonymize два JSON key преобразовались в один key {original_field:?}"
                    ));
                }
            }

            Ok(Value::Object(transformed))
        }

        Value::Array(values) => {
            if let Some(each) = scope.and_then(|scope| scope.values.as_deref()) {
                values
                    .iter()
                    .map(|value| {
                        let decision = field_action_policy(each, "<item>", value);
                        deanonymize_field(
                            "<item>",
                            value,
                            decision.policy,
                            mapping,
                            config,
                            each.nested(),
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(Value::Array)
            } else {
                values
                    .iter()
                    .map(|value| transform_deanonymize_nested(value, mapping, config, scope))
                    .collect::<Result<Vec<_>, _>>()
                    .map(Value::Array)
            }
        }

        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            if scope.is_some() {
                return Err(
                    "Nested result privacy config можно применять только к JSON object или array"
                        .into(),
                );
            }

            match config.default {
                ResultPrivacyDefault::Auto => deanonymize_sensitive_value(value, mapping),
                ResultPrivacyDefault::Preserve => Ok(value.clone()),
            }
        }
    }
}

fn anonymize_object_key(
    key: &str,
    action: &ResultPrivacyFieldAction,
    mapping: &mut PrivacyMapping,
) -> Result<String, String> {
    let transformed = match action {
        ResultPrivacyFieldAction::Preserve => key.to_owned(),
        ResultPrivacyFieldAction::Value => anonymize_string_value(key, mapping),
        ResultPrivacyFieldAction::Schema if is_safe_schema(key) => key.to_owned(),
        ResultPrivacyFieldAction::Schema => mapping.identifier(key, "schema"),
        ResultPrivacyFieldAction::Table => mapping.identifier(key, "table"),
        ResultPrivacyFieldAction::Column => mapping.identifier(key, "column"),
        ResultPrivacyFieldAction::Index => mapping.identifier(key, "index"),
        ResultPrivacyFieldAction::Database => mapping.identifier(key, "database"),
        ResultPrivacyFieldAction::Role => mapping.identifier(key, "role"),
        ResultPrivacyFieldAction::Application => mapping.identifier(key, "application"),
        ResultPrivacyFieldAction::Address => mapping.identifier(key, "address"),
        ResultPrivacyFieldAction::Tablespace => mapping.identifier(key, "tablespace"),
        _ => {
            validate_object_key_action(action)?;
            unreachable!("validate_object_key_action accepted unsupported action")
        }
    };

    Ok(transformed)
}

fn deanonymize_object_key(
    key: &str,
    action: &ResultPrivacyFieldAction,
    mapping: &PrivacyMapping,
) -> Result<String, String> {
    let transformed = match action {
        ResultPrivacyFieldAction::Preserve => key.to_owned(),
        ResultPrivacyFieldAction::Value => {
            mapping.original_for_alias(key).unwrap_or(key).to_owned()
        }
        ResultPrivacyFieldAction::Schema
        | ResultPrivacyFieldAction::Table
        | ResultPrivacyFieldAction::Column
        | ResultPrivacyFieldAction::Index
        | ResultPrivacyFieldAction::Database
        | ResultPrivacyFieldAction::Role
        | ResultPrivacyFieldAction::Application
        | ResultPrivacyFieldAction::Address
        | ResultPrivacyFieldAction::Tablespace => {
            mapping.reverse_identifier(key).unwrap_or(key).to_owned()
        }
        _ => {
            validate_object_key_action(action)?;
            unreachable!("validate_object_key_action accepted unsupported action")
        }
    };

    Ok(transformed)
}

fn anonymize_field(
    field: &str,
    value: &Value,
    policy: ResultPrivacyKind,
    mapping: &mut PrivacyMapping,
    stats: &mut ResultTransformStats,
    config: &ResultPrivacyConfig,
    nested_scope: Option<&ResultPrivacyNestedFieldConfig>,
) -> Result<Value, String> {
    match policy {
        ResultPrivacyKind::Safe => Ok(value.clone()),

        ResultPrivacyKind::SafeArray => match value {
            Value::Null => Ok(Value::Null),
            Value::Array(values)
                if values.iter().all(|value| {
                    matches!(value, Value::Null | Value::Bool(_) | Value::Number(_))
                }) =>
            {
                Ok(Value::Array(values.clone()))
            }
            _ => Err(format!(
                "Diagnostic result field {field:?} должен содержать safe scalar array"
            )),
        },

        ResultPrivacyKind::String => match value {
            Value::Null => Ok(Value::Null),
            Value::String(value) => Ok(Value::String(anonymize_string_value(value, mapping))),
            _ => Err(format!(
                "Diagnostic result field {field:?} должен быть string или null"
            )),
        },

        ResultPrivacyKind::SensitiveValue => anonymize_sensitive_value(value, mapping),

        ResultPrivacyKind::Identifier(identifier_kind) => match value {
            Value::Null => Ok(Value::Null),
            Value::String(value) => Ok(Value::String(
                if identifier_kind == "schema" && is_safe_schema(value) {
                    value.to_owned()
                } else {
                    mapping.identifier(value, identifier_kind)
                },
            )),
            _ => Err(format!(
                "Diagnostic result field {field:?} должен быть identifier string или null"
            )),
        },

        ResultPrivacyKind::IdentifierArray(identifier_kind) => match value {
            Value::Null => Ok(Value::Null),
            Value::Array(values) => {
                let transformed = values
                    .iter()
                    .map(|value| match value {
                        Value::Null => Ok(Value::Null),
                        Value::String(value) => {
                            Ok(Value::String(mapping.identifier(value, identifier_kind)))
                        }
                        _ => Err(format!(
                            "Diagnostic result field {field:?} должен содержать identifier strings"
                        )),
                    })
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(Value::Array(transformed))
            }
            Value::String(value) => {
                anonymize_identifier_list_string(value, identifier_kind, mapping)
                    .map(Value::String)
                    .map_err(|error| format!("Diagnostic result field {field:?}: {error}"))
            }
            _ => Err(format!(
                "Diagnostic result field {field:?} должен быть identifier array/string или null"
            )),
        },

        ResultPrivacyKind::Sql => match value {
            Value::Null => Ok(Value::Null),
            Value::String(value) => anonymize_sql_field(field, value, mapping, stats),
            _ => Err(format!(
                "Diagnostic result SQL field {field:?} должен быть string или null"
            )),
        },

        ResultPrivacyKind::SqlFragment => match value {
            Value::Null => Ok(Value::Null),
            Value::String(value) => anonymize_sql_fragment_field(field, value, mapping, stats),
            _ => Err(format!(
                "Diagnostic result SQL fragment field {field:?} должен быть string или null"
            )),
        },

        ResultPrivacyKind::Nested => {
            if nested_scope.is_some_and(|scope| scope.json) {
                match value {
                    Value::Null => Ok(Value::Null),
                    Value::String(value) => {
                        let parsed: Value = serde_json::from_str(value).map_err(|error| {
                            format!("Diagnostic result field {field:?} помечен как JSON, но не разбирается: {error}")
                        })?;
                        let transformed = transform_anonymize_nested(
                            &parsed, mapping, stats, config, nested_scope,
                        )?;
                        let output = serde_json::to_string(&transformed).map_err(|error| {
                            format!("Не удалось сериализовать JSON field {field:?}: {error}")
                        })?;
                        Ok(Value::String(output))
                    }
                    Value::Object(_) | Value::Array(_) => {
                        transform_anonymize_nested(value, mapping, stats, config, nested_scope)
                    }
                    _ => Err(format!(
                        "Diagnostic result field {field:?} помечен как JSON и должен быть string/object/array/null"
                    )),
                }
            } else {
                transform_anonymize_nested(value, mapping, stats, config, nested_scope)
            }
        }
    }
}

fn deanonymize_field(
    field: &str,
    value: &Value,
    policy: ResultPrivacyKind,
    mapping: &PrivacyMapping,
    config: &ResultPrivacyConfig,
    nested_scope: Option<&ResultPrivacyNestedFieldConfig>,
) -> Result<Value, String> {
    match policy {
        ResultPrivacyKind::Safe | ResultPrivacyKind::SafeArray => Ok(value.clone()),

        ResultPrivacyKind::String => match value {
            Value::Null => Ok(Value::Null),
            Value::String(value) => Ok(Value::String(
                mapping.original_for_alias(value).unwrap_or(value).to_owned(),
            )),
            _ => Err(format!(
                "Anonymous diagnostic result field {field:?} должен быть string или null"
            )),
        },

        ResultPrivacyKind::SensitiveValue => deanonymize_sensitive_value(value, mapping),

        ResultPrivacyKind::Identifier(_) => match value {
            Value::Null => Ok(Value::Null),
            Value::String(value) => Ok(Value::String(
                mapping
                    .reverse_identifier(value)
                    .unwrap_or(value)
                    .to_owned(),
            )),
            _ => Err(format!(
                "Anonymous diagnostic result field {field:?} должен быть identifier string или null"
            )),
        },

        ResultPrivacyKind::IdentifierArray(_) => match value {
            Value::Null => Ok(Value::Null),
            Value::Array(values) => values
                .iter()
                .map(|value| match value {
                    Value::Null => Ok(Value::Null),
                    Value::String(value) => Ok(Value::String(
                        mapping
                            .reverse_identifier(value)
                            .unwrap_or(value)
                            .to_owned(),
                    )),
                    _ => Err(format!(
                        "Anonymous diagnostic result field {field:?} должен содержать identifier strings"
                    )),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            Value::String(value) => Ok(Value::String(deanonymize_identifier_list_string(
                value, mapping,
            ))),
            _ => Err(format!(
                "Anonymous diagnostic result field {field:?} должен быть identifier array/string или null"
            )),
        },

        ResultPrivacyKind::Sql => match value {
            Value::Null => Ok(Value::Null),
            Value::String(value) => {
                if let Some(original) = mapping.reverse_redaction(value) {
                    return Ok(Value::String(original.to_owned()));
                }

                Ok(Value::String(sql::deanonymize(value, mapping)?))
            }
            _ => Err(format!(
                "Anonymous diagnostic result SQL field {field:?} должен быть string или null"
            )),
        },

        ResultPrivacyKind::SqlFragment => match value {
            Value::Null => Ok(Value::Null),
            Value::String(value) => {
                if let Some(original) = mapping.reverse_redaction(value) {
                    return Ok(Value::String(original.to_owned()));
                }

                Ok(Value::String(sql::deanonymize_fragment(value, mapping)?))
            }
            _ => Err(format!(
                "Anonymous diagnostic result SQL fragment field {field:?} должен быть string или null"
            )),
        },

        ResultPrivacyKind::Nested => {
            if nested_scope.is_some_and(|scope| scope.json) {
                match value {
                    Value::Null => Ok(Value::Null),
                    Value::String(value) => {
                        let parsed: Value = serde_json::from_str(value).map_err(|error| {
                            format!("Anonymous diagnostic result field {field:?} помечен как JSON, но не разбирается: {error}")
                        })?;
                        let transformed = transform_deanonymize_nested(
                            &parsed, mapping, config, nested_scope,
                        )?;
                        let output = serde_json::to_string(&transformed).map_err(|error| {
                            format!("Не удалось сериализовать deanonymized JSON field {field:?}: {error}")
                        })?;
                        Ok(Value::String(output))
                    }
                    Value::Object(_) | Value::Array(_) => {
                        transform_deanonymize_nested(value, mapping, config, nested_scope)
                    }
                    _ => Err(format!(
                        "Anonymous diagnostic result field {field:?} помечен как JSON и должен быть string/object/array/null"
                    )),
                }
            } else {
                transform_deanonymize_nested(value, mapping, config, nested_scope)
            }
        }
    }
}

fn identifier_list_parts(value: &str) -> Result<Option<Vec<&str>>, String> {
    let trimmed = value.trim();

    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("NO PRIMARY KEY") {
        return Ok(None);
    }

    let parts = trimmed.split(',').map(str::trim).collect::<Vec<_>>();

    if parts.iter().any(|part| part.is_empty()) {
        return Err("identifier list содержит пустой элемент".into());
    }

    Ok(Some(parts))
}

fn survey_identifier_array_value(
    field: &str,
    value: &Value,
    identifier_kind: &str,
    mapping: &mut PrivacyMapping,
) -> Result<(), String> {
    match value {
        Value::Null => Ok(()),
        Value::Array(values) => {
            for value in values {
                match value {
                    Value::Null => {}
                    Value::String(value) => {
                        mapping.identifier(value, identifier_kind);
                    }
                    _ => {
                        return Err(format!(
                            "Diagnostic result field {field:?} должен содержать identifier strings"
                        ));
                    }
                }
            }
            Ok(())
        }
        Value::String(value) => {
            if let Some(parts) = identifier_list_parts(value)? {
                for part in parts {
                    mapping.identifier(part, identifier_kind);
                }
            }
            Ok(())
        }
        _ => Err(format!(
            "Diagnostic result field {field:?} должен быть identifier array/string или null"
        )),
    }
}

fn anonymize_identifier_list_string(
    value: &str,
    identifier_kind: &str,
    mapping: &mut PrivacyMapping,
) -> Result<String, String> {
    let Some(parts) = identifier_list_parts(value)? else {
        return Ok(value.to_owned());
    };

    Ok(parts
        .into_iter()
        .map(|part| mapping.identifier(part, identifier_kind))
        .collect::<Vec<_>>()
        .join(", "))
}

fn deanonymize_identifier_list_string(value: &str, mapping: &PrivacyMapping) -> String {
    let Ok(Some(parts)) = identifier_list_parts(value) else {
        return value.to_owned();
    };

    parts
        .into_iter()
        .map(|part| mapping.reverse_identifier(part).unwrap_or(part).to_owned())
        .collect::<Vec<_>>()
        .join(", ")
}

fn anonymize_sql_field(
    field: &str,
    value: &str,
    mapping: &mut PrivacyMapping,
    stats: &mut ResultTransformStats,
) -> Result<Value, String> {
    let mut candidate = mapping.clone();

    match sql::anonymize(value, &mut candidate) {
        Ok((output, warnings)) => {
            *mapping = candidate;
            stats.sql_warnings.extend(warnings);
            Ok(Value::String(output))
        }
        Err(_) => {
            stats.redacted_sql_fields.insert(field.to_owned());
            Ok(Value::String(mapping.redaction(value)))
        }
    }
}

fn anonymize_sql_fragment_field(
    field: &str,
    value: &str,
    mapping: &mut PrivacyMapping,
    stats: &mut ResultTransformStats,
) -> Result<Value, String> {
    let mut candidate = mapping.clone();

    match sql::anonymize_fragment(value, &mut candidate) {
        Ok(output) => {
            *mapping = candidate;
            Ok(Value::String(output))
        }
        Err(_) => {
            stats.redacted_sql_fragment_fields.insert(field.to_owned());
            Ok(Value::String(mapping.redaction(value)))
        }
    }
}

fn anonymize_string_value(value: &str, mapping: &mut PrivacyMapping) -> String {
    if mapping.original_for_alias(value).is_some() {
        return value.to_owned();
    }

    mapping
        .existing_alias_for_original(value)
        .unwrap_or_else(|| mapping.string_value(value))
}

fn anonymize_sensitive_value(value: &Value, mapping: &mut PrivacyMapping) -> Result<Value, String> {
    match value {
        Value::Null => Ok(Value::Null),
        Value::Bool(value) => Ok(Value::Bool(*value)),
        Value::String(value) => Ok(Value::String(anonymize_string_value(value, mapping))),

        Value::Number(value) => {
            let original = value.to_string();

            if value.is_i64() || value.is_u64() {
                return Ok(Value::Number(mapping.integer_value(&original).into()));
            }

            let replacement = mapping.float_value(&original);

            serde_json::from_str::<Value>(&replacement).map_err(|error| {
                format!(
                    "Не удалось преобразовать anonymized numeric value {replacement:?}: {error}"
                )
            })
        }

        Value::Array(values) => values
            .iter()
            .map(|value| anonymize_sensitive_value(value, mapping))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),

        Value::Object(object) => {
            let mut transformed = Map::new();

            for (key, value) in object {
                transformed.insert(key.clone(), anonymize_sensitive_value(value, mapping)?);
            }

            Ok(Value::Object(transformed))
        }
    }
}

fn deanonymize_sensitive_value(value: &Value, mapping: &PrivacyMapping) -> Result<Value, String> {
    match value {
        Value::Null => Ok(Value::Null),
        Value::Bool(value) => Ok(Value::Bool(*value)),
        Value::String(value) => Ok(Value::String(
            mapping
                .original_for_alias(value)
                .unwrap_or(value)
                .to_owned(),
        )),

        Value::Number(value) => {
            let replacement = value.to_string();

            let original = mapping
                .reverse_integer(&replacement)
                .or_else(|| mapping.reverse_float(&replacement));

            let Some(original) = original else {
                return Ok(Value::Number(value.clone()));
            };

            serde_json::from_str::<Value>(original).map_err(|error| {
                format!("Не удалось восстановить numeric value {original:?}: {error}")
            })
        }

        Value::Array(values) => values
            .iter()
            .map(|value| deanonymize_sensitive_value(value, mapping))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),

        Value::Object(object) => {
            let mut transformed = Map::new();

            for (key, value) in object {
                transformed.insert(key.clone(), deanonymize_sensitive_value(value, mapping)?);
            }

            Ok(Value::Object(transformed))
        }
    }
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
    use serde_json::json;

    #[test]
    fn indexes_preserve_metrics_and_anonymize_semantic_fields() {
        let source = json!([
            {
                "schema": "shop",
                "table": "orders",
                "index": "orders_customer_id_idx",
                "columns": ["customer_id"],
                "index_definition":
                    "CREATE UNIQUE INDEX orders_customer_id_idx ON shop.orders USING btree (customer_id)",
                "size_bytes": 16777216,
                "valid": true,
                "ready": true,
                "unique": true,
                "primary": false
            }
        ]);

        let mut mapping = PrivacyMapping::default();
        let (anonymous, warnings) = anonymize_result(&source, &mut mapping).unwrap();

        assert_eq!(anonymous[0]["schema"], "schema_001");
        assert_eq!(anonymous[0]["table"], "table_001");
        assert_eq!(anonymous[0]["index"], "index_001");
        assert_eq!(anonymous[0]["columns"][0], "column_001");
        assert_eq!(anonymous[0]["size_bytes"], 16777216);
        assert_eq!(anonymous[0]["valid"], true);
        assert_eq!(anonymous[0]["unique"], true);
        assert!(warnings.is_empty());

        let definition = anonymous[0]["index_definition"].as_str().unwrap();
        assert!(definition.contains("index_001"));
        assert!(definition.contains("schema_001.table_001"));
        assert!(definition.contains("column_001"));
    }

    #[test]
    fn indexes_reuse_existing_mapping() {
        let source = json!([
            {
                "table": "orders",
                "index": "orders_customer_id_idx",
                "columns": ["customer_id"],
                "size_bytes": 42,
                "valid": true
            }
        ]);

        let mut mapping = PrivacyMapping::default();
        assert_eq!(mapping.identifier("orders", "table"), "table_001");

        let (anonymous, _) = anonymize_result(&source, &mut mapping).unwrap();

        assert_eq!(anonymous[0]["table"], "table_001");
    }

    #[test]
    fn indexes_round_trip() {
        let source = json!([
            {
                "schema": "shop",
                "table": "orders",
                "index": "orders_customer_id_idx",
                "columns": ["customer_id"],
                "index_definition": "CREATE INDEX orders_customer_id_idx ON shop.orders (customer_id)",
                "predicate": "deleted_at IS NULL",
                "size_bytes": 16777216,
                "valid": true,
                "ready": true,
                "unique": false,
                "primary": false
            }
        ]);

        let mut mapping = PrivacyMapping::default();
        let (anonymous, _) = anonymize_result(&source, &mut mapping).unwrap();
        let restored = deanonymize_result(&anonymous, &mapping).unwrap();

        assert_eq!(restored, source);
    }

    #[test]
    fn column_stats_preserve_planner_metrics_and_anonymize_data_values() {
        let source = json!([
            {
                "schema": "public",
                "table": "orders",
                "column": "customer_id",
                "null_frac": 0.01,
                "n_distinct": 12543,
                "most_common_vals": [123, 456, "vip"],
                "most_common_freqs": [0.012, 0.01, 0.008],
                "histogram_bounds": [1, 1000, 5000],
                "correlation": 0.72
            }
        ]);

        let mut mapping = PrivacyMapping::default();
        let (anonymous, warnings) = anonymize_result(&source, &mut mapping).unwrap();

        assert_eq!(anonymous[0]["schema"], "public");
        assert_eq!(anonymous[0]["table"], "table_001");
        assert_eq!(anonymous[0]["column"], "column_001");
        assert_eq!(anonymous[0]["null_frac"], 0.01);
        assert_eq!(anonymous[0]["n_distinct"], 12543);
        assert_eq!(
            anonymous[0]["most_common_freqs"],
            json!([0.012, 0.01, 0.008])
        );
        assert_eq!(anonymous[0]["correlation"], 0.72);
        assert_ne!(anonymous[0]["most_common_vals"][0], 123);
        assert_ne!(anonymous[0]["most_common_vals"][1], 456);
        assert_ne!(anonymous[0]["most_common_vals"][2], "vip");
        assert_ne!(anonymous[0]["histogram_bounds"][0], 1);
        assert!(warnings.is_empty());
    }

    #[test]
    fn column_stats_round_trip() {
        let source = json!([
            {
                "schema": "public",
                "table": "orders",
                "column": "customer_id",
                "null_frac": 0.01,
                "n_distinct": 12543,
                "most_common_vals": [123, 456, "vip"],
                "most_common_freqs": [0.012, 0.01, 0.008],
                "histogram_bounds": [1, 1000, 5000],
                "correlation": 0.72
            }
        ]);

        let mut mapping = PrivacyMapping::default();
        let (anonymous, _) = anonymize_result(&source, &mut mapping).unwrap();
        let restored = deanonymize_result(&anonymous, &mapping).unwrap();

        assert_eq!(restored, source);
    }

    #[test]
    fn pg_stat_activity_preserves_metrics_and_anonymizes_identity_and_query() {
        let source = json!([
            {
                "pid": 12345,
                "datname": "demo_db",
                "usename": "demo_reader",
                "application_name": "demo-app",
                "client_addr": "192.0.2.27",
                "state": "active",
                "wait_event_type": "Lock",
                "wait_event": "transactionid",
                "query": "SELECT email FROM private.users WHERE email = 'sample@example.test'",
                "query_start": "2020-01-01T12:00:00Z"
            }
        ]);

        let mut mapping = PrivacyMapping::default();
        let (anonymous, warnings) = anonymize_result(&source, &mut mapping).unwrap();

        assert_ne!(anonymous[0]["pid"], 12345);
        assert_eq!(anonymous[0]["datname"], "database_001");
        assert_eq!(anonymous[0]["usename"], "role_001");
        assert_eq!(anonymous[0]["application_name"], "application_001");
        assert_eq!(anonymous[0]["client_addr"], "address_001");
        assert_eq!(anonymous[0]["state"], "active");
        assert_eq!(anonymous[0]["wait_event_type"], "Lock");
        assert_eq!(anonymous[0]["wait_event"], "transactionid");
        assert_eq!(anonymous[0]["query_start"], "2020-01-01T12:00:00Z");

        let query = anonymous[0]["query"].as_str().unwrap();
        assert!(!query.contains("private"));
        assert!(!query.contains("users"));
        assert!(!query.contains("sample@example.test"));
        assert!(warnings.is_empty());

        let restored = deanonymize_result(&anonymous, &mapping).unwrap();
        assert_eq!(restored, source);
    }

    #[test]
    fn pg_stat_statements_keeps_statistics_and_reuses_query_mapping() {
        let source = json!([
            {
                "userid": 10,
                "dbid": 16384,
                "queryid": "918273645",
                "datname": "demo_db",
                "usename": "reporting",
                "query": "SELECT * FROM private.orders WHERE customer_id = 123456",
                "calls": 8000,
                "rows": 500000,
                "total_exec_time": 80000.0,
                "mean_exec_time": 10.0,
                "shared_blks_hit": 1234567,
                "shared_blks_read": 4321
            }
        ]);

        let mut mapping = PrivacyMapping::default();
        let (anonymous, warnings) = anonymize_result(&source, &mut mapping).unwrap();

        assert_eq!(anonymous[0]["calls"], 8000);
        assert_eq!(anonymous[0]["rows"], 500000);
        assert_eq!(anonymous[0]["total_exec_time"], 80000.0);
        assert_ne!(anonymous[0]["queryid"], "918273645");
        assert_eq!(anonymous[0]["datname"], "database_001");
        assert_eq!(anonymous[0]["usename"], "role_001");
        assert!(anonymous[0]["query"]
            .as_str()
            .unwrap()
            .contains("table_001"));
        assert!(warnings.is_empty());
    }

    #[test]
    fn unknown_strings_are_conservatively_anonymized() {
        let source = json!([
            {
                "calls": 10,
                "custom_label": "tenant-acme",
                "nested": {
                    "owner_note": "private incident"
                }
            }
        ]);

        let mut mapping = PrivacyMapping::default();
        let (anonymous, warnings) = anonymize_result(&source, &mut mapping).unwrap();

        assert_eq!(anonymous[0]["calls"], 10);
        assert_ne!(anonymous[0]["custom_label"], "tenant-acme");
        assert_ne!(anonymous[0]["nested"]["owner_note"], "private incident");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("custom_label"));
        assert!(warnings[0].contains("owner_note"));
    }

    #[test]
    fn id_like_numeric_fields_are_pseudonymized_but_metrics_stay_exact() {
        let source = json!([
            {
                "customer_id": 123456,
                "calls": 123456,
                "rows": 42
            }
        ]);

        let mut mapping = PrivacyMapping::default();
        let (anonymous, warnings) = anonymize_result(&source, &mut mapping).unwrap();

        assert_ne!(anonymous[0]["customer_id"], 123456);
        assert_eq!(anonymous[0]["calls"], 123456);
        assert_eq!(anonymous[0]["rows"], 42);
        assert!(warnings.is_empty());

        let restored = deanonymize_result(&anonymous, &mapping).unwrap();
        assert_eq!(restored, source);
    }

    #[test]
    fn numeric_business_codes_are_hidden_without_changing_dba_metrics() {
        let source = json!([{
            "customer_code": 987654321,
            "contract_number": 456789012,
            "internal_account": 1122334455,
            "employee_number": 87542,
            "calls": 350,
            "execution_time_ms": 18.75,
            "rows": 1200
        }]);
        let mut mapping = PrivacyMapping::default();
        let (anonymous, warnings) = anonymize_result(&source, &mut mapping).unwrap();
        for key in [
            "customer_code",
            "contract_number",
            "internal_account",
            "employee_number",
        ] {
            assert_ne!(
                anonymous[0][key], source[0][key],
                "{key} must be anonymized"
            );
        }
        for key in ["calls", "execution_time_ms", "rows"] {
            assert_eq!(
                anonymous[0][key], source[0][key],
                "{key} must remain diagnostic"
            );
        }
        assert!(warnings.is_empty());
        assert_eq!(deanonymize_result(&anonymous, &mapping).unwrap(), source);
    }

    #[test]
    fn dynamic_json_keys_can_be_anonymized_with_nested_policy() {
        let source = json!({
            "customer-demo-a": {
                "database_name": "demo_db",
                "tables": {"sample_orders": {"table_name": "sample_orders", "rows": 123}}
            }
        });
        let config: ResultPrivacyConfig = serde_json::from_value(json!({
            "default": "auto",
            "keys": "value",
            "patterns": [{
                "pattern": "^customer-.*$",
                "action": {
                    "default": "auto",
                    "fields": {"tables": {"keys": "table", "default": "auto"}}
                }
            }]
        }))
        .unwrap();
        let mut mapping = PrivacyMapping::default();
        let (anonymous, _) = anonymize_result_with_config(&source, &mut mapping, &config).unwrap();
        assert!(anonymous.get("customer-demo-a").is_none());
        let customer = anonymous.as_object().unwrap().values().next().unwrap();
        assert_eq!(customer["database_name"], "database_001");
        let tables = customer["tables"].as_object().unwrap();
        assert!(tables.get("sample_orders").is_none());
        let anonymized_table = tables.values().next().unwrap();
        assert_eq!(anonymized_table["table_name"], "table_001");
        assert_eq!(anonymized_table["rows"], 123);
        assert_eq!(
            deanonymize_result_with_config(&anonymous, &mapping, &config).unwrap(),
            source
        );
    }

    #[test]
    fn psql_aligned_numeric_business_code_is_hidden_but_rows_stay_exact() {
        let source = " customer_code | calls | rows\n---------------+-------+------\n 987654321     | 350   | 1200\n(1 row)\n";
        let mut mapping = PrivacyMapping::default();
        let (anonymous, _) = anonymize_result_input(source, &mut mapping).unwrap();
        assert!(!anonymous.contains("987654321"));
        assert!(anonymous.contains("350"));
        assert!(anonymous.contains("1200"));
        let restored = deanonymize_result_input(&anonymous, &mapping).unwrap();
        assert!(restored.contains("987654321"));
    }

    #[test]
    fn psql_gx_numeric_business_code_is_hidden_but_rows_stay_exact() {
        let source = "-[ RECORD 1 ]----------\ncustomer_code | 987654321\ncalls         | 350\nrows          | 1200\n";
        let mut mapping = PrivacyMapping::default();
        let (anonymous, _) = anonymize_result_input(source, &mut mapping).unwrap();
        assert!(!anonymous.contains("987654321"));
        assert!(anonymous.contains("350"));
        assert!(anonymous.contains("1200"));
        let restored = deanonymize_result_input(&anonymous, &mapping).unwrap();
        assert!(restored.contains("987654321"));
    }

    #[test]
    fn unparseable_query_is_redacted_instead_of_leaking_source() {
        let source = json!([
            {
                "query": "<insufficient privilege: secret tenant query>"
            }
        ]);

        let mut mapping = PrivacyMapping::default();
        let (anonymous, warnings) = anonymize_result(&source, &mut mapping).unwrap();

        let query = anonymous[0]["query"].as_str().unwrap();
        assert_eq!(query, "redacted_001");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("query"));
        assert!(!query.contains("secret"));

        let restored = deanonymize_result(&anonymous, &mapping).unwrap();
        assert_eq!(restored, source);
    }

    #[test]
    fn psql_pg_stat_statements_table_anonymizes_queryid_and_preserves_metrics() {
        let source = r#"       queryid        | calls | total_ms  | mean_ms  |   rows
----------------------+-------+-----------+----------+----------
 -1234567890123456789 |   120 | 4800060.5 |  40000.5 |  1234560
 -2345678901234567890 |     1 |  240000.5 | 240000.5 | 50000000
 -2345678901234567890 |     1 |  250000.5 | 250000.5 | 51000000
(3 rows)
"#;

        let mut mapping = PrivacyMapping::default();
        let (anonymous, warnings) = anonymize_result_input(source, &mut mapping).unwrap();

        assert!(!anonymous.contains("-1234567890123456789"));
        assert!(!anonymous.contains("-2345678901234567890"));
        assert!(anonymous.contains("900000001"));
        assert_eq!(anonymous.matches("900000002").count(), 2);
        assert!(anonymous.contains("4800060.5"));
        assert!(anonymous.contains("40000.5"));
        assert!(anonymous.contains("1234560"));
        assert!(anonymous.contains("(3 rows)"));
        assert!(warnings.is_empty());

        let restored = deanonymize_result_input(&anonymous, &mapping).unwrap();

        assert!(restored.contains("-1234567890123456789"));
        assert_eq!(restored.matches("-2345678901234567890").count(), 2);
        assert!(restored.contains("4800060.5"));
        assert!(restored.contains("51000000"));
    }

    #[test]
    fn psql_expanded_pg_stat_statements_anonymizes_queryid_and_preserves_metrics() {
        let source = r#"-[ RECORD 1 ]----------------------
queryid  | -1234567890123456789
calls    | 120
total_ms | 4800060.5
mean_ms  | 40000.5
rows     | 1234560
-[ RECORD 2 ]----------------------
queryid  | -2345678901234567890
calls    | 1
total_ms | 240000.5
mean_ms  | 240000.5
rows     | 50000000
-[ RECORD 3 ]----------------------
queryid  | -2345678901234567890
calls    | 1
total_ms | 250000.5
mean_ms  | 250000.5
rows     | 51000000
"#;

        let mut mapping = PrivacyMapping::default();
        let (anonymous, warnings) = anonymize_result_input(source, &mut mapping).unwrap();

        assert!(anonymous.starts_with("-[ RECORD 1 ]-"));
        assert!(!anonymous.contains("-1234567890123456789"));
        assert!(!anonymous.contains("-2345678901234567890"));
        assert!(anonymous.contains("900000001"));
        assert_eq!(anonymous.matches("900000002").count(), 2);
        assert!(anonymous.contains("4800060.5"));
        assert!(anonymous.contains("40000.5"));
        assert!(anonymous.contains("1234560"));
        assert!(warnings.is_empty());

        let restored = deanonymize_result_input(&anonymous, &mapping).unwrap();

        assert!(restored.contains("-1234567890123456789"));
        assert_eq!(restored.matches("-2345678901234567890").count(), 2);
        assert!(restored.contains("51000000"));
    }

    #[test]
    fn psql_expanded_accepts_record_marker_with_column_separator() {
        let source = r#"-[ RECORD 1 ]-------+-------------------------------------------------------------------------------------
relation_oid | 20001
table_name   | orders
n_live_tup   | 55000000
-[ RECORD 2 ]-------+-------------------------------------------------------------------------------------
relation_oid | 20002
table_name   | payments
n_live_tup   | 22000000
"#;

        let mut mapping = PrivacyMapping::default();
        let (anonymous, warnings) = anonymize_result_input(source, &mut mapping).unwrap();

        assert!(anonymous.starts_with("-[ RECORD 1 ]-"));
        assert!(anonymous.contains("-[ RECORD 2 ]-"));
        assert!(!anonymous.contains("20001"));
        assert!(!anonymous.contains("20002"));
        assert!(!anonymous.contains("orders"));
        assert!(!anonymous.contains("payments"));
        assert!(anonymous.contains("55000000"));
        assert!(anonymous.contains("22000000"));
        assert!(warnings.is_empty());
    }

    #[test]
    fn psql_expanded_supports_multiline_query_cells() {
        let source = r#"-[ RECORD 1 ]----------------------
pid     | 12345
datname | demo_db
state   | active
query   | SELECT *+
        | FROM private.users+
        | WHERE email = 'secret@example.com'
"#;

        let mut mapping = PrivacyMapping::default();
        let (anonymous, warnings) = anonymize_result_input(source, &mut mapping).unwrap();

        assert!(!anonymous.contains("demo_db"));
        assert!(!anonymous.contains("private.users"));
        assert!(!anonymous.contains("secret@example.com"));
        assert!(anonymous.contains("database_001"));
        assert!(anonymous.contains("schema_001.table_001"));
        assert!(warnings.is_empty());
    }

    #[test]
    fn psql_expanded_accepts_zero_rows_with_timing() {
        let source = "(0 rows)\nTime: 1.234 ms\n";
        let mut mapping = PrivacyMapping::default();

        let (anonymous, warnings) = anonymize_result_input(source, &mut mapping).unwrap();

        assert_eq!(anonymous, source);
        assert!(warnings.is_empty());
    }

    #[test]
    fn psql_expanded_preserves_timing_and_null_marker() {
        let source = r#"-[ RECORD 1 ]----------------------
queryid | -1234567890123456789
state   | ∅
(1 row)
Time: 1.234 ms
"#;

        let mut mapping = PrivacyMapping::default();
        let (anonymous, warnings) = anonymize_result_input(source, &mut mapping).unwrap();

        assert!(!anonymous.contains("-1234567890123456789"));
        assert!(anonymous.contains("900000001"));
        assert!(anonymous.contains("state   | ∅"));
        assert!(anonymous.ends_with("(1 row)\nTime: 1.234 ms\n"));
        assert!(warnings.is_empty());
    }

    #[test]
    fn psql_aligned_accepts_zero_rows_with_timing() {
        let source = "(0 rows)\nTime: 0.321 ms\n";
        let mut mapping = PrivacyMapping::default();

        let (anonymous, warnings) = anonymize_result_input(source, &mut mapping).unwrap();

        assert_eq!(anonymous, source);
        assert!(warnings.is_empty());
    }

    #[test]
    fn result_input_still_accepts_json() {
        let source = r#"[{"queryid":123,"calls":7}]"#;
        let mut mapping = PrivacyMapping::default();

        let (anonymous, warnings) = anonymize_result_input(source, &mut mapping).unwrap();
        let value: Value = serde_json::from_str(&anonymous).unwrap();

        assert_ne!(value[0]["queryid"], 123);
        assert_eq!(value[0]["calls"], 7);
        assert!(warnings.is_empty());
    }

    #[test]
    fn new_fields_use_generic_fallback_without_diagnostic_configuration() {
        let source = json!([
            {
                "table": "orders",
                "size_bytes": 42,
                "new_server_field": "tenant-private"
            }
        ]);

        let mut mapping = PrivacyMapping::default();
        let (anonymous, warnings) = anonymize_result(&source, &mut mapping).unwrap();

        assert_eq!(anonymous[0]["table"], "table_001");
        assert_eq!(anonymous[0]["size_bytes"], 42);
        assert_ne!(anonymous[0]["new_server_field"], "tenant-private");
        assert!(warnings[0].contains("new_server_field"));
    }

    #[test]
    fn custom_policy_preserves_default_and_anonymizes_selected_fields() {
        let source = json!([{
            "relation_oid": 20001,
            "schema_name": "public",
            "table_name": "orders",
            "heap_size": "12 GB",
            "primary_key": ["order_id", "item_id"]
        }]);
        let config = ResultPrivacyConfig {
            default: ResultPrivacyDefault::Preserve,
            keys: None,
            fields: BTreeMap::from([
                ("table_name".into(), ResultPrivacyFieldAction::Table),
                ("primary_key".into(), ResultPrivacyFieldAction::Columns),
            ]),
            patterns: Vec::new(),
        };
        let mut mapping = PrivacyMapping::default();

        let (anonymous, warnings) =
            anonymize_result_with_config(&source, &mut mapping, &config).unwrap();

        assert_eq!(anonymous[0]["relation_oid"], 20001);
        assert_eq!(anonymous[0]["schema_name"], "public");
        assert_eq!(anonymous[0]["heap_size"], "12 GB");
        assert_eq!(anonymous[0]["table_name"], "table_001");
        assert_eq!(
            anonymous[0]["primary_key"],
            json!(["column_001", "column_002"])
        );
        assert!(warnings.is_empty());
        assert_eq!(
            deanonymize_result_with_config(&anonymous, &mapping, &config).unwrap(),
            source
        );
    }

    #[test]
    fn custom_policy_handles_psql_expanded_column_list() {
        let source = r#"-[ RECORD 1 ]-------+----------------
table_name  | orders
heap_size   | 12 GB
primary_key | order_id, item_id
(1 row)
"#;
        let config = ResultPrivacyConfig {
            default: ResultPrivacyDefault::Preserve,
            keys: None,
            fields: BTreeMap::from([
                ("table_name".into(), ResultPrivacyFieldAction::Table),
                ("primary_key".into(), ResultPrivacyFieldAction::Columns),
            ]),
            patterns: Vec::new(),
        };
        let mut mapping = PrivacyMapping::default();

        let (anonymous, warnings) =
            anonymize_result_input_with_config(source, &mut mapping, &config).unwrap();

        assert!(anonymous.contains("table_001"));
        assert!(anonymous.contains("12 GB"));
        assert!(anonymous.contains("column_001, column_002"));
        assert!(warnings.is_empty());

        let restored = deanonymize_result_input_with_config(&anonymous, &mapping, &config).unwrap();
        assert!(restored.contains("orders"));
        assert!(restored.contains("order_id, item_id"));
    }

    #[test]
    fn custom_policy_preserves_primary_key_sentinel() {
        let source = json!({"primary_key": "NO PRIMARY KEY"});
        let config = ResultPrivacyConfig {
            default: ResultPrivacyDefault::Preserve,
            keys: None,
            fields: BTreeMap::from([("primary_key".into(), ResultPrivacyFieldAction::Columns)]),
            patterns: Vec::new(),
        };
        let mut mapping = PrivacyMapping::default();

        let (anonymous, _) = anonymize_result_with_config(&source, &mut mapping, &config).unwrap();

        assert_eq!(anonymous["primary_key"], "NO PRIMARY KEY");
    }

    #[test]
    fn nested_policy_anonymizes_dynamic_table_keys_and_nested_columns() {
        let source = json!({
            "tables": {
                "demo_items": {
                    "row_identity": {
                        "type": null,
                        "columns": ["group_id"],
                        "database_enforced_unique": null
                    }
                }
            },
            "metrics": {
                "demo_items": {
                    "count": 42
                }
            }
        });
        let config: ResultPrivacyConfig = serde_json::from_value(json!({
            "default": "preserve",
            "fields": {
                "tables": {
                    "keys": "table",
                    "values": {
                        "fields": {
                            "row_identity": {
                                "fields": {
                                    "columns": "columns"
                                }
                            }
                        }
                    }
                }
            }
        }))
        .unwrap();
        let mut mapping = PrivacyMapping::default();

        let (anonymous, warnings) =
            anonymize_result_with_config(&source, &mut mapping, &config).unwrap();

        assert!(anonymous["tables"].get("demo_items").is_none());
        assert_eq!(
            anonymous["tables"]["table_001"]["row_identity"]["columns"],
            json!(["column_001"])
        );
        assert_eq!(
            anonymous["tables"]["table_001"]["row_identity"]["type"],
            Value::Null
        );
        assert_eq!(anonymous["metrics"]["demo_items"]["count"], 42);
        assert!(warnings.is_empty());

        let restored = deanonymize_result_with_config(&anonymous, &mapping, &config).unwrap();
        assert_eq!(restored, source);
    }

    #[test]
    fn nested_policy_rejects_array_action_for_object_keys() {
        let config: ResultPrivacyConfig = serde_json::from_value(json!({
            "default": "preserve",
            "fields": {
                "tables": {
                    "keys": "columns"
                }
            }
        }))
        .unwrap();

        let error = config.normalized().unwrap_err();
        assert!(error.contains("object keys"));
    }

    #[test]
    fn simple_field_action_json_remains_backward_compatible() {
        let config: ResultPrivacyConfig = serde_json::from_value(json!({
            "default": "preserve",
            "fields": {
                "table_name": "table",
                "primary_key": "columns"
            }
        }))
        .unwrap();

        assert_eq!(config.fields["table_name"], ResultPrivacyFieldAction::Table);
        assert_eq!(
            config.fields["primary_key"],
            ResultPrivacyFieldAction::Columns
        );
    }

    #[test]
    fn structured_json_string_field_anonymizes_only_configured_nested_fields() {
        let row_identity =
            r#"{"type":"primary_key","columns":["order_id"],"database_enforced_unique":true}"#;

        let first_width = 12;
        let second_width = row_identity.chars().count() + 1;

        let source = format!(
            " table_name | row_identity\n{}+{}\n orders     | {}\n(1 row)\n",
            "-".repeat(first_width),
            "-".repeat(second_width),
            row_identity,
        );

        let config: ResultPrivacyConfig = serde_json::from_value(json!({
            "default": "preserve",
            "fields": {
                "table_name": "table",
                "row_identity": {
                    "json": true,
                    "default": "preserve",
                    "fields": {
                        "columns": "columns"
                    }
                }
            }
        }))
        .unwrap();

        let mut mapping = PrivacyMapping::default();

        let (anonymous, warnings) =
            anonymize_result_input_with_config(&source, &mut mapping, &config).unwrap();

        assert!(anonymous.contains("table_001"));
        assert!(anonymous.contains("\"type\":\"primary_key\""));
        assert!(anonymous.contains("column_001"));
        assert!(warnings.is_empty());

        let restored = deanonymize_result_input_with_config(&anonymous, &mapping, &config).unwrap();

        assert!(restored.contains("orders"));
        assert!(restored.contains("order_id"));
    }

    #[test]
    fn regex_field_rule_can_target_multiple_result_fields() {
        let source = json!({
            "source_table": "orders",
            "target_table": "archive_orders",
            "label": "keep-me"
        });
        let config: ResultPrivacyConfig = serde_json::from_value(json!({
            "default": "preserve",
            "patterns": [
                {"pattern": "^(source|target)_table$", "action": "table"}
            ]
        }))
        .unwrap();
        let mut mapping = PrivacyMapping::default();

        let (anonymous, warnings) =
            anonymize_result_with_config(&source, &mut mapping, &config).unwrap();

        assert_eq!(anonymous["source_table"], "table_001");
        assert_eq!(anonymous["target_table"], "table_002");
        assert_eq!(anonymous["label"], "keep-me");
        assert!(warnings.is_empty());
    }
}
