use super::{
    analyze::{analyze, PlanAnalysis},
    model::{all_nodes, PlanDocument, PlanNode},
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize)]
pub struct MetricDelta {
    pub key: String,
    pub label: String,
    pub unit: String,
    pub baseline: Option<f64>,
    pub candidate: Option<f64>,
    pub delta: Option<f64>,
    pub delta_pct: Option<f64>,
    pub lower_is_better: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct NodeTypeDelta {
    pub node_type: String,
    pub baseline_count: usize,
    pub candidate_count: usize,
    pub delta: isize,
}

#[derive(Clone, Debug, Serialize)]
pub struct RelationStrategyChange {
    pub relation: String,
    pub baseline_strategies: Vec<String>,
    pub candidate_strategies: Vec<String>,
    pub changed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct FindingDelta {
    pub code: String,
    pub baseline_count: usize,
    pub candidate_count: usize,
    pub delta: isize,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlanComparison {
    pub metrics: Vec<MetricDelta>,
    pub node_types: Vec<NodeTypeDelta>,
    pub relation_strategies: Vec<RelationStrategyChange>,
    pub findings: Vec<FindingDelta>,
    pub baseline_analysis: PlanAnalysis,
    pub candidate_analysis: PlanAnalysis,
    pub baseline_shape: String,
    pub candidate_shape: String,
    pub structure_changed: bool,
    pub summary: Vec<String>,
}

pub fn compare(baseline: &PlanDocument, candidate: &PlanDocument) -> PlanComparison {
    let baseline_analysis = analyze(baseline);
    let candidate_analysis = analyze(candidate);
    let baseline_nodes = all_nodes(baseline);
    let candidate_nodes = all_nodes(candidate);

    let metrics = vec![
        metric(
            "execution_time",
            "Execution Time",
            "ms",
            baseline.execution_time_ms,
            candidate.execution_time_ms,
            true,
        ),
        metric(
            "planning_time",
            "Planning Time",
            "ms",
            baseline.planning_time_ms,
            candidate.planning_time_ms,
            true,
        ),
        metric(
            "shared_read",
            "Shared Read Blocks",
            "blocks",
            baseline.root.shared_read_blocks,
            candidate.root.shared_read_blocks,
            true,
        ),
        metric(
            "shared_hit",
            "Shared Hit Blocks",
            "blocks",
            baseline.root.shared_hit_blocks,
            candidate.root.shared_hit_blocks,
            false,
        ),
        metric(
            "temp_written",
            "Temp Written Blocks",
            "blocks",
            baseline.root.temp_written_blocks,
            candidate.root.temp_written_blocks,
            true,
        ),
        metric(
            "temp_read",
            "Temp Read Blocks",
            "blocks",
            baseline.root.temp_read_blocks,
            candidate.root.temp_read_blocks,
            true,
        ),
        metric(
            "wal_bytes",
            "WAL Bytes",
            "bytes",
            baseline.root.wal_bytes,
            candidate.root.wal_bytes,
            true,
        ),
        metric(
            "actual_rows",
            "Root Actual Rows",
            "rows",
            baseline.root.actual_rows_total(),
            candidate.root.actual_rows_total(),
            false,
        ),
        metric(
            "total_cost",
            "Root Total Cost",
            "cost",
            baseline.root.total_cost,
            candidate.root.total_cost,
            true,
        ),
        metric(
            "heap_fetches",
            "Heap Fetches",
            "fetches",
            sum_nodes(&baseline_nodes, |node| node.heap_fetches),
            sum_nodes(&candidate_nodes, |node| node.heap_fetches),
            true,
        ),
        metric(
            "rows_removed",
            "Rows Removed by Filter",
            "rows",
            sum_nodes(&baseline_nodes, |node| {
                node.rows_removed_by_filter
                    .map(|value| value * node.loop_count())
            }),
            sum_nodes(&candidate_nodes, |node| {
                node.rows_removed_by_filter
                    .map(|value| value * node.loop_count())
            }),
            true,
        ),
        metric(
            "workers_launched",
            "Max Workers Launched",
            "workers",
            max_nodes(&baseline_nodes, |node| node.workers_launched),
            max_nodes(&candidate_nodes, |node| node.workers_launched),
            false,
        ),
        metric(
            "node_count",
            "Plan Nodes",
            "nodes",
            Some(baseline.node_count as f64),
            Some(candidate.node_count as f64),
            false,
        ),
    ];

    let baseline_types = count_node_types(&baseline_nodes);
    let candidate_types = count_node_types(&candidate_nodes);
    let all_types = baseline_types
        .keys()
        .chain(candidate_types.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    let node_types = all_types
        .into_iter()
        .map(|node_type| {
            let baseline_count = *baseline_types.get(&node_type).unwrap_or(&0);
            let candidate_count = *candidate_types.get(&node_type).unwrap_or(&0);

            NodeTypeDelta {
                node_type,
                baseline_count,
                candidate_count,
                delta: candidate_count as isize - baseline_count as isize,
            }
        })
        .filter(|item| item.delta != 0)
        .collect::<Vec<_>>();

    let baseline_relations = relation_strategies(&baseline_nodes);
    let candidate_relations = relation_strategies(&candidate_nodes);
    let relation_names = baseline_relations
        .keys()
        .chain(candidate_relations.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    let relation_strategies = relation_names
        .into_iter()
        .map(|relation| {
            let baseline_strategies = baseline_relations
                .get(&relation)
                .cloned()
                .unwrap_or_default();
            let candidate_strategies = candidate_relations
                .get(&relation)
                .cloned()
                .unwrap_or_default();
            let changed = baseline_strategies != candidate_strategies;

            RelationStrategyChange {
                relation,
                baseline_strategies,
                candidate_strategies,
                changed,
            }
        })
        .filter(|item| item.changed)
        .collect::<Vec<_>>();

    let baseline_findings = count_findings(&baseline_analysis);
    let candidate_findings = count_findings(&candidate_analysis);
    let finding_codes = baseline_findings
        .keys()
        .chain(candidate_findings.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    let findings = finding_codes
        .into_iter()
        .map(|code| {
            let baseline_count = *baseline_findings.get(&code).unwrap_or(&0);
            let candidate_count = *candidate_findings.get(&code).unwrap_or(&0);

            FindingDelta {
                code,
                baseline_count,
                candidate_count,
                delta: candidate_count as isize - baseline_count as isize,
            }
        })
        .filter(|item| item.delta != 0)
        .collect::<Vec<_>>();

    let baseline_shape = shape_fingerprint(&baseline.root);
    let candidate_shape = shape_fingerprint(&candidate.root);
    let structure_changed = baseline_shape != candidate_shape;

    let summary = build_summary(&metrics, &relation_strategies, &findings, structure_changed);

    PlanComparison {
        metrics,
        node_types,
        relation_strategies,
        findings,
        baseline_analysis,
        candidate_analysis,
        baseline_shape,
        candidate_shape,
        structure_changed,
        summary,
    }
}

fn metric(
    key: &str,
    label: &str,
    unit: &str,
    baseline: Option<f64>,
    candidate: Option<f64>,
    lower_is_better: bool,
) -> MetricDelta {
    let delta = match (baseline, candidate) {
        (Some(left), Some(right)) => Some(right - left),
        _ => None,
    };

    let delta_pct = match (baseline, candidate) {
        (Some(left), Some(right)) if left.abs() > f64::EPSILON => {
            Some((right - left) / left * 100.0)
        }
        _ => None,
    };

    MetricDelta {
        key: key.into(),
        label: label.into(),
        unit: unit.into(),
        baseline,
        candidate,
        delta,
        delta_pct,
        lower_is_better,
    }
}

fn sum_nodes<F>(nodes: &[&PlanNode], mut value: F) -> Option<f64>
where
    F: FnMut(&PlanNode) -> Option<f64>,
{
    let mut seen = false;
    let mut total = 0.0;

    for node in nodes {
        if let Some(current) = value(node) {
            seen = true;
            total += current;
        }
    }

    seen.then_some(total)
}

fn max_nodes<F>(nodes: &[&PlanNode], mut value: F) -> Option<f64>
where
    F: FnMut(&PlanNode) -> Option<f64>,
{
    nodes.iter().filter_map(|node| value(node)).reduce(f64::max)
}

fn count_node_types(nodes: &[&PlanNode]) -> BTreeMap<String, usize> {
    let mut result = BTreeMap::new();

    for node in nodes {
        *result.entry(node.node_type.clone()).or_insert(0) += 1;
    }

    result
}

fn relation_strategies(nodes: &[&PlanNode]) -> BTreeMap<String, Vec<String>> {
    let mut result: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for node in nodes {
        let Some(relation) = &node.relation_name else {
            continue;
        };

        let relation = match &node.schema {
            Some(schema) => format!("{schema}.{relation}"),
            None => relation.clone(),
        };

        result
            .entry(relation)
            .or_default()
            .insert(node.node_type.clone());
    }

    result
        .into_iter()
        .map(|(relation, strategies)| (relation, strategies.into_iter().collect::<Vec<_>>()))
        .collect()
}

fn count_findings(analysis: &PlanAnalysis) -> BTreeMap<String, usize> {
    let mut result = BTreeMap::new();

    for finding in &analysis.findings {
        *result.entry(finding.code.clone()).or_insert(0) += 1;
    }

    result
}

fn shape_fingerprint(node: &PlanNode) -> String {
    fn walk(node: &PlanNode, output: &mut String) {
        output.push('(');
        output.push_str(&node.node_type);

        // Include object identity: the same Node Type tree over different
        // relations/indexes is not necessarily the same execution strategy.
        for attribute in [
            &node.join_type,
            &node.schema,
            &node.relation_name,
            &node.index_name,
        ] {
            output.push(':');
            output.push_str(&format!("{:?}", attribute));
        }

        for child in &node.children {
            walk(child, output);
        }

        output.push(')');
    }

    let mut output = String::new();
    walk(node, &mut output);
    output
}

fn build_summary(
    metrics: &[MetricDelta],
    relation_changes: &[RelationStrategyChange],
    finding_changes: &[FindingDelta],
    structure_changed: bool,
) -> Vec<String> {
    let mut result = vec![
        crate::i18n::text("Сравнение двух отдельных планов не контролирует кеш, конкурирующую нагрузку и условия измерений.", "Comparing two separate plans does not control for cache state, competing workload, or measurement conditions.").into(),
        crate::i18n::text("Сокращение Actual Rows или изменение количества Cache Hits само по себе не доказывает улучшение.", "A reduction in Actual Rows or change in Cache Hits alone does not prove an improvement.").into(),
    ];

    if let Some(execution) = metrics.iter().find(|metric| metric.key == "execution_time") {
        if let Some(delta_pct) = execution.delta_pct {
            if delta_pct <= -5.0 {
                result.push(format!(
                    "В этих двух замерах Execution Time candidate ниже baseline примерно на {:.1}%; для вывода об ускорении нужны повторные измерения.",
                    -delta_pct,
                ));
            } else if delta_pct >= 5.0 {
                result.push(format!(
                    "В этих двух замерах Execution Time candidate выше baseline примерно на {:.1}%; проверьте повторяемость результата.",
                    delta_pct,
                ));
            }
        }
    }

    if let Some(reads) = metrics.iter().find(|metric| metric.key == "shared_read") {
        if let Some(delta_pct) = reads.delta_pct {
            if delta_pct <= -10.0 {
                result.push(format!(
                    "Shared Read Blocks уменьшились примерно на {:.1}%.",
                    -delta_pct,
                ));
            } else if delta_pct >= 10.0 {
                result.push(format!(
                    "Shared Read Blocks выросли примерно на {:.1}%.",
                    delta_pct,
                ));
            }
        }
    }

    if structure_changed {
        result.push(
            crate::i18n::text(
                "Форма плана изменилась: дерево Node Type baseline и candidate различается.",
                "The plan shape changed: baseline and candidate Node Type trees differ.",
            )
            .into(),
        );
    } else {
        result.push(
            crate::i18n::text(
                "Основная форма дерева Node Type не изменилась.",
                "The main Node Type tree shape did not change.",
            )
            .into(),
        );
    }

    if !relation_changes.is_empty() {
        result.push(format!(
            "Изменилась scan/join strategy для {} relation(s).",
            relation_changes.len(),
        ));
    }

    let added_findings = finding_changes.iter().filter(|item| item.delta > 0).count();
    let removed_findings = finding_changes.iter().filter(|item| item.delta < 0).count();

    if removed_findings > 0 {
        result.push(format!(
            "В candidate исчезло/уменьшилось {} классов диагностических findings.",
            removed_findings,
        ));
    }

    if added_findings > 0 {
        result.push(format!(
            "В candidate появилось/усилилось {} классов диагностических findings.",
            added_findings,
        ));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::model::parse_explain_json;

    #[test]
    fn detects_different_relation_with_same_node_type() {
        let baseline =
            parse_explain_json(r#"[{"Plan":{"Node Type":"Seq Scan","Relation Name":"orders"}}]"#)
                .unwrap();
        let candidate =
            parse_explain_json(r#"[{"Plan":{"Node Type":"Seq Scan","Relation Name":"payments"}}]"#)
                .unwrap();
        let comparison = compare(&baseline, &candidate);
        assert!(comparison.structure_changed);
    }

    #[test]
    fn time_summary_is_not_a_performance_guarantee() {
        let baseline =
            parse_explain_json(r#"[{"Plan":{"Node Type":"Seq Scan"},"Execution Time":100}]"#)
                .unwrap();
        let candidate =
            parse_explain_json(r#"[{"Plan":{"Node Type":"Seq Scan"},"Execution Time":50}]"#)
                .unwrap();
        let comparison = compare(&baseline, &candidate);
        assert!(comparison
            .summary
            .iter()
            .any(|line| line.contains("повторные измерения")));
    }

    #[test]
    fn preserves_missing_metrics_as_none() {
        let baseline = parse_explain_json(r#"[{"Plan":{"Node Type":"Seq Scan"}}]"#).unwrap();
        let candidate =
            parse_explain_json(r#"[{"Plan":{"Node Type":"Seq Scan"},"Execution Time":50}]"#)
                .unwrap();
        let comparison = compare(&baseline, &candidate);
        let execution = comparison
            .metrics
            .iter()
            .find(|item| item.key == "execution_time")
            .unwrap();
        assert!(execution.delta.is_none());
        assert!(execution.delta_pct.is_none());
    }

    #[test]
    fn detects_scan_strategy_change() {
        let baseline = parse_explain_json(
            r#"[{"Plan":{"Node Type":"Seq Scan","Schema":"public","Relation Name":"orders","Plan Rows":1000}}]"#,
        )
        .unwrap();
        let candidate = parse_explain_json(
            r#"[{"Plan":{"Node Type":"Index Scan","Schema":"public","Relation Name":"orders","Index Name":"orders_idx","Plan Rows":10}}]"#,
        )
        .unwrap();

        let comparison = compare(&baseline, &candidate);
        assert!(comparison.structure_changed);
        assert_eq!(comparison.relation_strategies.len(), 1);
    }
}
