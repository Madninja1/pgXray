use super::model::{all_nodes, PlanDocument, PlanNode};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    High,
    Warning,
    Info,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlanFinding {
    pub severity: FindingSeverity,
    pub code: String,
    pub title: String,
    pub summary: String,
    pub evidence: Vec<String>,
    pub next_checks: Vec<String>,
    pub node_id: Option<usize>,
    pub node_type: Option<String>,
    pub reference_key: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlanHotspot {
    pub node_id: usize,
    pub label: String,
    pub repeated_time_ms: Option<f64>,
    pub actual_rows_total: Option<f64>,
    pub actual_loops: Option<f64>,
    pub shared_read_blocks: Option<f64>,
    pub temp_written_blocks: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlanAnalysisSummary {
    pub high_count: usize,
    pub warning_count: usize,
    pub info_count: usize,
    pub execution_time_ms: Option<f64>,
    pub root_shared_read_blocks: Option<f64>,
    pub root_shared_hit_blocks: Option<f64>,
    pub root_temp_written_blocks: Option<f64>,
    pub root_wal_bytes: Option<f64>,
    pub node_count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlanAnalysis {
    pub summary: PlanAnalysisSummary,
    pub findings: Vec<PlanFinding>,
    pub hotspots: Vec<PlanHotspot>,
}

pub fn analyze(document: &PlanDocument) -> PlanAnalysis {
    let nodes = all_nodes(document);
    let mut findings = Vec::new();

    for node in &nodes {
        analyze_node(node, &mut findings);
    }

    analyze_parent_child(&document.root, &mut findings);
    analyze_repeated_relations(&nodes, &mut findings);
    analyze_global(document, &mut findings);

    findings.sort_by(|left, right| {
        severity_rank(right.severity)
            .cmp(&severity_rank(left.severity))
            .then_with(|| left.node_id.cmp(&right.node_id))
            .then_with(|| left.code.cmp(&right.code))
    });

    let mut hotspots = nodes
        .iter()
        .filter_map(|node| {
            let repeated = node.repeated_time_ms();
            let reads = node.shared_read_blocks.unwrap_or(0.0);
            let temp = node.temp_written_blocks.unwrap_or(0.0);

            if repeated.unwrap_or(0.0) <= 0.0 && reads <= 0.0 && temp <= 0.0 {
                return None;
            }

            Some(PlanHotspot {
                node_id: node.id,
                label: node.display_name(),
                repeated_time_ms: repeated,
                actual_rows_total: node.actual_rows_total(),
                actual_loops: node.actual_loops,
                shared_read_blocks: node.shared_read_blocks,
                temp_written_blocks: node.temp_written_blocks,
            })
        })
        .collect::<Vec<_>>();

    hotspots.sort_by(|left, right| {
        score_hotspot(right)
            .partial_cmp(&score_hotspot(left))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hotspots.truncate(15);

    let high_count = findings
        .iter()
        .filter(|finding| finding.severity == FindingSeverity::High)
        .count();
    let warning_count = findings
        .iter()
        .filter(|finding| finding.severity == FindingSeverity::Warning)
        .count();
    let info_count = findings
        .iter()
        .filter(|finding| finding.severity == FindingSeverity::Info)
        .count();

    PlanAnalysis {
        summary: PlanAnalysisSummary {
            high_count,
            warning_count,
            info_count,
            execution_time_ms: document.execution_time_ms,
            root_shared_read_blocks: document.root.shared_read_blocks,
            root_shared_hit_blocks: document.root.shared_hit_blocks,
            root_temp_written_blocks: document.root.temp_written_blocks,
            root_wal_bytes: document.root.wal_bytes,
            node_count: document.node_count,
        },
        findings,
        hotspots,
    }
}

fn analyze_node(node: &PlanNode, findings: &mut Vec<PlanFinding>) {
    if node.actual_loops.unwrap_or(1.0) > 0.0 {
        if let (Some(plan_rows), Some(actual_rows)) = (node.plan_rows, node.actual_rows) {
            let ratio = cardinality_ratio(plan_rows, actual_rows);

            if ratio >= 10.0 && (plan_rows.max(actual_rows) >= 100.0) {
                let severity = if ratio >= 100.0 {
                    FindingSeverity::High
                } else {
                    FindingSeverity::Warning
                };

                findings.push(PlanFinding {
                    severity,
                    code: "cardinality_mismatch".into(),
                    title: crate::i18n::text("Сильное расхождение Plan Rows / Actual Rows", "Large Plan Rows / Actual Rows discrepancy").into(),
                    summary: format!(
                        "Planner ожидал примерно {}, фактически узел вернул примерно {} строк на loop; расхождение ~{ratio:.1}×.",
                        format_number(plan_rows),
                        format_number(actual_rows),
                    ),
                    evidence: vec![
                        format!("Plan Rows: {}", format_number(plan_rows)),
                        format!("Actual Rows: {}", format_number(actual_rows)),
                        format!("Actual Loops: {}", format_number(node.loop_count())),
                    ],
                    next_checks: vec![
                        crate::i18n::text("Проверить актуальность statistics и n_distinct/MCV/histogram по колонкам predicates.", "Check whether statistics and n_distinct/MCV/histograms are current for predicate columns.").into(),
                        crate::i18n::text("Проверить корреляцию колонок и необходимость extended statistics.", "Check column correlation and whether extended statistics are needed.").into(),
                        crate::i18n::text("Сравнить ошибку оценки на нескольких значениях параметров, а не на одном примере.", "Compare estimation errors across multiple parameter values, not just one example.").into(),
                    ],
                    node_id: Some(node.id),
                    node_type: Some(node.node_type.clone()),
                    reference_key: Some("cardinality".into()),
                });
            }
        }
    }

    let rows_removed = node.rows_removed_by_filter.unwrap_or(0.0) * node.loop_count();
    let returned = node.actual_rows_total().unwrap_or(0.0);

    if rows_removed >= 1_000.0 && rows_removed > returned.max(1.0) * 3.0 {
        findings.push(PlanFinding {
            severity: if rows_removed >= 100_000.0 {
                FindingSeverity::High
            } else {
                FindingSeverity::Warning
            },
            code: "rows_removed_by_filter".into(),
            title: crate::i18n::text("Много строк отбрасывается Filter", "Many rows discarded by Filter").into(),
            summary: crate::i18n::text("Узел читает заметно больше строк, чем возвращает после Filter. Это может быть нормой, но часто указывает на позднюю фильтрацию или predicate, который не попал в access condition.", "The node reads substantially more rows than it returns after Filter. This may be normal, but often indicates late filtering or a predicate absent from the access condition.").into(),
            evidence: vec![
                format!("Rows Removed by Filter × loops: {}", format_number(rows_removed)),
                format!("Returned rows × loops: {}", format_number(returned)),
                node.filter
                    .as_ref()
                    .map(|value| format!("Filter: {value}"))
                    .unwrap_or_else(|| crate::i18n::text("Filter: <нет текста>", "Filter: <no text>").into()),
            ],
            next_checks: vec![
                crate::i18n::text("Сравнить Filter с Index Cond / Recheck Cond.", "Compare Filter with Index Cond / Recheck Cond.").into(),
                crate::i18n::text("Проверить, можно ли переписать predicate так, чтобы он применялся раньше.", "Check whether the predicate can be rewritten to apply earlier.").into(),
                crate::i18n::text("Не создавать индекс автоматически: сначала проверить, способен ли текущий SQL использовать нужное условие как access condition.", "Do not create an index automatically: first verify whether the current SQL can use the condition as an access condition.").into(),
            ],
            node_id: Some(node.id),
            node_type: Some(node.node_type.clone()),
            reference_key: Some("index_cond_vs_filter".into()),
        });
    }

    let join_removed = node.rows_removed_by_join_filter.unwrap_or(0.0) * node.loop_count();

    if join_removed >= 1_000.0 && join_removed > returned.max(1.0) * 3.0 {
        findings.push(PlanFinding {
            severity: if join_removed >= 100_000.0 {
                FindingSeverity::High
            } else {
                FindingSeverity::Warning
            },
            code: "rows_removed_by_join_filter".into(),
            title: "Join Filter отбрасывает большой объём строк".into(),
            summary: "Join сформировал много candidate pairs, которые затем были отброшены Join Filter. Причина может быть в join order, слабом join condition или позднем predicate.".into(),
            evidence: vec![
                format!(
                    "Rows Removed by Join Filter × loops: {}",
                    format_number(join_removed),
                ),
                node.join_filter
                    .as_ref()
                    .map(|value| format!("Join Filter: {value}"))
                    .unwrap_or_else(|| "Join Filter: <нет текста>".into()),
            ],
            next_checks: vec![
                "Проверить основной Hash/Merge/Index Cond и placement дополнительного Join Filter.".into(),
                "Проверить cardinality обеих сторон join и порядок соединений.".into(),
                "Проверить, можно ли применить часть predicate до join без изменения semantics.".into(),
            ],
            node_id: Some(node.id),
            node_type: Some(node.node_type.clone()),
            reference_key: Some("rows_removed".into()),
        });
    }

    if node.node_type == "Seq Scan" {
        let returned = node.actual_rows_total();
        let scanned = returned
            .map(|rows| rows + node.rows_removed_by_filter.unwrap_or(0.0) * node.loop_count())
            .or(node.plan_rows);
        let rows = scanned.unwrap_or(0.0);
        let reads = node.shared_read_blocks.unwrap_or(0.0);

        if rows >= 100_000.0 || reads >= 10_000.0 {
            findings.push(PlanFinding {
                severity: FindingSeverity::Warning,
                code: "large_seq_scan".into(),
                title: "Крупный Seq Scan".into(),
                summary: "Sequential scan сам по себе не является ошибкой: он может быть оптимальным для большой доли таблицы. Здесь объём достаточно велик, чтобы проверить селективность и I/O.".into(),
                evidence: vec![
                    format!("Оценка просмотренных строк (returned + filtered, по всем loops): {}", format_number(rows)),
                    format!("Shared Read Blocks: {}", format_number(reads)),
                    node.filter
                        .as_ref()
                        .map(|value| format!("Filter: {value}"))
                        .unwrap_or_else(|| "Filter: <нет>".into()),
                ],
                next_checks: vec![
                    "Оценить долю таблицы, реально нужную запросу.".into(),
                    "Проверить существующие индексы и predicates; не считать Seq Scan доказательством необходимости нового индекса.".into(),
                    "Если план parallel, проверить фактическое участие workers.".into(),
                ],
                node_id: Some(node.id),
                node_type: Some(node.node_type.clone()),
                reference_key: Some("seq_scan".into()),
            });
        }
    }

    if node.node_type == "Bitmap Heap Scan" && node.lossy_heap_blocks.unwrap_or(0.0) > 0.0 {
        let lossy = node.lossy_heap_blocks.unwrap_or(0.0);
        let exact = node.exact_heap_blocks.unwrap_or(0.0);

        findings.push(PlanFinding {
            severity: if lossy > exact.max(1.0) {
                FindingSeverity::Warning
            } else {
                FindingSeverity::Info
            },
            code: "lossy_bitmap".into(),
            title: "Bitmap Heap Scan использует lossy pages".into(),
            summary: "Часть bitmap стала lossy, поэтому PostgreSQL должен перепроверять tuple на heap pages. Это часто связано с размером bitmap/work_mem и большим входным набором.".into(),
            evidence: vec![
                format!("Lossy Heap Blocks: {}", format_number(lossy)),
                format!("Exact Heap Blocks: {}", format_number(exact)),
                format!(
                    "Rows Removed by Index Recheck × loops: {}",
                    format_number(
                        node.rows_removed_by_index_recheck
                            .unwrap_or(0.0)
                            * node.loop_count(),
                    ),
                ),
            ],
            next_checks: vec![
                "Проверить Recheck Cond и объём bitmap input.".into(),
                "Сначала проверить селективность predicates; увеличение work_mem — только controlled experiment.".into(),
            ],
            node_id: Some(node.id),
            node_type: Some(node.node_type.clone()),
            reference_key: Some("bitmap_heap_scan".into()),
        });
    }

    if matches!(node.node_type.as_str(), "Index Scan" | "Index Only Scan") {
        let loops = node.loop_count();
        let repeated_time = node.repeated_time_ms().unwrap_or(0.0);

        if loops >= 1_000.0 && repeated_time >= 500.0 {
            findings.push(PlanFinding {
                severity: if loops >= 10_000.0 && repeated_time >= 5_000.0 {
                    FindingSeverity::High
                } else {
                    FindingSeverity::Warning
                },
                code: "many_index_lookups".into(),
                title: "Много повторных index lookup".into(),
                summary: "Один lookup может быть дешёвым, но при тысячах/десятках тысяч loops суммарная повторная работа становится bottleneck.".into(),
                evidence: vec![
                    format!("Actual Loops: {}", format_number(loops)),
                    format!("Actual Total Time × loops: ~{:.1} ms", repeated_time),
                    node.index_name
                        .as_ref()
                        .map(|value| format!("Index: {value}"))
                        .unwrap_or_else(|| "Index: <не указан>".into()),
                ],
                next_checks: vec![
                    "Проверить объём outer rows у родительского Nested Loop.".into(),
                    "Проверить, нельзя ли уменьшить outer set до дорогих JOIN/lookup (например, корректным ранним LIMIT).".into(),
                    "Сравнить Index Cond и Filter; часть условий может применяться слишком поздно.".into(),
                ],
                node_id: Some(node.id),
                node_type: Some(node.node_type.clone()),
                reference_key: Some("actual_loops".into()),
            });
        }

        if node.index_cond.is_none() && node.filter.is_some() && loops >= 100.0 {
            findings.push(PlanFinding {
                severity: FindingSeverity::Warning,
                code: "index_scan_filter_without_index_cond".into(),
                title: "Index Scan с Filter, но без Index Cond".into(),
                summary: "Индексный узел выполняется, но predicate виден только как Filter. Это может означать, что условие не используется для поиска по индексу.".into(),
                evidence: vec![
                    format!("Actual Loops: {}", format_number(loops)),
                    format!("Filter: {}", node.filter.as_deref().unwrap_or("<нет>")),
                ],
                next_checks: vec![
                    "Проверить форму выражения, OR и коррелированные условия.".into(),
                    "Проверить definition индекса и порядок ключей.".into(),
                    "Не делать вывод только по названию Index Scan: важен именно Index Cond.".into(),
                ],
                node_id: Some(node.id),
                node_type: Some(node.node_type.clone()),
                reference_key: Some("index_cond_vs_filter".into()),
            });
        }
    }

    if node.node_type == "Index Only Scan" {
        let heap_fetches = node.heap_fetches.unwrap_or(0.0);
        let rows = node.actual_rows_total().unwrap_or(0.0);

        if heap_fetches > 0.0 && heap_fetches > rows.max(1.0) * 0.05 {
            findings.push(PlanFinding {
                severity: FindingSeverity::Warning,
                code: "index_only_heap_fetches".into(),
                title: "Index Only Scan всё ещё ходит в heap".into(),
                summary: "Index Only Scan получает заметную долю строк через heap. Частая причина — недостаточная видимость страниц в visibility map.".into(),
                evidence: vec![
                    format!("Heap Fetches: {}", format_number(heap_fetches)),
                    format!("Rows × loops: {}", format_number(rows)),
                ],
                next_checks: vec![
                    "Проверить relallvisible / relpages и состояние autovacuum.".into(),
                    "Не считать covering index успешным только по Node Type — смотреть Heap Fetches.".into(),
                ],
                node_id: Some(node.id),
                node_type: Some(node.node_type.clone()),
                reference_key: Some("heap_fetches".into()),
            });
        }
    }

    let temp_written = node.temp_written_blocks.unwrap_or(0.0);
    if temp_written > 0.0 {
        findings.push(PlanFinding {
            severity: if temp_written >= 10_000.0 {
                FindingSeverity::High
            } else {
                FindingSeverity::Warning
            },
            code: "temp_spill".into(),
            title: "Операция пишет во временные блоки".into(),
            summary: "Узел использует temp I/O. Причина может быть в Sort/Hash/Aggregate или в объёме промежуточных данных; увеличение work_mem не должно быть первым автоматическим выводом.".into(),
            evidence: vec![
                format!("Temp Written Blocks: {}", format_number(temp_written)),
                format!("Temp Read Blocks: {}", format_number(node.temp_read_blocks.unwrap_or(0.0))),
            ],
            next_checks: vec![
                "Определить оператор, который spill-ится, и размер входного набора.".into(),
                "Сначала проверить форму SQL и cardinality, затем уже тестировать memory settings.".into(),
            ],
            node_id: Some(node.id),
            node_type: Some(node.node_type.clone()),
            reference_key: Some("temp_io".into()),
        });
    }

    if node.node_type.contains("Sort")
        && node
            .sort_space_type
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case("disk"))
    {
        findings.push(PlanFinding {
            severity: FindingSeverity::Warning,
            code: "sort_on_disk".into(),
            title: "Sort выполняется на disk".into(),
            summary: "Sort вышел за доступную память для этой операции и использовал disk-backed temporary space.".into(),
            evidence: vec![
                format!("Sort Method: {}", node.sort_method.as_deref().unwrap_or("<unknown>")),
                format!("Sort Space Used: {} kB", format_number(node.sort_space_used_kb.unwrap_or(0.0))),
            ],
            next_checks: vec![
                "Проверить объём строк до Sort и возможность уменьшить его раньше.".into(),
                "Только после этого тестировать work_mem в контролируемой сессии.".into(),
            ],
            node_id: Some(node.id),
            node_type: Some(node.node_type.clone()),
            reference_key: Some("sort".into()),
        });
    }

    if node.node_type == "Hash" && node.hash_batches.unwrap_or(1.0) > 1.0 {
        findings.push(PlanFinding {
            severity: FindingSeverity::Warning,
            code: "hash_batches".into(),
            title: "Hash разбит на несколько batches".into(),
            summary: "Hash table не поместилась в один batch. Это обычно означает дополнительную работу и потенциальный temp I/O.".into(),
            evidence: vec![
                format!("Hash Batches: {}", format_number(node.hash_batches.unwrap_or(0.0))),
                format!("Peak Memory Usage: {} kB", format_number(node.peak_memory_kb.unwrap_or(0.0))),
            ],
            next_checks: vec![
                "Проверить ошибку cardinality на build-side.".into(),
                "Проверить объём входа и только потом экспериментировать с work_mem/hash_mem_multiplier.".into(),
            ],
            node_id: Some(node.id),
            node_type: Some(node.node_type.clone()),
            reference_key: Some("hash".into()),
        });
    }

    if let (Some(planned), Some(launched)) = (node.workers_planned, node.workers_launched) {
        if planned > 0.0 && launched < planned {
            findings.push(PlanFinding {
                severity: FindingSeverity::Info,
                code: "workers_not_fully_launched".into(),
                title: "Запущено меньше parallel workers, чем планировалось".into(),
                summary: "Planner ожидал больше workers. Это не доказывает проблему, но может менять фактический runtime.".into(),
                evidence: vec![
                    format!("Workers Planned: {}", format_number(planned)),
                    format!("Workers Launched: {}", format_number(launched)),
                    format!("Workers with per-worker metrics: {}", node.workers_reported),
                ],
                next_checks: vec![
                    "Проверить max_parallel_workers, max_parallel_workers_per_gather и текущую конкуренцию за workers.".into(),
                    "Смотреть фактические rows каждого worker, а не только Workers Launched.".into(),
                ],
                node_id: Some(node.id),
                node_type: Some(node.node_type.clone()),
                reference_key: Some("parallel_workers".into()),
            });
        }
    }

    if node.node_type == "Memoize" {
        let hits = node.cache_hits.unwrap_or(0.0);
        let misses = node.cache_misses.unwrap_or(0.0);
        let total = hits + misses;

        if total >= 1_000.0 && misses / total >= 0.90 {
            findings.push(PlanFinding {
                severity: FindingSeverity::Info,
                code: "memoize_low_hit_rate".into(),
                title: "Memoize почти не переиспользует lookup results".into(),
                summary: "Memoize полезен при повторяющихся parameterized keys. Здесь большая часть обращений — cache misses, поэтому кэш мало сокращает inner work.".into(),
                evidence: vec![
                    format!("Cache Hits: {}", format_number(hits)),
                    format!("Cache Misses: {}", format_number(misses)),
                    format!(
                        "Cache Evictions: {}",
                        format_number(node.cache_evictions.unwrap_or(0.0)),
                    ),
                    format!(
                        "Cache Overflows: {}",
                        format_number(node.cache_overflows.unwrap_or(0.0)),
                    ),
                ],
                next_checks: vec![
                    "Проверить, действительно ли outer side повторяет keys; при почти уникальных keys Memoize ожидаемо мало помогает.".into(),
                    "Смотреть на источник большого outer set, а не только на Memoize.".into(),
                ],
                node_id: Some(node.id),
                node_type: Some(node.node_type.clone()),
                reference_key: Some("memoize".into()),
            });
        }
    }

    if node.workers_launched.unwrap_or(0.0) >= 2.0
        && node.workers_reported >= 2
        && node.workers_with_rows * 2 < node.workers_reported
    {
        findings.push(PlanFinding {
            severity: FindingSeverity::Warning,
            code: "parallel_worker_underuse".into(),
            title: "Часть parallel workers не получила реальную работу".into(),
            summary: "Workers были запущены, но rows появились только у меньшинства. Это может указывать на skew, parameterized branch или leader-heavy plan shape.".into(),
            evidence: vec![
                format!("Workers reported: {}", node.workers_reported),
                format!("Workers with rows: {}", node.workers_with_rows),
                format!(
                    "Worker rows total: {}",
                    format_number(node.worker_rows_total.unwrap_or(0.0)),
                ),
            ],
            next_checks: vec![
                "В PEV2 открыть Workers и сравнить rows/time по каждому worker.".into(),
                "Проверить, не начинается ли дорогая downstream-ветка после Gather или в leader process.".into(),
                "Проверить plan shape на разных параметрах — plan cliff может менять степень parallelism.".into(),
            ],
            node_id: Some(node.id),
            node_type: Some(node.node_type.clone()),
            reference_key: Some("parallel_workers".into()),
        });
    }
}

fn analyze_parent_child(node: &PlanNode, findings: &mut Vec<PlanFinding>) {
    if node.node_type.starts_with("Nested Loop") && node.children.len() >= 2 {
        let inner = &node.children[1];
        let inner_loops = inner.loop_count();
        let inner_time = inner.repeated_time_ms().unwrap_or(0.0);

        if inner_loops >= 1_000.0 && inner_time >= 500.0 {
            findings.push(PlanFinding {
                severity: if inner_loops >= 10_000.0 && inner_time >= 5_000.0 {
                    FindingSeverity::High
                } else {
                    FindingSeverity::Warning
                },
                code: "nested_loop_expensive_inner".into(),
                title: "Nested Loop многократно выполняет дорогую inner-ветку".into(),
                summary: "Nested Loop сам по себе нормален. Здесь подозрительно сочетание большого количества inner loops и заметной стоимости повторной работы.".into(),
                evidence: vec![
                    format!("Nested Loop node: #{}", node.id),
                    format!("Inner node: {}", inner.display_name()),
                    format!("Inner Actual Loops: {}", format_number(inner_loops)),
                    format!("Inner Actual Total Time × loops: ~{inner_time:.1} ms"),
                ],
                next_checks: vec![
                    "Понять, почему outer side производит столько строк.".into(),
                    "Проверить predicates inner scan: Index Cond vs Filter.".into(),
                    "Проверить, можно ли корректно сократить outer set до дорогой join-работы.".into(),
                ],
                node_id: Some(node.id),
                node_type: Some(node.node_type.clone()),
                reference_key: Some("nested_loop".into()),
            });
        }
    }

    if node.node_type == "Limit" && !node.children.is_empty() {
        let child = &node.children[0];
        let limit_rows = node.actual_rows_total().or(node.plan_rows).unwrap_or(0.0);
        let child_rows = child.actual_rows_total().or(child.plan_rows).unwrap_or(0.0);
        let child_time = child.repeated_time_ms().unwrap_or(0.0);

        if limit_rows > 0.0 && child_rows >= 1_000.0 && child_rows > limit_rows * 10.0 {
            findings.push(PlanFinding {
                severity: if child_time >= 5_000.0 {
                    FindingSeverity::High
                } else {
                    FindingSeverity::Warning
                },
                code: "late_limit".into(),
                title: "LIMIT отбрасывает результат после большого объёма работы".into(),
                summary: "Маленький итоговый LIMIT не гарантирует раннюю остановку. Под LIMIT уже приходит существенно больший набор строк.".into(),
                evidence: vec![
                    format!("Limit rows: {}", format_number(limit_rows)),
                    format!("Child rows: {}", format_number(child_rows)),
                    format!("Child repeated time: ~{child_time:.1} ms"),
                ],
                next_checks: vec![
                    "Проверить, можно ли семантически выполнить filter/order/limit до дорогих lookup/JOIN.".into(),
                    "Перед rewrite доказать корректность результата и проверить ties на границе LIMIT.".into(),
                ],
                node_id: Some(node.id),
                node_type: Some(node.node_type.clone()),
                reference_key: Some("limit".into()),
            });
        }
    }

    for child in &node.children {
        analyze_parent_child(child, findings);
    }
}

fn analyze_repeated_relations(nodes: &[&PlanNode], findings: &mut Vec<PlanFinding>) {
    let mut relations: BTreeMap<String, Vec<&PlanNode>> = BTreeMap::new();

    for node in nodes {
        let Some(relation) = &node.relation_name else {
            continue;
        };

        let qualified = match &node.schema {
            Some(schema) => format!("{schema}.{relation}"),
            None => relation.clone(),
        };

        relations.entry(qualified).or_default().push(node);
    }

    for (relation, scans) in relations {
        if scans.len() < 2 {
            continue;
        }

        let heavy = scans
            .iter()
            .filter(|node| {
                node.actual_rows_total().unwrap_or(0.0) >= 10_000.0
                    || node.shared_read_blocks.unwrap_or(0.0) >= 1_000.0
                    || node.repeated_time_ms().unwrap_or(0.0) >= 1_000.0
            })
            .count();

        if heavy == 0 {
            continue;
        }

        findings.push(PlanFinding {
            severity: FindingSeverity::Info,
            code: "repeated_relation_scan".into(),
            title: "Одна relation читается несколькими узлами".into(),
            summary: format!(
                "Relation {relation} встречается в {} scan-узлах. Это может быть нормальной частью плана, но для VIEW/CTE полезно проверить архитектурный повтор.",
                scans.len(),
            ),
            evidence: scans
                .iter()
                .map(|node| node.display_name())
                .collect(),
            next_checks: vec![
                "Проверить, не сканируется ли одна большая базовая таблица повторно через вложенную VIEW/CTE.".into(),
                "Не удалять JOIN автоматически: сначала доказать uniqueness и кратность результата.".into(),
            ],
            node_id: scans.first().map(|node| node.id),
            node_type: None,
            reference_key: Some("repeated_scan".into()),
        });
    }
}

fn analyze_global(document: &PlanDocument, findings: &mut Vec<PlanFinding>) {
    if let (Some(jit), Some(execution)) = (document.jit_total_time_ms, document.execution_time_ms) {
        if execution > 0.0 && jit / execution >= 0.10 && jit >= 5.0 {
            findings.push(PlanFinding {
                severity: FindingSeverity::Info,
                code: "jit_share".into(),
                title: "JIT занимает заметную долю execution time".into(),
                summary: format!(
                    "JIT ~{jit:.1} ms из execution ~{execution:.1} ms ({:.1}%). На коротких запросах compilation overhead иногда превышает выгоду.",
                    jit / execution * 100.0,
                ),
                evidence: vec![
                    format!("JIT Total: {jit:.3} ms"),
                    format!("Execution Time: {execution:.3} ms"),
                ],
                next_checks: vec![
                    "Сравнить несколько повторных запусков и не путать JIT с cache warm-up.".into(),
                    "Не отключать JIT глобально по одному запросу; сначала проверить controlled session experiment.".into(),
                ],
                node_id: None,
                node_type: None,
                reference_key: Some("jit".into()),
            });
        }
    }

    let mut seen = BTreeSet::new();
    findings.retain(|finding| {
        let key = format!("{}:{:?}:{}", finding.code, finding.node_id, finding.title);
        seen.insert(key)
    });
}

fn cardinality_ratio(plan_rows: f64, actual_rows: f64) -> f64 {
    let low = plan_rows.min(actual_rows).max(1.0);
    let high = plan_rows.max(actual_rows).max(1.0);
    high / low
}

fn format_number(value: f64) -> String {
    if value.abs() >= 1_000_000.0 {
        format!("{:.2}M", value / 1_000_000.0)
    } else if value.abs() >= 1_000.0 {
        format!("{:.1}k", value / 1_000.0)
    } else if (value.fract()).abs() < f64::EPSILON {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}

fn severity_rank(value: FindingSeverity) -> u8 {
    match value {
        FindingSeverity::High => 3,
        FindingSeverity::Warning => 2,
        FindingSeverity::Info => 1,
    }
}

fn score_hotspot(value: &PlanHotspot) -> f64 {
    value.repeated_time_ms.unwrap_or(0.0)
        + value.shared_read_blocks.unwrap_or(0.0) * 0.02
        + value.temp_written_blocks.unwrap_or(0.0) * 0.05
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::model::parse_explain_json;

    #[test]
    fn skipped_subplan_is_not_a_cardinality_mismatch() {
        let source = r#"[{"Plan":{"Node Type":"Nested Loop","Plan Rows":1,"Actual Rows":0,"Actual Loops":1,"Plans":[{"Node Type":"Seq Scan","Plan Rows":25860792,"Actual Rows":0,"Actual Loops":0}]}}]"#;
        let document = parse_explain_json(source).unwrap();
        assert!(!analyze(&document)
            .findings
            .iter()
            .any(|item| item.code == "cardinality_mismatch"));
    }

    #[test]
    fn detects_selective_large_seq_scan() {
        let source = r#"[{"Plan":{"Node Type":"Seq Scan","Plan Rows":1,"Actual Rows":1,"Actual Loops":1,"Rows Removed by Filter":200000,"Filter":"id > 10"},"Execution Time":100}]"#;
        let document = parse_explain_json(source).unwrap();
        assert!(analyze(&document)
            .findings
            .iter()
            .any(|finding| finding.code == "large_seq_scan"));
    }

    #[test]
    fn does_not_flag_small_seq_scan() {
        let document = parse_explain_json(r#"[{"Plan":{"Node Type":"Seq Scan","Plan Rows":100,"Actual Rows":50,"Actual Loops":1}}]"#).unwrap();
        assert!(!analyze(&document)
            .findings
            .iter()
            .any(|finding| finding.code == "large_seq_scan"));
    }

    #[test]
    fn detects_many_nested_loop_lookups() {
        let source = r#"[
          {
            "Plan": {
              "Node Type": "Nested Loop",
              "Actual Rows": 10000,
              "Actual Loops": 1,
              "Plans": [
                {"Node Type":"Seq Scan","Actual Rows":10000,"Actual Loops":1},
                {"Node Type":"Index Scan","Actual Rows":1,"Actual Loops":10000,"Actual Total Time":1.0,"Index Name":"idx"}
              ]
            },
            "Execution Time": 12000
          }
        ]"#;

        let document = parse_explain_json(source).unwrap();
        let analysis = analyze(&document);

        assert!(analysis
            .findings
            .iter()
            .any(|finding| { finding.code == "nested_loop_expensive_inner" }));
    }
}
