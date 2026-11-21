use pg_query::NodeRef;
use serde::Deserialize;

use super::model::AutovacuumUpdateStatement;

const MAX_PGSS_MATCHES: usize = 20;

/// Raw text is ephemeral: never expose it in serialized detail, log or SQLite.
#[derive(Debug, Deserialize)]
pub(super) struct PgssUpdateCandidate {
    pub queryid: Option<String>,
    pub userid: Option<i64>,
    pub dbid: Option<i64>,
    pub toplevel: Option<bool>,
    pub calls: i64,
    pub total_exec_time_ms: f64,
    pub mean_exec_time_ms: f64,
    pub rows: i64,
    pub sql_text: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MatchLevel {
    ParsedQualified,
    HeuristicUnqualified,
}

fn update_target_level(sql: &str, schema: &str, table: &str) -> Option<MatchLevel> {
    // Parser handles WITH ... UPDATE and quoted identifiers. We never execute SQL.
    let parsed = pg_query::parse(sql).ok()?;
    let mut found = None;
    for (node, _, _, _) in parsed.protobuf.nodes() {
        let NodeRef::UpdateStmt(update) = node else {
            continue;
        };
        let Some(relation) = update.relation.as_ref() else {
            continue;
        };
        if relation.relname != table {
            continue;
        }
        if relation.schemaname == schema {
            return Some(MatchLevel::ParsedQualified);
        }
        if relation.schemaname.is_empty() {
            found = Some(MatchLevel::HeuristicUnqualified);
        }
    }
    found
}

pub(super) fn identify_update_targets(
    candidates: Vec<PgssUpdateCandidate>,
    relation_oid: u32,
    schema: &str,
    table: &str,
) -> Vec<AutovacuumUpdateStatement> {
    candidates.into_iter().filter_map(|candidate| {
        let level = update_target_level(candidate.sql_text.as_deref()?, schema, table)?;
        let (evidence_level, evidence_note) = match level {
            MatchLevel::ParsedQualified => (
                "parsed_qualified", crate::i18n::text("pg_query разобрал UPDATE с явным schema.table; это не доказательство runtime-зависимости от relation OID", "pg_query parsed UPDATE with an explicit schema.table; this does not prove a runtime dependency on the relation OID"),
            ),
            MatchLevel::HeuristicUnqualified => (
                "heuristic_unqualified", crate::i18n::text("pg_query нашёл UPDATE table без схемы: без search_path невозможно однозначно определить relation OID", "pg_query found UPDATE on an unqualified table; relation OID cannot be resolved unambiguously without search_path"),
            ),
        };
        Some(AutovacuumUpdateStatement {
            queryid: candidate.queryid, userid: candidate.userid, dbid: candidate.dbid,
            toplevel: candidate.toplevel, relation_oid, evidence_level: evidence_level.to_owned(),
            evidence_note: evidence_note.to_owned(),
            calls: candidate.calls, total_exec_time_ms: candidate.total_exec_time_ms,
            mean_exec_time_ms: candidate.mean_exec_time_ms, rows: candidate.rows,
            query: crate::i18n::text("UPDATE (SQL скрыт; см. queryid)", "UPDATE (SQL hidden; see queryid)").to_owned(),
        })
    }).take(MAX_PGSS_MATCHES).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsed_sql_distinguishes_qualified_and_unqualified_update_targets() {
        assert_eq!(
            update_target_level(
                "UPDATE demo.sample_items SET enabled = false",
                "demo",
                "sample_items"
            ),
            Some(MatchLevel::ParsedQualified)
        );
        assert_eq!(
            update_target_level(
                "WITH updated AS (UPDATE demo.sample_items SET enabled=false RETURNING id) SELECT count(*) FROM updated",
                "demo",
                "sample_items"
            ),
            Some(MatchLevel::ParsedQualified)
        );
        assert_eq!(
            update_target_level(
                "UPDATE sample_items SET enabled = false",
                "demo",
                "sample_items"
            ),
            Some(MatchLevel::HeuristicUnqualified)
        );
        assert_eq!(
            update_target_level(
                "UPDATE other.sample_items SET enabled = false",
                "demo",
                "sample_items"
            ),
            None
        );
        assert_eq!(
            update_target_level("SELECT * FROM demo.sample_items", "demo", "sample_items"),
            None
        );
        assert_eq!(
            update_target_level(
                "UPDATE demo.other_items SET enabled = false FROM demo.sample_items",
                "demo",
                "sample_items"
            ),
            None
        );
        assert_eq!(
            update_target_level(
                r#"UPDATE "demo"."MixedCaseItems" SET enabled = false"#,
                "demo",
                "MixedCaseItems"
            ),
            Some(MatchLevel::ParsedQualified)
        );
    }

    #[test]
    fn raw_sql_never_leaves_redacted_evidence() {
        let candidate = PgssUpdateCandidate {
            queryid: Some("123".into()),
            userid: Some(2),
            dbid: Some(3),
            toplevel: Some(true),
            calls: 5,
            total_exec_time_ms: 50.0,
            mean_exec_time_ms: 10.0,
            rows: 3,
            sql_text: Some("UPDATE demo.sample_items SET note='do-not-expose'".into()),
        };
        let evidence = identify_update_targets(vec![candidate], 4242, "demo", "sample_items");
        assert_eq!(evidence.len(), 1);
        let json = serde_json::to_string(&evidence).unwrap();
        assert!(!json.contains("do-not-expose"));
        assert_eq!(evidence[0].evidence_level, "parsed_qualified");
        assert_eq!(evidence[0].relation_oid, 4242);
    }
}
