use super::model::*;

pub(super) fn interpret_snapshot(
    overview: &XidOverview,
    relations: &XidSection<Vec<XidRelationAge>>,
    triggers: &XidSection<Vec<XidAutovacuumTrigger>>,
    vacuums: &XidSection<Vec<XidVacuum>>,
    backends: &XidSection<Vec<XidBackend>>,
    slots: &XidSection<Vec<XidSlot>>,
    prepared: &XidSection<XidPreparedSummary>,
) -> XidInterpretation {
    let mut items = Vec::new();

    let mut limitations = Vec::new();

    if !overview.autovacuum {
        items.push(
            interpretation_item(
                "global-autovacuum-off",
                crate::i18n::text("Глобальный autovacuum выключен", "Global autovacuum is disabled"),
                vec![
                    "current_setting('autovacuum') = off".into(),
                ],
                vec![
                    crate::i18n::text("Normal autovacuum triggers из этого снимка нельзя трактовать как ожидаемый автоматический запуск worker.", "Normal autovacuum triggers in this snapshot must not be interpreted as proof that a worker should start automatically.").into(),
                    crate::i18n::text("Сам этот флаг не объясняет текущий XID age и не доказывает наличие freeze-проблемы.", "This flag alone does not explain the current XID age or prove a freeze problem.").into(),
                ],
                vec![
                    crate::i18n::text("Проверить, является ли отключение autovacuum осознанной политикой этого сервера.", "Verify whether autovacuum was deliberately disabled on this server.").into(),
                    crate::i18n::text("Проверить фактический VACUUM lifecycle и изменение XID age между снимками.", "Check the actual VACUUM lifecycle and XID age changes between snapshots.").into(),
                ],
            ),
        );
    }

    let oldest_relation = relations.data.as_ref().and_then(|rows| rows.first());

    if let Some(relation) = oldest_relation {
        let mut observed = vec![
            format!(
                "{} = {}.{} (OID {}, {})",
                crate::i18n::text("Самая старая relation", "Oldest relation"),
                relation.schema_name,
                relation.relation_name,
                relation.relation_oid,
                relation_kind_label(&relation.relation_type,),
            ),
            format!(
                "XID age = {}; effective autovacuum_freeze_max_age = {} (source: {})",
                relation.xid_age, relation.effective_freeze_max_age, relation.freeze_max_source,
            ),
        ];

        if let Some(percent) = relation.pct_of_freeze_max {
            observed.push(format!(
                "Отношение к configured effective freeze max = {:.1}%",
                percent,
            ));
        }

        let reached_freeze_max = relation.xid_age >= relation.effective_freeze_max_age;

        let meaning = if reached_freeze_max {
            vec![
                    crate::i18n::text("Relation достигла или превысила свой configured autovacuum_freeze_max_age.", "The relation has reached or exceeded its configured autovacuum_freeze_max_age.").into(),
                    crate::i18n::text("Это делает anti-wraparound VACUUM непосредственно релевантным для проверки, но не означает, что wraparound уже произошёл или PostgreSQL находится в failure state.", "This warrants checking anti-wraparound VACUUM, but does not mean wraparound has already happened or PostgreSQL is in a failure state.").into(),
                ]
        } else {
            vec![
                    crate::i18n::text("Это relation с самым старым freeze horizon среди relations текущего snapshot.", "This relation has the oldest freeze horizon among relations in the current snapshot.").into(),
                    crate::i18n::text("Процент показывает положение относительно configured threshold, а не универсальную severity pgXray.", "The percentage indicates proximity to the configured threshold, not a universal pgXray severity.").into(),
                ]
        };

        let next_steps = if reached_freeze_max {
            vec![
                    crate::i18n::text("Проверить, виден ли для relation anti-wraparound VACUUM.", "Check whether anti-wraparound VACUUM is running for the relation.").into(),
                    crate::i18n::text("Проверить backend XMIN, replication slots и prepared transactions.", "Check backend XMIN, replication slots, and prepared transactions.").into(),
                    crate::i18n::text("Сохранить следующий snapshot и проверить, уменьшается ли age или продолжает расти.", "Save another snapshot and check whether the age decreases or continues rising.").into(),
                ]
        } else {
            vec![
                    crate::i18n::text("Сопоставить relation с normal autovacuum triggers.", "Compare the relation with normal autovacuum triggers.").into(),
                    crate::i18n::text("Проверить blockers и текущий VACUUM, если эта relation является причиной операционного alert.", "Inspect blockers and active VACUUM if this relation triggered an operational alert.").into(),
                    crate::i18n::text("Для оценки срочности использовать XID rate после накопления snapshots.", "Use the XID rate after collecting snapshots to assess urgency.").into(),
                ]
        };

        items.push(interpretation_item(
            "oldest-relation",
            crate::i18n::text("Самый старый freeze horizon", "Oldest freeze horizon"),
            observed,
            meaning,
            next_steps,
        ));

        if relation.relation_type == "t" {
            let mut observed = vec![format!(
                "Старейшая relation {}.{} имеет relkind=t (TOAST).",
                relation.schema_name, relation.relation_name,
            )];

            if let (Some(parent_schema), Some(parent_relation)) = (
                relation.parent_schema.as_deref(),
                relation.parent_relation.as_deref(),
            ) {
                observed.push(format!(
                    "Parent = {}.{}{}",
                    parent_schema,
                    parent_relation,
                    relation
                        .parent_xid_age
                        .map(|age| { format!("; parent XID age = {age}",) })
                        .unwrap_or_default(),
                ));
            }

            items.push(
                interpretation_item(
                    "toast-parent",
                    "Старейшая relation — TOAST",
                    observed,
                    vec![
                        crate::i18n::text("TOAST и parent имеют собственные freeze horizons; старый TOAST не означает автоматически такой же age у parent table.", "TOAST and its parent have separate freeze horizons; an old TOAST relation does not imply the same age for the parent table.").into(),
                    ],
                    vec![
                        "Сравнить XID age TOAST и parent.".into(),
                        "При дальнейшем расследовании сохранять identity обеих relations.".into(),
                    ],
                ),
            );
        }

        if let Some(trigger) = triggers.data.as_ref().and_then(|rows| {
            rows.iter()
                .find(|row| row.relation_oid == relation.relation_oid)
        }) {
            let active_vacuum = vacuums.data.as_ref().and_then(|rows| {
                rows.iter()
                    .find(|row| row.relation_oid == relation.relation_oid)
            });

            // A disabled INSERT trigger is known; a missing estimate for an
            // enabled trigger is not evidence that it was not reached.
            let insert_disabled = trigger.effective_insert_threshold < 0.0;
            let insert_trigger = match trigger.estimated_insert_trigger {
                Some(value) => value.to_string(),
                None if insert_disabled => "disabled".into(),
                None => "неизвестно (reltuples не оценён)".into(),
            };

            let dead_reached = trigger
                .estimated_dead_tuple_trigger
                .map(|_| trigger.dead_trigger_reached);
            let insert_reached = if insert_disabled {
                None
            } else {
                trigger
                    .estimated_insert_trigger
                    .map(|_| trigger.insert_trigger_reached)
            };
            let insert_reached_text = if insert_disabled {
                "disabled"
            } else {
                optional_bool(insert_reached)
            };

            let observed = vec![
                format!("autovacuum_enabled = {}", trigger.autovacuum_enabled,),
                format!(
                    "dead tuples = {}; calculated dead trigger = {}; reached = {}",
                    trigger.estimated_dead_rows,
                    trigger
                        .estimated_dead_tuple_trigger
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "неизвестно (reltuples не оценён)".into()),
                    optional_bool(dead_reached),
                ),
                format!(
                    "inserts since vacuum = {}; calculated insert trigger = {}; reached = {}",
                    trigger.inserts_since_vacuum, insert_trigger, insert_reached_text,
                ),
            ];

            let any_trigger_reached = dead_reached == Some(true) || insert_reached == Some(true);
            let thresholds_known =
                dead_reached.is_some() && (insert_disabled || insert_reached.is_some());

            let (meaning, next_steps) = if !trigger.autovacuum_enabled {
                (
                        vec![
                            "Для этой relation collector получил effective autovacuum_enabled=false.".into(),
                            "Отсутствие normal autovacuum при таком состоянии нельзя считать scheduler failure.".into(),
                        ],
                        vec![
                            "Проверить global setting и reloptions relation.".into(),
                            "Freeze lifecycle оценивать отдельно по XID age, текущему VACUUM и последующим snapshots.".into(),
                        ],
                    )
            } else if any_trigger_reached {
                if let Some(vacuum) = active_vacuum {
                    (
                            vec![
                                "Как минимум один normal DML trigger достигнут, и VACUUM этой relation виден в текущем snapshot.".into(),
                                "Сам факт текущего VACUUM ещё не показывает, успевает ли он продвигать freeze horizon достаточно быстро.".into(),
                            ],
                            vec![
                                format!(
                                    "Наблюдать PID {} / phase '{}' и повторно измерить XID age после завершения.",
                                    vacuum.pid,
                                    vacuum.phase,
                                ),
                                "Не запускать второй VACUUM поверх уже работающего только из-за этого snapshot.".into(),
                            ],
                        )
                } else if vacuums.data.is_none() {
                    (
                        vec![
                            "Normal DML trigger достигнут, но секция текущего VACUUM недоступна: наличие worker не проверено.".into(),
                        ],
                        vec![
                            "Восстановить сбор секции VACUUM и повторить snapshot прежде, чем оценивать autovacuum scheduling.".into(),
                        ],
                    )
                } else {
                    (
                            vec![
                                "Как минимум один normal DML trigger достигнут, но текущий VACUUM этой relation в момент snapshot не виден.".into(),
                                "Это кандидат на отдельную проверку autovacuum scheduling/performance, а не доказательство неисправности: snapshot мгновенный.".into(),
                            ],
                            vec![
                                "Повторить snapshot и проверить, сохраняется ли сочетание trigger reached + no VACUUM.".into(),
                                "Если сохраняется, исследовать autovacuum workers, locks, competing databases, cost throttling, PostgreSQL logs и I/O pressure.".into(),
                            ],
                        )
                }
            } else if !thresholds_known {
                (
                    vec![
                        "Нельзя установить, достигнуты ли все обычные DML-пороги: как минимум один применимый порог неизвестен.".into(),
                        "Отсутствие подтверждённого превышения не означает, что пороги не достигнуты.".into(),
                    ],
                    vec![
                        "Проверить оценку reltuples и доступность расчётных порогов для relation.".into(),
                        "Не делать вывод об отсутствии normal autovacuum trigger по неизвестным значениям.".into(),
                    ],
                )
            } else {
                (
                        vec![
                            "Обычные dead-tuple и insert triggers для старейшей relation в текущем snapshot не достигнуты.".into(),
                            "Это само по себе не означает неисправность autovacuum; для static/cold или append-only relation такое состояние может быть штатным до freeze-driven VACUUM.".into(),
                        ],
                        vec![
                            "Посмотреть cumulative DML counters вместе со stats_reset.".into(),
                            "Не уменьшать normal cleanup thresholds только ради XID age без отдельного анализа workload.".into(),
                        ],
                    )
            };

            items.push(interpretation_item(
                "normal-autovacuum-trigger",
                "Normal autovacuum для старейшей relation",
                observed,
                meaning,
                next_steps,
            ));

            if !any_trigger_reached && thresholds_known && trigger.autovacuum_enabled {
                if let Some((title, pattern)) = workload_pattern(trigger) {
                    items.push(
                        interpretation_item(
                            "workload-pattern",
                            title,
                            vec![
                                format!(
                                    "Cumulative counters since stats_reset: INSERT={}, UPDATE={}, DELETE={}, HOT={}.",
                                    trigger.tuples_inserted,
                                    trigger.tuples_updated,
                                    trigger.tuples_deleted,
                                    trigger.hot_updates,
                                ),
                                format!(
                                    "stats_reset = {}",
                                    trigger
                                        .stats_reset
                                        .as_deref()
                                        .unwrap_or(
                                            "unknown",
                                        ),
                                ),
                            ],
                            vec![
                                pattern,
                                "Это описание cumulative counters в доступном статистическом окне, а не доказанная workload-классификация и не rate.".into(),
                            ],
                            vec![
                                "Подтвердить профиль по следующим snapshots и интервалу между ними.".into(),
                            ],
                        ),
                    );
                }
            }
        }
    }

    // XID and MultiXact have independent freeze horizons. Relations are
    // ordered by XID age, so searching only the first relation misses MXID.
    let oldest_mxid_relation = relations.data.as_ref().and_then(|rows| {
        rows.iter()
            .filter(|relation| {
                relation.mxid_age.is_some()
                    && relation
                        .effective_multixact_freeze_max_age
                        .is_some_and(|age| age > 0)
            })
            .max_by(|left, right| {
                let left_age = left.mxid_age.unwrap_or_default() as i128;
                let right_age = right.mxid_age.unwrap_or_default() as i128;
                let left_max = left.effective_multixact_freeze_max_age.unwrap_or(1) as i128;
                let right_max = right.effective_multixact_freeze_max_age.unwrap_or(1) as i128;
                (left_age * right_max).cmp(&(right_age * left_max))
            })
    });

    let database_mxid = overview.database_mxid_age.zip(
        overview
            .autovacuum_multixact_freeze_max_age
            .filter(|age| *age > 0),
    );
    if oldest_mxid_relation.is_some() || database_mxid.is_some() {
        let mut observed = Vec::new();
        let mut threshold_reached = false;
        let mut threshold_near = false;

        if let Some((age, max_age)) = database_mxid {
            threshold_reached |= age >= max_age;
            threshold_near |= (age as i128) * 10 >= (max_age as i128) * 9;
            observed.push(format!(
                "Database {}: MXID age = {}; global autovacuum_multixact_freeze_max_age = {}.",
                overview.database, age, max_age
            ));
        }
        if let Some(relation) = oldest_mxid_relation {
            let age = relation.mxid_age.unwrap_or_default();
            let max_age = relation.effective_multixact_freeze_max_age.unwrap_or(1);
            threshold_reached |= age >= max_age;
            threshold_near |= (age as i128) * 10 >= (max_age as i128) * 9;
            observed.push(format!(
                "Наибольшая доля MXID freeze max: {}.{} (OID {}), MXID age = {}; effective multixact freeze max = {}.",
                relation.schema_name, relation.relation_name, relation.relation_oid, age, max_age
            ));
        }

        items.push(interpretation_item(
            "multixact-freeze-horizon",
            if threshold_reached {
                "Достигнут configured MultiXact freeze threshold"
            } else if threshold_near {
                "MultiXact близок к configured freeze threshold (>=90%, эвристика pgXray)"
            } else {
                "MultiXact freeze horizon"
            },
            observed,
            if threshold_reached {
                vec![
                    "MXID age достиг настроенного порога: защитное обслуживание требует отдельной проверки, независимо от XID age.".into(),
                    "Один snapshot не доказывает неисправность или наступление wraparound.".into(),
                ]
            } else if threshold_near {
                vec![
                    "MXID age достиг как минимум 90% настроенного порога; это ранний сигнал pgXray для проверки защитного VACUUM, а не отдельный PostgreSQL threshold.".into(),
                    "Низкий XID age не исключает риска по MXID; требуется история наблюдений.".into(),
                ]
            } else {
                vec![
                    "MultiXact имеет собственный freeze horizon: низкий XID age не исключает риска по MXID.".into(),
                ]
            },
            vec![
                "Проверить VACUUM, relminmxid/datminmxid и динамику MXID age по следующим snapshots.".into(),
                "Не применять XID freeze threshold к MultiXact age.".into(),
            ],
        ));
    } else {
        limitations.push(
            "MXID age или действующий multixact freeze threshold недоступны: отдельная оценка MultiXact ограничена (в том числе для старых snapshots).".into(),
        );
    }

    let wraparound_vacuum = vacuums
        .data
        .as_ref()
        .and_then(|rows| rows.iter().find(|row| row.is_wraparound_vacuum));

    if let Some(vacuum) = wraparound_vacuum {
        items.push(
            interpretation_item(
                "anti-wraparound-vacuum",
                "Anti-wraparound VACUUM виден",
                vec![
                    format!(
                        "PID {} vacuum-ит {}.{} (OID {}), phase='{}', XID age={}.",
                        vacuum.pid,
                        vacuum.schema_name,
                        vacuum.relation_name,
                        vacuum.relation_oid,
                        vacuum.phase,
                        vacuum.xid_age,
                    ),
                ],
                vec![
                    "В текущем snapshot pgXray распознал защитный autovacuum worker по тексту активности PostgreSQL.".into(),
                    "Это подтверждает наличие freeze-driven VACUUM в данный момент, но не показывает будущую скорость снижения age.".into(),
                ],
                vec![
                    "Не запускать параллельный ручной VACUUM той же relation без отдельной причины.".into(),
                    "Сравнить XID age после завершения worker в следующем snapshot.".into(),
                ],
            ),
        );
    } else if let Some(relation) = oldest_relation {
        if relation.xid_age >= relation.effective_freeze_max_age {
            let unknown_detection = vacuums.data.as_ref().is_some_and(|rows| {
                rows.iter().any(|vacuum| {
                    !vacuum.is_wraparound_vacuum
                        && vacuum.wraparound_detection.as_deref() != Some("query_text")
                })
            });
            let (id, title, evidence, meaning, next_steps) = if vacuums.data.is_none() {
                (
                    "freeze-max-vacuum-unavailable",
                    "Freeze threshold достигнут, секция VACUUM недоступна",
                    "Текущий VACUUM не удалось проверить: данные секции отсутствуют.",
                    "Нет данных, чтобы утверждать отсутствие anti-wraparound worker.",
                    "Повторить сбор секции VACUUM и проверить права/ошибку сбора.",
                )
            } else if unknown_detection {
                (
                    "freeze-max-vacuum-unconfirmed",
                    "Freeze threshold достигнут, тип текущего VACUUM не подтверждён",
                    "wraparound_detection=unknown хотя бы у одного VACUUM: отсутствие защитного worker не подтверждено.",
                    "Среди доступных процессов может быть anti-wraparound VACUUM с недоступным текстом активности.",
                    "Проверить доступность текста pg_stat_activity и повторить snapshot.",
                )
            } else {
                (
                    "freeze-max-no-vacuum",
                    "Freeze threshold достигнут, текущий anti-wraparound VACUUM не виден",
                    "В доступной секции не обнаружен подтверждённый anti-wraparound VACUUM.",
                    "Один мгновенный snapshot не доказывает, что worker не запускался или не запустится.",
                    "Повторить snapshot и проверить XID age progression и состояние autovacuum workers.",
                )
            };
            items.push(interpretation_item(
                id,
                title,
                vec![
                    format!(
                        "{}.{}: XID age {} >= effective freeze max {}.",
                        relation.schema_name,
                        relation.relation_name,
                        relation.xid_age,
                        relation.effective_freeze_max_age,
                    ),
                    evidence.into(),
                ],
                vec![meaning.into()],
                vec![
                    next_steps.into(),
                    "Проверить blockers и состояние autovacuum workers.".into(),
                ],
            ));
        }
    }

    // Heuristic for prioritising investigation, NOT a PostgreSQL safety
    // threshold. Tiny backend_xmin ages should not produce a blocker finding.
    let backend_min_age = oldest_relation
        .filter(|relation| relation.effective_freeze_max_age > 0)
        .map(|relation| relation.effective_freeze_max_age / 2);
    let backend_candidate = backend_min_age.and_then(|min_age| {
        backends.data.as_ref().and_then(|rows| {
            rows.iter()
                .filter(|backend| backend.xmin_age >= min_age)
                .max_by_key(|backend| backend.xmin_age)
        })
    });

    if let Some(backend) = backend_candidate {
        let mut observed = vec![format!(
            "PID {}: backend_xmin age = {}.",
            backend.pid, backend.xmin_age,
        )];

        if let Some(seconds) = backend.transaction_age_seconds {
            observed.push(format!("Transaction age = {} seconds.", seconds,));
        }
        if let Some(min_age) = backend_min_age {
            observed.push(format!(
                "Investigation heuristic: backend_xmin age >= 50% of oldest relation effective freeze max ({} XIDs).",
                min_age
            ));
        }

        items.push(
            interpretation_item(
                "backend-xmin-candidate",
                "Backend XMIN требует проверки",
                observed,
                vec![
                    "Существенный backend_xmin может ограничивать vacuum horizon, поэтому backend является кандидатом на blocker; порог 50% — эвристика pgXray, не порог безопасности PostgreSQL.".into(),
                    "Snapshot не доказывает, что именно он удерживает freeze horizon старейшей relation.".into(),
                ],
                vec![
                    "Определить владельца и назначение backend.".into(),
                    "Сопоставить xmin age, transaction age и динамику relation age между snapshots.".into(),
                    "Не завершать production backend только по возрасту XID.".into(),
                ],
            ),
        );
    } else if backend_min_age.is_none()
        && backends.data.as_ref().is_some_and(|rows| !rows.is_empty())
    {
        limitations.push(
            "Backend XMIN получен, но effective freeze threshold старейшей relation неизвестен: значимость backend не оценивалась.".into(),
        );
    }

    let slot_candidate = slots.data.as_ref().and_then(|rows| {
        rows.iter().max_by_key(|slot| {
            slot.xmin_age
                .unwrap_or(0)
                .max(slot.catalog_xmin_age.unwrap_or(0))
        })
    });

    if let Some(slot) = slot_candidate {
        items.push(
            interpretation_item(
                "replication-slot-candidate",
                "Replication slot требует проверки",
                vec![
                    format!(
                        "Slot '{}': type={}, active={}, xmin_age={}, catalog_xmin_age={}.",
                        slot.slot_name,
                        slot.slot_type,
                        slot.active,
                        optional_i64(
                            slot.xmin_age,
                        ),
                        optional_i64(
                            slot.catalog_xmin_age,
                        ),
                    ),
                ],
                vec![
                    "Slot с xmin/catalog_xmin может удерживать transaction horizon и поэтому является кандидатом на blocker.".into(),
                    "Наличие старого slot XMIN не означает автоматически, что slot можно удалять.".into(),
                ],
                vec![
                    "Понять назначение и owner slot.".into(),
                    "Сопоставить его XMIN age с relation/database age и последующими snapshots.".into(),
                ],
            ),
        );
    }

    if let Some(prepared) = prepared.data.as_ref() {
        if prepared.count > 0 {
            let mut observed = vec![format!(
                "Prepared transactions = {}; oldest age = {} seconds.",
                prepared.count,
                optional_i64(prepared.oldest_age_seconds),
            )];
            observed.extend(prepared.examples.iter().map(|example| {
                format!(
                    "Prepared GID='{}', XID={}, database={}, owner={}, prepared_at={}, age={} seconds.",
                    example.gid,
                    example.xid,
                    example.database,
                    example.owner,
                    example.prepared_at,
                    example.age_seconds,
                )
            }));
            items.push(
                interpretation_item(
                    "prepared-xact-candidate",
                    "Есть prepared transactions",
                    observed,
                    vec![
                        "Prepared transaction может удерживать transaction horizon и должна рассматриваться как кандидат при freeze investigation.".into(),
                        "Возраст здесь временной, а не XID age, поэтому напрямую сравнивать его с age(relfrozenxid) нельзя.".into(),
                    ],
                    vec![
                        "Определить owner и назначение prepared transaction.".into(),
                        "Не выполнять COMMIT PREPARED / ROLLBACK PREPARED без проверки business impact.".into(),
                    ],
                ),
            );
        }
    }

    let blockers_fully_available =
        backends.error.is_none() && slots.error.is_none() && prepared.error.is_none();

    let no_backend = backends.data.as_ref().is_some_and(Vec::is_empty);

    let no_slots = slots.data.as_ref().is_some_and(Vec::is_empty);

    let no_prepared = prepared
        .data
        .as_ref()
        .is_some_and(|summary| summary.count == 0);

    if blockers_fully_available && no_backend && no_slots && no_prepared {
        items.push(
            interpretation_item(
                "no-xmin-blockers-observed",
                "Явные XMIN blockers не обнаружены",
                vec![
                    "backend_xmin: 0; replication slots с xmin/catalog_xmin: 0; prepared transactions: 0.".into(),
                ],
                vec![
                    "В доступных системных views текущего snapshot явный blocker не найден.".into(),
                    "Это не доказывает отсутствие всех возможных причин медленного freeze: snapshot не включает PostgreSQL logs, I/O pressure и полный lock analysis.".into(),
                ],
                vec![
                    "Если XID age продолжает расти, сравнить следующий snapshot, current VACUUM и XID rate.".into(),
                ],
            ),
        );
    }

    push_section_limitation(&mut limitations, "Возраст relations", relations);

    push_section_limitation(&mut limitations, "Normal autovacuum triggers", triggers);

    push_section_limitation(&mut limitations, "Текущий VACUUM", vacuums);

    push_section_limitation(&mut limitations, "Backend XMIN", backends);

    push_section_limitation(&mut limitations, "Replication slots", slots);

    push_section_limitation(&mut limitations, "Prepared transactions", prepared);

    limitations.push(
        "Интерпретация построена по одному snapshot. Она не знает XID rate и не может подтвердить устойчивую тенденцию без истории измерений."
            .into(),
    );

    limitations.push(
        "Current VACUUM, backend state и wait events — мгновенное состояние; отсутствие объекта в одном snapshot не доказывает, что его не было до или после измерения."
            .into(),
    );

    XidInterpretation { items, limitations }
}

