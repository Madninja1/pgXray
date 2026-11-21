//! Per-user scheduler rules and atomic cross-process slot claims.
use super::DiagnosticStore;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::Value;

pub type ScheduleRule = Value;

fn validate_rule(mut rule: Value) -> Result<(String, Value), String> {
    let obj = rule.as_object_mut().ok_or(crate::i18n::text(
        "Некорректное расписание",
        "Invalid schedule",
    ))?;
    obj.remove("dbRevision");
    const ALLOWED: &[&str] = &[
        "key",
        "fingerprint",
        "targetName",
        "connectionId",
        "kind",
        "mode",
        "intervalMinutes",
        "windowStart",
        "windowEnd",
        "intervalAnchor",
        "daysOfWeek",
        "dailyTime",
        "seriesCount",
        "seriesSpanMinutes",
        "enabled",
        "status",
        "createdAt",
        "nextAt",
        "seriesStartedAt",
        "seriesCompleted",
        "lastAttemptAt",
        "lastSuccessAt",
        "lastDurationMs",
        "lastError",
    ];
    if obj.len() != ALLOWED.len() || obj.keys().any(|key| !ALLOWED.contains(&key.as_str())) {
        return Err(crate::i18n::text(
            "Недопустимые поля расписания (секреты не сохраняются)",
            "Unsupported schedule fields (secrets must not be stored)",
        )
        .into());
    }
    let key = obj
        .get("key")
        .and_then(Value::as_str)
        .ok_or(crate::i18n::text(
            "Нет ключа расписания",
            "Missing schedule key",
        ))?
        .to_owned();
    let connection_id =
        obj.get("connectionId")
            .and_then(Value::as_i64)
            .ok_or(crate::i18n::text(
                "Нет connectionId",
                "Missing connectionId",
            ))?;
    let kind = obj
        .get("kind")
        .and_then(Value::as_str)
        .ok_or(crate::i18n::text(
            "Нет вида снимка",
            "Missing snapshot kind",
        ))?;
    let mode = obj
        .get("mode")
        .and_then(Value::as_str)
        .ok_or(crate::i18n::text("Нет режима", "Missing schedule mode"))?;
    if connection_id <= 0
        || ![
            "xid",
            "pgss",
            "autovacuum",
            "pg_settings",
            "io_wal",
            "index_health",
        ]
        .contains(&kind)
        || !["daily", "interval", "series"].contains(&mode)
        || !key.starts_with(&format!("{connection_id}:{kind}:"))
        || key.len() > 128
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b':' || b == b'_')
        || !obj.get("nextAt").is_some_and(Value::is_u64)
        || !obj.get("enabled").is_some_and(Value::is_boolean)
        || !obj
            .get("status")
            .and_then(Value::as_str)
            .is_some_and(|v| ["active", "paused", "complete", "expired"].contains(&v))
        || !obj.get("daysOfWeek").is_some_and(Value::is_array)
        || obj
            .get("fingerprint")
            .and_then(Value::as_str)
            .is_none_or(|s| s.len() > 2048)
        || obj
            .get("lastError")
            .and_then(Value::as_str)
            .is_some_and(|s| s.len() > 1000)
        || serde_json::to_vec(&rule).map_err(|e| e.to_string())?.len() > 4096
    {
        return Err(crate::i18n::text(
            "Некорректные значения расписания",
            "Invalid schedule values",
        )
        .into());
    }
    Ok((key, rule))
}

fn add_revision(mut rule: Value, revision: i64) -> Value {
    rule.as_object_mut()
        .expect("validated rule")
        .insert("dbRevision".into(), revision.into());
    rule
}

