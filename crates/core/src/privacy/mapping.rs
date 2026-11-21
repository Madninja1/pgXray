use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    thread,
    time::{Duration, SystemTime},
};

const MAPPING_VERSION: i64 = 3;
const LOCK_WAIT_STEPS: usize = 100;
const LOCK_WAIT: Duration = Duration::from_millis(50);
const STALE_LOCK_AFTER: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IdentifierRecord {
    pub replacement: String,
    pub kind: String,
    #[serde(default)]
    pub kinds: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PrivacyMapping {
    pub version: i64,
    pub engine_version: String,
    pub warning: String,
    pub identifiers: BTreeMap<String, IdentifierRecord>,
    pub strings: BTreeMap<String, String>,
    pub integers: BTreeMap<String, i64>,
    pub floats: BTreeMap<String, String>,
    pub bitstrings: BTreeMap<String, String>,
    pub redactions: BTreeMap<String, String>,
    pub counters: BTreeMap<String, u64>,
}

impl Default for PrivacyMapping {
    fn default() -> Self {
        Self {
            version: MAPPING_VERSION,
            engine_version: env!("CARGO_PKG_VERSION").to_owned(),
            warning: "SENSITIVE: mapping.json содержит реальные значения. Хранить только локально."
                .into(),
            identifiers: BTreeMap::new(),
            strings: BTreeMap::new(),
            integers: BTreeMap::new(),
            floats: BTreeMap::new(),
            bitstrings: BTreeMap::new(),
            redactions: BTreeMap::new(),
            counters: BTreeMap::new(),
        }
    }
}

impl PrivacyMapping {
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let text = fs::read_to_string(path).map_err(|error| {
            format!("Не удалось прочитать mapping {}: {error}", path.display(),)
        })?;

        let raw: Value = serde_json::from_str(&text)
            .map_err(|error| format!("Некорректный mapping JSON: {error}"))?;
        let object = raw.as_object().ok_or(crate::i18n::text(
            "Некорректный mapping JSON: ожидается объект",
            "Invalid mapping JSON: expected an object",
        ))?;

        // Missing sections are permitted for legacy mappings; present sections
        // must have the correct types, otherwise silently losing aliases is unsafe.
        for section in [
            "identifiers",
            "strings",
            "integers",
            "floats",
            "bitstrings",
            "redactions",
            "counters",
        ] {
            if object.get(section).is_some_and(|value| !value.is_object()) {
                return Err(format!(
                    "Некорректный mapping: {section} должен быть объектом"
                ));
            }
        }

        let mut mapping = Self::default();

        if let Some(version) = raw.get("version").and_then(Value::as_i64) {
            mapping.version = version.max(MAPPING_VERSION);
        }

        mapping.engine_version = raw
            .get("engine_version")
            .or_else(|| raw.get("script_version"))
            .and_then(Value::as_str)
            .unwrap_or(env!("CARGO_PKG_VERSION"))
            .to_owned();

        mapping.warning = raw
            .get("warning")
            .or_else(|| raw.get("_warning"))
            .and_then(Value::as_str)
            .unwrap_or(
                "SENSITIVE: mapping.json содержит реальные значения. Хранить только локально.",
            )
            .to_owned();

        if let Some(object) = raw.get("identifiers").and_then(Value::as_object) {
            for (original, value) in object {
                let record = if let Some(replacement) = value.as_str() {
                    let kind = infer_kind(replacement);

                    IdentifierRecord {
                        replacement: replacement.to_owned(),
                        kind: kind.clone(),
                        kinds: vec![kind],
                    }
                } else {
                    serde_json::from_value::<IdentifierRecord>(value.clone()).map_err(|error| {
                        format!("Некорректная identifier mapping для {original:?}: {error}")
                    })?
                };

                mapping
                    .identifiers
                    .insert(original.clone(), normalize_record(record));
            }
        }

        load_string_map(raw.get("strings"), &mut mapping.strings)?;

        load_string_map(raw.get("floats"), &mut mapping.floats)?;

        load_string_map(raw.get("bitstrings"), &mut mapping.bitstrings)?;

        load_string_map(raw.get("redactions"), &mut mapping.redactions)?;

        if let Some(object) = raw.get("integers").and_then(Value::as_object) {
            for (original, value) in object {
                let replacement = value
                    .as_i64()
                    .ok_or_else(|| format!("Некорректная integer mapping для {original:?}"))?;

                mapping.integers.insert(original.clone(), replacement);
            }
        }

        if let Some(object) = raw.get("counters").and_then(Value::as_object) {
            for (key, value) in object {
                let value = value
                    .as_u64()
                    .ok_or_else(|| format!("Некорректный counter {key:?}"))?;

                mapping.counters.insert(key.clone(), value);
            }
        }

        mapping.rebuild_counters();

        Ok(mapping)
    }

    pub fn save(&mut self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Не удалось создать каталог mapping: {error}"))?;

            fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).map_err(|error| {
                format!("Не удалось выставить 0700 на каталог mapping: {error}")
            })?;
        }

        self.version = MAPPING_VERSION;

        self.engine_version = env!("CARGO_PKG_VERSION").to_owned();

        self.rebuild_counters();

        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| format!("Не удалось сериализовать mapping: {error}"))?;

        let tmp = path.with_extension("json.tmp");

        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|error| format!("Не удалось открыть временный mapping: {error}"))?;

        file.write_all(&bytes)
            .map_err(|error| format!("Не удалось записать временный mapping: {error}"))?;

        file.write_all(b"\n")
            .map_err(|error| format!("Не удалось завершить временный mapping: {error}"))?;

        // A pre-existing tmp file may have weaker permissions than mode(0600).
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("Не удалось выставить 0600 на временный mapping: {error}"))?;

        file.sync_all()
            .map_err(|error| format!("Не удалось синхронизировать mapping: {error}"))?;

        fs::rename(&tmp, path)
            .map_err(|error| format!("Не удалось атомарно заменить mapping: {error}"))?;

        // The rename is atomic, but the directory entry must also be durable.
        let parent = path
            .parent()
            .ok_or("Mapping path не имеет parent directory")?;
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| {
                format!("Mapping заменён, но не удалось синхронизировать каталог: {error}")
            })
    }

    pub fn identifier(&mut self, original: &str, kind: &str) -> String {
        if self
            .identifiers
            .values()
            .any(|record| record.replacement == original)
        {
            return original.to_owned();
        }

        if let Some(record) = self.identifiers.get_mut(original) {
            if !record.kinds.iter().any(|value| value == kind) {
                record.kinds.push(kind.to_owned());

                record.kinds.sort();
                record.kinds.dedup();
            }

            return record.replacement.clone();
        }

        let replacement = self.next_identifier_alias(kind);

        self.identifiers.insert(
            original.to_owned(),
            IdentifierRecord {
                replacement: replacement.clone(),
                kind: kind.to_owned(),
                kinds: vec![kind.to_owned()],
            },
        );

        replacement
    }

    pub fn string_value(&mut self, original: &str) -> String {
        if let Some(record) = self.identifiers.get(original) {
            return record.replacement.clone();
        }

        if self.strings.values().any(|value| value == original) {
            return original.to_owned();
        }

        if let Some(value) = self.strings.get(original) {
            return value.clone();
        }

        let replacement = self.next_alias("value", "value");

        self.strings
            .insert(original.to_owned(), replacement.clone());

        replacement
    }

    pub fn integer_value(&mut self, original: &str) -> i64 {
        if let Ok(value) = original.parse::<i64>() {
            if self.integers.values().any(|known| *known == value) {
                return value;
            }
        }

        if let Some(value) = self.integers.get(original) {
            return *value;
        }

        let next = self.next_counter("number");

        let replacement = 900_000_000_i64.saturating_add(i64::try_from(next).unwrap_or(0));

        self.integers.insert(original.to_owned(), replacement);

        replacement
    }

    pub fn float_value(&mut self, original: &str) -> String {
        if self.floats.values().any(|value| value == original) {
            return original.to_owned();
        }

        if let Some(value) = self.floats.get(original) {
            return value.clone();
        }

        let next = self.next_counter("float");

        let replacement = format!("900000.{next:03}");

        self.floats.insert(original.to_owned(), replacement.clone());

        replacement
    }

    pub fn redaction(&mut self, original: &str) -> String {
        if self.redactions.values().any(|value| value == original) {
            return original.to_owned();
        }

        if let Some(value) = self.redactions.get(original) {
            return value.clone();
        }

        let replacement = self.next_alias("redacted", "redacted");

        self.redactions
            .insert(original.to_owned(), replacement.clone());

        replacement
    }

    pub fn existing_alias_for_original(&self, original: &str) -> Option<String> {
        if let Some(record) = self.identifiers.get(original) {
            return Some(record.replacement.clone());
        }

        if let Some(value) = self.strings.get(original) {
            return Some(value.clone());
        }

        if let Some(value) = self.integers.get(original) {
            return Some(value.to_string());
        }

        if let Some(value) = self.floats.get(original) {
            return Some(value.clone());
        }

        if let Some(value) = self.bitstrings.get(original) {
            return Some(value.clone());
        }

        self.redactions.get(original).cloned()
    }

    pub fn original_for_alias(&self, replacement: &str) -> Option<&str> {
        self.reverse_identifier(replacement)
            .or_else(|| self.reverse_string(replacement))
            .or_else(|| self.reverse_integer(replacement))
            .or_else(|| self.reverse_float(replacement))
            .or_else(|| self.reverse_bitstring(replacement))
            .or_else(|| self.reverse_redaction(replacement))
    }

    pub fn reverse_identifier(&self, replacement: &str) -> Option<&str> {
        self.identifiers.iter().find_map(|(original, record)| {
            (record.replacement == replacement).then_some(original.as_str())
        })
    }

    pub fn reverse_string(&self, replacement: &str) -> Option<&str> {
        reverse_lookup(&self.strings, replacement)
    }

    pub fn reverse_integer(&self, replacement: &str) -> Option<&str> {
        let Ok(replacement) = replacement.parse::<i64>() else {
            return None;
        };

        self.integers
            .iter()
            .find_map(|(original, value)| (*value == replacement).then_some(original.as_str()))
    }

    pub fn reverse_float(&self, replacement: &str) -> Option<&str> {
        reverse_lookup(&self.floats, replacement)
    }

    pub fn reverse_bitstring(&self, replacement: &str) -> Option<&str> {
        reverse_lookup(&self.bitstrings, replacement)
    }

    pub fn reverse_redaction(&self, replacement: &str) -> Option<&str> {
        reverse_lookup(&self.redactions, replacement)
    }

    pub fn known_anonymous_tokens(&self) -> BTreeSet<String> {
        let mut result = BTreeSet::new();

        for record in self.identifiers.values() {
            result.insert(record.replacement.clone());
        }

        result.extend(self.strings.values().cloned());

        result.extend(self.integers.values().map(ToString::to_string));

        result.extend(self.floats.values().cloned());

        result.extend(self.bitstrings.values().cloned());

        result.extend(self.redactions.values().cloned());

        result
    }

    pub fn rebuild_counters(&mut self) {
        let identifier_data = self
            .identifiers
            .values()
            .map(|record| (record.kind.clone(), record.replacement.clone()))
            .collect::<Vec<_>>();

        for (kind, replacement) in identifier_data {
            update_counter_from_alias(&mut self.counters, &kind, &replacement);
        }

        let string_values = self.strings.values().cloned().collect::<Vec<_>>();

        for value in string_values {
            update_counter_from_alias(&mut self.counters, "value", &value);
        }

        let redaction_values = self.redactions.values().cloned().collect::<Vec<_>>();

        for value in redaction_values {
            update_counter_from_alias(&mut self.counters, "redacted", &value);
        }

        let max_number = self
            .integers
            .values()
            .filter_map(|value| value.checked_sub(900_000_000))
            .filter_map(|value| u64::try_from(value).ok())
            .max()
            .unwrap_or(0);

        self.counters
            .entry("number".into())
            .and_modify(|value| {
                *value = (*value).max(max_number);
            })
            .or_insert(max_number);

        let max_float = self
            .floats
            .values()
            .filter_map(|value| value.split_once('.'))
            .filter_map(|(_, fraction)| fraction.parse::<u64>().ok())
            .max()
            .unwrap_or(0);

        self.counters
            .entry("float".into())
            .and_modify(|value| {
                *value = (*value).max(max_float);
            })
            .or_insert(max_float);
    }

    fn next_identifier_alias(&mut self, kind: &str) -> String {
        let prefix = identifier_prefix(kind);

        self.next_alias(kind, prefix)
    }

    fn next_alias(&mut self, counter: &str, prefix: &str) -> String {
        loop {
            let next = self.next_counter(counter);

            let candidate = format!("{prefix}_{next:03}");

            if !self.alias_exists(&candidate) {
                return candidate;
            }
        }
    }

    fn next_counter(&mut self, name: &str) -> u64 {
        let value = self.counters.entry(name.to_owned()).or_insert(0);

        *value = value.saturating_add(1);

        *value
    }

    fn alias_exists(&self, candidate: &str) -> bool {
        self.identifiers
            .values()
            .any(|record| record.replacement == candidate)
            || self.strings.values().any(|value| value == candidate)
            || self.redactions.values().any(|value| value == candidate)
            || self.floats.values().any(|value| value == candidate)
            || self.bitstrings.values().any(|value| value == candidate)
            || self
                .integers
                .values()
                .any(|value| value.to_string() == candidate)
    }
}

