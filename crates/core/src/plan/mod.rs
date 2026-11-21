mod analyze;
mod compare;
mod model;
mod prompt;
mod query;
mod reference;

pub use prompt::explain_analysis_prompt;

use crate::{internal_postgres::InternalPostgresRegistry, postgres::ConnectionInput};
use serde::Serialize;

pub use analyze::{
    analyze, FindingSeverity, PlanAnalysis, PlanAnalysisSummary, PlanFinding, PlanHotspot,
};
pub use compare::{
    compare, FindingDelta, MetricDelta, NodeTypeDelta, PlanComparison, RelationStrategyChange,
};
pub use model::{all_nodes, parse_explain_json, PlanDocument, PlanNode};
pub use query::{
    compare_results, execute_call_action, explain_query, validate_call_action,
    validate_readonly_select, CallActionExecution, CallActionInfo, QueryResultComparison,
    ReadonlyQueryInfo,
};
pub use reference::{
    all as plan_reference, for_node_type as plan_reference_for_node_type, PlanReferenceEntry,
};

#[derive(Clone, Debug, Serialize)]
pub struct PlanInspection {
    pub document: PlanDocument,
    pub analysis: PlanAnalysis,
}

#[derive(Clone, Debug, Serialize)]
pub struct QueryPlanComparison {
    pub analyze: bool,
    pub baseline_plan_json: String,
    pub candidate_plan_json: String,
    pub baseline: PlanInspection,
    pub candidate: PlanInspection,
    pub comparison: PlanComparison,
    pub warnings: Vec<String>,
}

pub fn inspect_json(input: &str) -> Result<PlanInspection, String> {
    let document = parse_explain_json(input)?;
    let analysis = analyze(&document);

    Ok(PlanInspection { document, analysis })
}

pub fn compare_json(baseline_json: &str, candidate_json: &str) -> Result<PlanComparison, String> {
    let baseline = parse_explain_json(baseline_json)?;
    let candidate = parse_explain_json(candidate_json)?;

    Ok(compare(&baseline, &candidate))
}

pub async fn compare_query_plans(
    internal_postgres: &InternalPostgresRegistry,
    connection_id: i64,
    input: &ConnectionInput,
    baseline_sql: &str,
    candidate_sql: &str,
    analyze_queries: bool,
    timeout_ms: u64,
) -> Result<QueryPlanComparison, String> {
    validate_readonly_select(baseline_sql)?;
    validate_readonly_select(candidate_sql)?;

    let baseline_plan_json = explain_query(
        internal_postgres,
        connection_id,
        input,
        baseline_sql,
        analyze_queries,
        timeout_ms,
    )
    .await?;

    let candidate_plan_json = explain_query(
        internal_postgres,
        connection_id,
        input,
        candidate_sql,
        analyze_queries,
        timeout_ms,
    )
    .await?;

    let baseline = inspect_json(&baseline_plan_json)?;
    let candidate = inspect_json(&candidate_plan_json)?;
    let comparison = compare(&baseline.document, &candidate.document);

    let warnings = if analyze_queries {
        vec![
            crate::i18n::text("EXPLAIN ANALYZE реально выполнил оба SELECT. Сравнивайте результаты на одинаковом workload/cache context и повторяйте несколько runs.", "EXPLAIN ANALYZE actually executed both SELECT statements. Compare under similar workload/cache conditions and repeat several runs.").into(),
            crate::i18n::text("Actual node times inclusive и усредняются по loops; pgXray не суммирует их как wall-clock time.", "Actual node times are inclusive and averaged over loops; pgXray does not add them up as wall-clock time.").into(),
        ]
    } else {
        vec![
            crate::i18n::text("Планы получены через EXPLAIN без ANALYZE: statement не выполнялся, поэтому Actual Rows/Time/Buffers отсутствуют.", "Plans were obtained using EXPLAIN without ANALYZE: the statement was not executed, so Actual Rows/Time/Buffers are unavailable.").into(),
        ]
    };

    Ok(QueryPlanComparison {
        analyze: analyze_queries,
        baseline_plan_json,
        candidate_plan_json,
        baseline,
        candidate,
        comparison,
        warnings,
    })
}