fn interpretation_item(
    id: &str,
    title: &str,
    observed: Vec<String>,
    meaning: Vec<String>,
    next_steps: Vec<String>,
) -> XidInterpretationItem {
    XidInterpretationItem {
        id: id.to_owned(),
        title: title.to_owned(),
        observed,
        meaning,
        next_steps,
    }
}

fn push_section_limitation<T>(limitations: &mut Vec<String>, title: &str, section: &XidSection<T>) {
    if let Some(error) = section.error.as_deref() {
        limitations.push(format!("{title}: секция недоступна — {error}",));
    }
}

fn workload_pattern(trigger: &XidAutovacuumTrigger) -> Option<(&'static str, String)> {
    if trigger.stats_reset.is_none() {
        return Some((
            "Накопительные DML-счётчики: статистическое окно неизвестно",
            "stats_reset неизвестен: нулевые или ненулевые cumulative counters не позволяют достоверно классифицировать период как static/cold, append-only или churn.".into(),
        ));
    }
    if trigger.tuples_inserted == 0 && trigger.tuples_updated == 0 && trigger.tuples_deleted == 0 {
        return Some((
            "Cumulative counters похожи на static/cold период",
            "В доступном статистическом окне INSERT/UPDATE/DELETE равны нулю; это согласуется со static/cold поведением."
                .into(),
        ));
    }

    if trigger.tuples_inserted > 0 && trigger.tuples_updated == 0 && trigger.tuples_deleted == 0 {
        return Some((
            "Cumulative counters похожи на append-only период",
            "В доступном статистическом окне есть INSERT, но нет UPDATE/DELETE; это согласуется с append-only поведением."
                .into(),
        ));
    }

    if trigger.tuples_updated > 0 || trigger.tuples_deleted > 0 {
        return Some((
            "Cumulative counters показывают churn",
            "В статистическом окне есть UPDATE и/или DELETE; normal dead-tuple trigger для такой relation требует отдельного внимания."
                .into(),
        ));
    }

    None
}

fn relation_kind_label(kind: &str) -> &'static str {
    match kind {
        "r" => "table",
        "m" => "materialized view",
        "t" => "TOAST",
        _ => "relation",
    }
}

fn optional_i64(value: Option<i64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "NULL".into())
}

fn optional_bool(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "unknown",
    }
}