pub struct MappingLock {
    path: PathBuf,
}

impl MappingLock {
    pub fn acquire(mapping_path: &Path) -> Result<Self, String> {
        let parent = mapping_path
            .parent()
            .ok_or("Mapping path не имеет parent directory")?;

        fs::create_dir_all(parent)
            .map_err(|error| format!("Не удалось создать каталог mapping lock: {error}"))?;

        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("Не удалось выставить 0700 на privacy scope: {error}"))?;

        let lock_path = mapping_path.with_extension("json.lock");

        for _ in 0..LOCK_WAIT_STEPS {
            match OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&lock_path)
            {
                Ok(mut file) => {
                    let _ = writeln!(file, "pid={}", std::process::id(),);

                    return Ok(Self { path: lock_path });
                }

                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if lock_is_stale(&lock_path) {
                        let _ = fs::remove_file(&lock_path);

                        continue;
                    }

                    thread::sleep(LOCK_WAIT);
                }

                Err(error) => {
                    return Err(format!("Не удалось создать mapping lock: {error}"));
                }
            }
        }

        Err(crate::i18n::text(
            "Mapping занят другим процессом. Повторите операцию.",
            "Mapping is in use by another process. Retry the operation.",
        )
        .into())
    }
}

impl Drop for MappingLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn load_string_map(
    value: Option<&Value>,
    target: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    let Some(object) = value.and_then(Value::as_object) else {
        return Ok(());
    };

    for (original, replacement) in object {
        let replacement = replacement
            .as_str()
            .ok_or_else(|| format!("Mapping для {original:?} должен быть строкой"))?;

        target.insert(original.clone(), replacement.to_owned());
    }

    Ok(())
}