impl DiagnosticStore {
    pub fn list_schedule_rules(&self, owner: &str) -> Result<Vec<ScheduleRule>, String> {
        let mut stmt = self.connection.prepare(
            "SELECT rule_json, revision FROM snapshot_schedules WHERE owner = ?1 ORDER BY rule_key"
        ).map_err(|e| e.to_string())?;
        let mapped = stmt
            .query_map([owner], |row| {
                let raw: String = row.get(0)?;
                let revision: i64 = row.get(1)?;
                let rule: Value = serde_json::from_str(&raw).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
                Ok(add_revision(rule, revision))
            })
            .map_err(|e| e.to_string())?;
        // The iterator borrows `stmt`: collect it before leaving this scope.
        let rules = mapped
            .collect::<rusqlite::Result<Vec<ScheduleRule>>>()
            .map_err(|e| e.to_string())?;
        Ok(rules)
    }

    pub fn put_schedule_rule(
        &mut self,
        owner: &str,
        rule: Value,
        expected: Option<i64>,
    ) -> Result<Value, String> {
        let (key, rule) = validate_rule(rule)?;
        let json = serde_json::to_string(&rule).map_err(|e| e.to_string())?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let revision = if let Some(expected) = expected {
            let affected = tx.execute(
                "UPDATE snapshot_schedules SET rule_json = ?1, revision = revision + 1 WHERE owner = ?2 AND rule_key = ?3 AND revision = ?4",
                params![json, owner, key, expected]
            ).map_err(|e| e.to_string())?;
            if affected != 1 {
                return Err(crate::i18n::text(
                    "Расписание изменено в другом экземпляре pgXray. Обнови список.",
                    "Schedule was modified in another pgXray instance. Refresh the list.",
                )
                .into());
            }
            expected + 1
        } else {
            let count: i64 = tx
                .query_row(
                    "SELECT count(*) FROM snapshot_schedules WHERE owner = ?1",
                    [owner],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if count >= 60 {
                return Err(
                    crate::i18n::text("Максимум 60 расписаний", "Maximum of 60 schedules").into(),
                );
            }
            tx.execute("INSERT INTO snapshot_schedules(owner, rule_key, rule_json, revision) VALUES (?1, ?2, ?3, 1)",
                params![owner, key, json]).map_err(|e| format!("Не удалось добавить расписание (возможно, такой ключ уже создан): {e}"))?;
            1
        };
        tx.commit().map_err(|e| e.to_string())?;
        Ok(add_revision(rule, revision))
    }

    pub fn remove_schedule_rule(
        &mut self,
        owner: &str,
        key: &str,
        expected: i64,
    ) -> Result<(), String> {
        let deleted = self.connection.execute(
            "DELETE FROM snapshot_schedules WHERE owner = ?1 AND rule_key = ?2 AND revision = ?3",
            params![owner, key, expected],
        ).map_err(|e| e.to_string())?;
        if deleted != 1 {
            return Err("Расписание изменено в другом экземпляре. Обнови список.".into());
        }
        Ok(())
    }

    pub fn claim_schedule_slot(
        &mut self,
        owner: &str,
        rule: Value,
        expected: i64,
        slot_at_ms: i64,
        claimed_at_ms: i64,
    ) -> Result<Option<Value>, String> {
        let (key, rule) = validate_rule(rule)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let current: Option<(String, i64)> = tx.query_row(
            "SELECT rule_json, revision FROM snapshot_schedules WHERE owner = ?1 AND rule_key = ?2",
            params![owner, key], |row| Ok((row.get(0)?, row.get(1)?))
        ).optional().map_err(|e| e.to_string())?;
        let Some((raw, revision)) = current else {
            return Ok(None);
        };
        let previous: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        if revision != expected
            || !previous["enabled"].as_bool().unwrap_or(false)
            || previous["status"].as_str() != Some("active")
            || previous["nextAt"].as_i64() != Some(slot_at_ms)
        {
            return Ok(None);
        }
        let connection_id = previous["connectionId"]
            .as_i64()
            .ok_or("Некорректный connectionId")?;
        let kind = previous["kind"].as_str().ok_or("Некорректный kind")?;
        // Never execute a slot twice, even with two Tauri processes or different local accounts.
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO snapshot_schedule_claims(connection_id, kind, slot_at_ms, claimed_at_ms) VALUES (?1, ?2, ?3, ?4)",
            params![connection_id, kind, slot_at_ms, claimed_at_ms]
        ).map_err(|e| e.to_string())?;
        if inserted == 0 {
            return Ok(None);
        }
        tx.execute(
            "UPDATE snapshot_schedules SET rule_json = ?1, revision = revision + 1 WHERE owner = ?2 AND rule_key = ?3 AND revision = ?4",
            params![serde_json::to_string(&rule).map_err(|e| e.to_string())?, owner, key, expected]
        ).map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM snapshot_schedule_claims WHERE slot_at_ms < ?1",
            [claimed_at_ms.saturating_sub(30 * 86_400_000)],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(Some(add_revision(rule, expected + 1)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use serde_json::json;

    fn test_store() -> DiagnosticStore {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(r#"
            CREATE TABLE snapshot_schedules(owner TEXT NOT NULL,rule_key TEXT NOT NULL,rule_json TEXT NOT NULL,revision INTEGER NOT NULL,PRIMARY KEY(owner,rule_key));
            CREATE TABLE snapshot_schedule_claims(connection_id INTEGER NOT NULL,kind TEXT NOT NULL,slot_at_ms INTEGER NOT NULL,claimed_at_ms INTEGER NOT NULL,PRIMARY KEY(connection_id,kind,slot_at_ms));
        "#).unwrap();
        DiagnosticStore { connection }
    }
    fn rule(key: &str, slot: i64) -> Value {
        json!({
            "key":key,"connectionId":1,"kind":"xid","mode":"daily",
            "fingerprint":"test","targetName":"test","intervalMinutes":60,
            "windowStart":"00:00","windowEnd":"00:00","intervalAnchor":"09:00",
            "daysOfWeek":[0,1,2,3,4,5,6],"dailyTime":"09:00",
            "seriesCount":3,"seriesSpanMinutes":60,"enabled":true,"status":"active",
            "createdAt":1,"nextAt":slot,"seriesStartedAt":null,"seriesCompleted":0,
            "lastAttemptAt":null,"lastSuccessAt":null,"lastDurationMs":null,"lastError":null
        })
    }
    #[test]
    fn versioned_updates_reject_stale_process() {
        let mut store = test_store();
        let first = store
            .put_schedule_rule("dba", rule("1:xid:100:0", 100), None)
            .unwrap();
        assert_eq!(first["dbRevision"], 1);
        assert!(store
            .put_schedule_rule("dba", rule("1:xid:100:0", 200), Some(1))
            .is_ok());
        assert!(store
            .put_schedule_rule("dba", rule("1:xid:100:0", 300), Some(1))
            .is_err());
        assert_eq!(store.list_schedule_rules("dba").unwrap()[0]["nextAt"], 200);
        assert!(store.list_schedule_rules("other").unwrap().is_empty());
    }
    #[test]
    fn claims_are_atomic_and_deduplicate_different_schedules() {
        let mut store = test_store();
        let first = rule("1:xid:100:0", 100);
        let second = rule("1:xid:100:1", 100);
        store.put_schedule_rule("dba", first.clone(), None).unwrap();
        store
            .put_schedule_rule("dba", second.clone(), None)
            .unwrap();
        let mut reserved = first.clone();
        reserved["nextAt"] = 200.into();
        assert!(store
            .claim_schedule_slot("dba", reserved, 1, 100, 101)
            .unwrap()
            .is_some());
        assert!(store
            .claim_schedule_slot("dba", second, 1, 100, 101)
            .unwrap()
            .is_none());
        assert_eq!(store.list_schedule_rules("dba").unwrap().len(), 2);
    }
    #[test]
    fn rule_rejects_credentials_and_unexpected_fields() {
        let mut rule = rule("1:xid:100:0", 100);
        rule["password"] = "secret".into();
        assert!(validate_rule(rule).is_err());
    }
}