fn reverse_lookup<'a>(mapping: &'a BTreeMap<String, String>, replacement: &str) -> Option<&'a str> {
    mapping
        .iter()
        .find_map(|(original, value)| (value == replacement).then_some(original.as_str()))
}

fn normalize_record(mut record: IdentifierRecord) -> IdentifierRecord {
    if record.kind.trim().is_empty() {
        record.kind = infer_kind(&record.replacement);
    }

    record.kinds.push(record.kind.clone());

    record.kinds.sort();
    record.kinds.dedup();

    record
}

fn identifier_prefix(kind: &str) -> &str {
    match kind {
        "database" => "database",
        "schema" => "schema",
        "view" => "view",
        "table" => "table",
        "index" => "index",
        "cte" => "cte",
        "alias" => "alias",
        "column" => "column",
        "column_alias" => "column_alias",
        "constraint" => "constraint",
        "function" => "function",
        "procedure" => "procedure",
        "type" => "type",
        "collation" => "collation",
        "window" => "window",
        "argument" => "argument",
        "role" => "role",
        "application" => "application",
        "address" => "address",
        "tablespace" => "tablespace",
        "prepared" => "prepared",
        "setting" => "setting",
        "object" => "object",
        _ => "ident",
    }
}

fn infer_kind(replacement: &str) -> String {
    for kind in [
        "database",
        "schema",
        "view",
        "table",
        "index",
        "cte",
        "alias",
        "column_alias",
        "column",
        "constraint",
        "function",
        "procedure",
        "type",
        "collation",
        "window",
        "argument",
        "role",
        "application",
        "address",
        "tablespace",
        "prepared",
        "setting",
        "object",
        "ident",
    ] {
        if replacement.starts_with(&format!("{kind}_")) {
            return if kind == "ident" {
                "identifier".into()
            } else {
                kind.into()
            };
        }
    }

    "identifier".into()
}

fn update_counter_from_alias(counters: &mut BTreeMap<String, u64>, name: &str, alias: &str) {
    let Some((_, suffix)) = alias.rsplit_once('_') else {
        return;
    };

    let Ok(value) = suffix.parse::<u64>() else {
        return;
    };

    counters
        .entry(name.to_owned())
        .and_modify(|current| {
            *current = (*current).max(value);
        })
        .or_insert(value);
}

fn lock_is_stale(path: &Path) -> bool {
    if let Ok(contents) = fs::read_to_string(path) {
        if let Some(pid) = contents
            .lines()
            .find_map(|line| line.strip_prefix("pid="))
            .and_then(|value| value.trim().parse::<u32>().ok())
        {
            if Path::new("/proc").join(pid.to_string()).exists() {
                return false;
            }

            return true;
        }
    }

    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };

    let Ok(modified) = metadata.modified() else {
        return false;
    };

    let Ok(age) = SystemTime::now().duration_since(modified) else {
        return false;
    };

    age > STALE_LOCK_AFTER
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_identifier_keeps_replacement() {
        let mut mapping = PrivacyMapping::default();

        let first = mapping.identifier("orders", "table");

        let second = mapping.identifier("orders", "object");

        assert_eq!(first, second,);
    }

    #[test]
    fn aliases_are_kind_specific() {
        let mut mapping = PrivacyMapping::default();

        assert_eq!(mapping.identifier("orders", "table",), "table_001",);

        assert_eq!(mapping.identifier("customer_id", "column",), "column_001",);

        assert_eq!(mapping.identifier("orders_view", "view",), "view_001",);

        assert_eq!(
            mapping.identifier("refresh_orders", "procedure",),
            "procedure_001",
        );
    }

    #[test]
    fn rejects_present_sections_with_invalid_types() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mapping.json");

        for contents in [
            "null",
            r#"{"strings": null}"#,
            r#"{"identifiers": []}"#,
            r#"{"integers": "oops"}"#,
            r#"{"counters": 4}"#,
        ] {
            fs::write(&path, contents).unwrap();
            assert!(PrivacyMapping::load(&path).is_err(), "accepted {contents}");
        }
    }

    #[test]
    fn save_and_reload_mapping_with_private_permissions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("scope").join("mapping.json");
        let mut mapping = PrivacyMapping::default();
        let alias = mapping.identifier("private_orders", "table");

        mapping.save(&path).unwrap();
        let reloaded = PrivacyMapping::load(&path).unwrap();

        assert_eq!(reloaded.reverse_identifier(&alias), Some("private_orders"));
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[test]
    fn string_reuses_existing_identifier_alias() {
        let mut mapping = PrivacyMapping::default();

        let identifier = mapping.identifier("customer_id", "column");
        let string = mapping.string_value("customer_id");

        assert_eq!(identifier, "column_001");
        assert_eq!(string, identifier);
        assert_eq!(mapping.identifiers.len(), 1);
        assert!(mapping.strings.is_empty());
    }
}
