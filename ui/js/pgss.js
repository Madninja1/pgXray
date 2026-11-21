"use strict";

let pgssCurrentSnapshotId = null;

function renderPgssConnectionOptions(connections) {
  const select = byId("pgss-connection");
  const previous = select.value;

  select.replaceChildren();

  if (connections.length === 0) {
    const option = document.createElement("option");

    option.value = "";
    option.textContent = "Нет сохранённых подключений";

    select.append(option);
    select.disabled = true;
    byId("collect-pgss").disabled = true;

    return;
  }

  select.disabled = false;
  byId("collect-pgss").disabled = false;

  for (const connection of connections) {
    const option = document.createElement("option");

    option.value = String(connection.id);
    option.textContent =
        `${connection.name} — ${connection.username}@${connection.host}:${connection.port}/${connection.database}`;

    select.append(option);
  }

  if (
      previous
      && connections.some(
          (connection) => String(connection.id) === previous,
      )
  ) {
    select.value = previous;
  }
}

function setPgssTab(name) {
  document.querySelectorAll(".pgss-tab").forEach((button) => {
    const active = button.dataset.pgssTab === name;

    button.classList.toggle("active", active);
    button.setAttribute("aria-selected", String(active));
  });

  for (const candidate of [
    "health",
    "cardinality",
    "performance",
    "statement",
    "methodology",
    "queries",
  ]) {
    byId(`pgss-${candidate}-tab`).hidden = candidate !== name;
  }
}

function renderPgssSectionError(id, section) {
  const element = byId(id);
  const error = section?.error;

  element.hidden = !error;
  element.textContent = error
      ? `Раздел недоступен: ${error}`
      : "";
}

function renderPgssInterpretation(interpretation) {
  const list = byId("pgss-interpretation-list");
  const limitationsDetails = byId(
      "pgss-interpretation-limitations",
  );
  const limitationsList = byId(
      "pgss-interpretation-limitations-list",
  );

  list.replaceChildren();
  limitationsList.replaceChildren();

  if (!interpretation) {
    const empty = document.createElement("p");

    empty.className = "empty-state";
    empty.textContent = "Интерпретация отсутствует.";

    list.append(empty);
    limitationsDetails.hidden = true;

    return;
  }

  for (const item of interpretation.items ?? []) {
    const card = document.createElement("article");
    const title = document.createElement("h4");

    card.className = "interpretation-card";
    title.textContent = item.title;
    card.append(title);

    for (const [label, values] of [
      ["Что обнаружено", item.observed ?? []],
      ["Что это может означать", item.meaning ?? []],
      ["Что проверить следующим", item.next_steps ?? []],
    ]) {
      const group = document.createElement("div");
      const heading = document.createElement("strong");
      const listElement = document.createElement("ul");

      group.className = "interpretation-group";
      heading.textContent = label;

      for (const value of values) {
        const row = document.createElement("li");

        row.textContent = value;
        listElement.append(row);
      }

      group.append(heading, listElement);
      card.append(group);
    }

    list.append(card);
  }

  const limitations = interpretation.limitations ?? [];

  limitationsDetails.hidden = limitations.length === 0;

  for (const value of limitations) {
    const row = document.createElement("li");

    row.textContent = value;
    limitationsList.append(row);
  }
}

function renderPgssRoleDatabase(section) {
  renderPgssSectionError(
      "pgss-role-database-error",
      section,
  );

  const body = byId("pgss-role-database-list");
  const rows = section?.data ?? [];

  body.replaceChildren();

  if (rows.length === 0) {
    tableEmpty(body, 6);
    return;
  }

  for (const item of rows) {
    const row = document.createElement("tr");

    appendCells(
        row,
        [
          `${item.role_name ?? "<unknown>"} / ${item.database ?? "<unknown>"}`,
          formatInteger(item.entries),
          formatPercent(item.entries_pct),
          `${formatInteger(item.one_call_entries)} (${formatPercent(item.one_call_pct)})`,
          `${formatInteger(item.up_to_5_calls_entries)} (${formatPercent(item.up_to_5_calls_pct)})`,
          formatInteger(item.total_calls),
        ],
    );

    body.append(row);
  }
}

function renderPgssCommands(section) {
  renderPgssSectionError(
      "pgss-commands-error",
      section,
  );

  const body = byId("pgss-command-list");
  const rows = section?.data ?? [];

  body.replaceChildren();

  if (rows.length === 0) {
    tableEmpty(body, 7);
    return;
  }

  for (const item of rows) {
    const row = document.createElement("tr");

    appendCells(
        row,
        [
          item.command,
          `${item.role_name ?? "<unknown>"} / ${item.database ?? "<unknown>"}`,
          formatInteger(item.entries),
          formatPercent(item.entries_pct),
          `${formatInteger(item.one_call_entries)} (${formatPercent(item.one_call_pct)})`,
          `${formatInteger(item.up_to_5_calls_entries)} (${formatPercent(item.up_to_5_calls_pct)})`,
          formatInteger(item.total_calls),
        ],
    );

    body.append(row);
  }
}

function renderPgssCallTargets(section) {
  renderPgssSectionError(
      "pgss-call-targets-error",
      section,
  );

  const body = byId("pgss-call-target-list");
  const rows = section?.data ?? [];

  body.replaceChildren();

  if (rows.length === 0) {
    tableEmpty(
        body,
        7,
        "CALL entries не найдены или query text недоступен текущей роли.",
    );

    return;
  }

  for (const item of rows) {
    const row = document.createElement("tr");

    appendCells(
        row,
        [
          item.call_tag.slice(0, 12),
          `${item.role_name ?? "<unknown>"} / ${item.database ?? "<unknown>"}`,
          formatInteger(item.entries),
          formatPercent(item.entries_pct_of_max),
          formatInteger(item.one_call_entries),
          formatPercent(item.one_call_pct),
          formatInteger(item.total_calls),
        ],
    );

    body.append(row);
  }
}

function pgssPerformanceRows() {
  if (!pgssPerformanceSamples) {
    return [];
  }

  return pgssPerformanceSamples[
      byId("pgss-performance-kind").value
      ] ?? [];
}

function renderPgssPerformance() {
  const body = byId("pgss-performance-list");
  const rows = pgssPerformanceRows();

  body.replaceChildren();

  if (rows.length === 0) {
    tableEmpty(
        body,
        12,
        "Для выбранного ranking нет данных.",
    );

    return;
  }

  for (const item of rows) {
    const row = document.createElement("tr");
    const queryidCell = document.createElement("td");
    const identityCell = document.createElement("td");
    const callsCell = document.createElement("td");
    const totalCell = document.createElement("td");
    const meanCell = document.createElement("td");
    const stddevCell = document.createElement("td");
    const rowsCell = document.createElement("td");
    const tempCell = document.createElement("td");
    const sharedCell = document.createElement("td");
    const walCell = document.createElement("td");
    const planCell = document.createElement("td");
    const actionCell = document.createElement("td");
    const detail = document.createElement("button");

    queryidCell.textContent = item.queryid ?? "не виден";
    identityCell.textContent =
        `${item.role_name ?? "<unknown>"} / ${item.database ?? "<unknown>"}`;

    callsCell.textContent = formatInteger(item.calls);
    totalCell.textContent = `${formatNumber(item.total_exec_time_ms, 1)} ms`;
    meanCell.textContent =
        `${formatNumber(item.mean_exec_time_ms, 3)} / ${formatNumber(item.max_exec_time_ms, 3)} ms`;

    stddevCell.textContent =
        `${formatNumber(item.stddev_exec_time_ms, 3)} ms`;

    rowsCell.textContent = formatInteger(item.rows);
    tempCell.textContent =
        `${formatInteger(item.temp_blks_written)} / ${formatInteger(item.temp_blks_read)}`;

    sharedCell.textContent =
        `${formatInteger(item.shared_blks_read)} / ${formatInteger(item.shared_blks_hit)}`;

    walCell.textContent = formatBytes(item.wal_bytes);
    planCell.textContent =
        `${formatNumber(item.total_plan_time_ms, 1)} / ${formatNumber(item.mean_plan_time_ms, 3)} ms`;

    detail.type = "button";
    detail.className = "secondary pgss-detail-button";
    detail.textContent = "Разобрать";
    detail.disabled = !item.queryid;

    if (item.queryid) {
      detail.dataset.pgssQueryid = item.queryid;
      detail.dataset.pgssUserid = String(item.userid);
      detail.dataset.pgssDbid = String(item.dbid);
      detail.dataset.pgssToplevel = String(item.toplevel);
    }

    actionCell.append(detail);

    row.append(
        queryidCell,
        identityCell,
        callsCell,
        totalCell,
        meanCell,
        stddevCell,
        rowsCell,
        tempCell,
        sharedCell,
        walCell,
        planCell,
        actionCell,
    );

    body.append(row);
  }
}

function renderPgssQueryDetail(detail) {
  byId("pgss-query-detail-error").hidden = true;
  byId("pgss-query-detail").hidden = false;

  byId("pgss-query-detail-meta").textContent =
      `queryid=${detail.queryid} · ${detail.role_name ?? "<unknown>"} / ${detail.database ?? "<unknown>"} · ${detail.toplevel ? "top-level" : "nested"}`;

  byId("pgss-query-detail-sql").textContent =
      detail.query ?? "Query text недоступен текущей PostgreSQL-роли.";

  byId("pgss-detail-calls").textContent =
      formatInteger(detail.calls);

  byId("pgss-detail-exec").textContent =
      `${formatNumber(detail.total_exec_time_ms, 1)} / ${formatNumber(detail.mean_exec_time_ms, 3)} / ${formatNumber(detail.max_exec_time_ms, 3)} / ${formatNumber(detail.stddev_exec_time_ms, 3)}`;

  byId("pgss-detail-plan").textContent =
      `${formatNumber(detail.total_plan_time_ms, 1)} / ${formatNumber(detail.mean_plan_time_ms, 3)} / ${formatNumber(detail.stddev_plan_time_ms, 3)}`;

  byId("pgss-detail-rows").textContent =
      formatInteger(detail.rows);

  byId("pgss-detail-shared").textContent =
      `${formatInteger(detail.shared_blks_hit)} / ${formatInteger(detail.shared_blks_read)}`;

  byId("pgss-detail-temp").textContent =
      `${formatInteger(detail.temp_blks_read)} / ${formatInteger(detail.temp_blks_written)}`;

  byId("pgss-detail-wal").textContent =
      formatBytes(detail.wal_bytes);
}

function pgssStatementDeltaStateLabel(value) {
  return {
    baseline: "baseline",
    comparable: "confirmed delta",
    sample_gap: "sample gap",
    stats_reset_changed: "stats reset",
    dealloc_changed: "dealloc changed",
    dealloc_unknown: "dealloc unknown",
    counter_reset: "counter reset",
    invalid_interval: "invalid interval",
    unavailable: "unavailable",
  }[value] ?? value ?? "—";
}

function pgssStatementDeltaStateNote(value) {
  return {
    comparable:
        "Confirmed delta: соседние snapshots, тот же stats_reset, dealloc не изменился, cumulative counters не уменьшились.",
    baseline:
        "Это первая доступная sampled point; для interval delta нужна предыдущая соседняя точка.",
    sample_gap:
        "Предыдущий snapshot не содержал statement в bounded history sample. Это не доказывает отсутствие PGSS entry.",
    stats_reset_changed:
        "PGSS statistics period изменился или stats_reset недоступен; cumulative counters не вычитаются.",
    dealloc_changed:
        "Между snapshots изменился dealloc. Entry могла быть deallocated и появиться снова, поэтому delta не считается confirmed.",
    dealloc_unknown:
        "dealloc недоступен хотя бы для одной точки; continuity statement не подтверждена.",
    counter_reset:
        "Один из cumulative counters уменьшился; interval не используется для confirmed delta.",
    invalid_interval:
        "Snapshots не образуют положительный временной интервал.",
  }[value] ?? "";
}

function formatPgssSignedBytes(value) {
  if (value === null || value === undefined) return "—";
  const number = Number(value);
  if (number === 0) return "0 B";
  return `${number > 0 ? "+" : "−"}${formatBytes(Math.abs(number))}`;
}

function pgssStatementChangeStateLabel(value) {
  return {
    ready: "ready",
    insufficient_history: "need history",
    latest_interval_not_comparable: "latest unsafe",
    previous_interval_not_comparable: "previous unsafe",
    no_comparable_metrics: "no metrics",
  }[value] ?? value ?? "—";
}

function formatPgssChangeMetric(value, unit) {
  if (value === null || value === undefined) return "—";

  switch (unit) {
    case "per_hour":
      return `${formatNumber(value, 2)}/h`;
    case "ms_per_call":
      return `${formatNumber(value, 3)} ms/call`;
    case "ms_per_plan":
      return `${formatNumber(value, 3)} ms/plan`;
    case "percent":
      return formatPercent(value);
    case "blocks_per_call":
      return `${formatNumber(value, 3)} blocks/call`;
    case "bytes_per_call":
      return `${formatBytes(value)}/call`;
    case "rows_per_call":
      return `${formatNumber(value, 3)} rows/call`;
    default:
      return formatNumber(value, 3);
  }
}

function formatPgssChangeDelta(item) {
  if (item.direction === "appeared") return "appeared";
  if (item.direction === "disappeared") return "disappeared";
  if (item.direction === "unchanged") return "unchanged";

  if (item.relative_change_pct !== null && item.relative_change_pct !== undefined) {
    const value = Number(item.relative_change_pct);
    return `${value > 0 ? "+" : ""}${formatNumber(value, 1)}%`;
  }

  return item.direction ?? "—";
}

function pgssEvidenceLevelLabel(value) {
  return {
    confirmed_change: "Confirmed change",
    observed: "Observed",
  }[value] ?? value ?? "—";
}

function renderPgssStatementChangeAnalysis(analysis) {
  const status = byId("pgss-statement-change-status");
  const body = byId("pgss-statement-change-list");
  const interval = byId("pgss-statement-change-intervals");
  const limitationsDetails = byId("pgss-statement-change-limitations");
  const limitationsList = byId("pgss-statement-change-limitations-list");

  body.replaceChildren();
  limitationsList.replaceChildren();

  const state = analysis?.state ?? "insufficient_history";
  status.textContent = pgssStatementChangeStateLabel(state);
  status.dataset.state = state;

  if (state === "ready") {
    interval.textContent =
        `previous #${analysis.older_from_snapshot_id} → #${analysis.older_to_snapshot_id} · latest #${analysis.newer_from_snapshot_id} → #${analysis.newer_to_snapshot_id}`;
  } else {
    interval.textContent =
        "Change Analysis появится после двух последовательных safe intervals без reset/dealloc/sample gap.";
  }

  const items = analysis?.items ?? [];
  if (items.length === 0) {
    tableEmpty(body, 5, "Пока нет двух последовательных confirmed intervals для сравнения.");
  } else {
    for (const item of items) {
      const row = document.createElement("tr");
      appendCells(row, [
        item.label,
        formatPgssChangeMetric(item.older_value, item.unit),
        formatPgssChangeMetric(item.newer_value, item.unit),
        formatPgssChangeDelta(item),
        pgssEvidenceLevelLabel(item.evidence_level),
      ]);
      body.append(row);
    }
  }

  const limitations = analysis?.limitations ?? [];
  for (const text of limitations) {
    const item = document.createElement("li");
    item.textContent = text;
    limitationsList.append(item);
  }
  limitationsDetails.hidden = limitations.length === 0;
}

function resetPgssStatementInvestigation() {
  byId("pgss-statement-empty").hidden = false;
  byId("pgss-statement-content").hidden = true;
  byId("pgss-statement-history-error").hidden = true;
  byId("pgss-statement-history-error").textContent = "";
  byId("pgss-statement-history-list").replaceChildren();
  byId("pgss-statement-history-limitations-list").replaceChildren();
  byId("pgss-statement-history-limitations").hidden = true;
  byId("pgss-statement-change-list").replaceChildren();
  byId("pgss-statement-change-status").textContent = "—";
  byId("pgss-statement-change-intervals").textContent = "";
  byId("pgss-statement-change-limitations-list").replaceChildren();
  byId("pgss-statement-change-limitations").hidden = true;
  byId("pgss-query-detail").hidden = true;
  byId("pgss-query-detail-error").hidden = true;
  byId("pgss-query-detail-error").textContent = "";
  byId("pgss-query-detail-sql").textContent = "";
}

function renderPgssStatementHistory(history) {
  const summary = history.summary;
  const points = history.points ?? [];
  const latestState = summary.latest_delta_state;

  byId("pgss-statement-empty").hidden = true;
  byId("pgss-statement-content").hidden = false;
  byId("pgss-statement-meta").textContent =
      `queryid=${history.queryid} · ${history.role_name ?? "<unknown>"} / ${history.database ?? "<unknown>"} · ${history.toplevel ? "top-level" : "nested"} · anchor #${history.anchor_snapshot_id}`;

  byId("pgss-statement-history-span").textContent =
      `${formatInteger(summary.history_points)} points · ${formatInteger(summary.comparable_intervals)} safe${summary.history_span_seconds === null ? "" : ` · ${formatDuration(summary.history_span_seconds)}`}`;
  byId("pgss-statement-sample-coverage").textContent =
      history.anchor_current_entries === null || history.anchor_current_entries === undefined
          ? `${formatInteger(history.anchor_sampled_entries)} · ${history.anchor_sample_scope}`
          : `${formatInteger(history.anchor_sampled_entries)} / ${formatInteger(history.anchor_current_entries)}${history.anchor_sample_coverage_pct === null || history.anchor_sample_coverage_pct === undefined ? "" : ` · ${formatPercent(history.anchor_sample_coverage_pct)}`}`;
  byId("pgss-statement-latest-interval").textContent =
      summary.latest_elapsed_seconds === null
          ? pgssStatementDeltaStateLabel(latestState)
          : `${formatDuration(summary.latest_elapsed_seconds)} · ${pgssStatementDeltaStateLabel(latestState)}`;
  byId("pgss-statement-calls-delta").textContent =
      summary.calls_delta === null
          ? "—"
          : `${formatSignedInteger(summary.calls_delta)} · ${formatNumber(summary.calls_per_hour, 2)}/h`;
  byId("pgss-statement-exec-delta").textContent =
      summary.total_exec_time_delta_ms === null
          ? "—"
          : `+${formatNumber(summary.total_exec_time_delta_ms, 1)} ms · ${formatNumber(summary.interval_mean_exec_time_ms, 3)} ms/call`;
  byId("pgss-statement-reads-delta").textContent =
      formatSignedInteger(summary.shared_blks_read_delta);
  byId("pgss-statement-temp-delta").textContent =
      summary.temp_blks_written_delta === null && summary.temp_blks_read_delta === null
          ? "—"
          : `${formatSignedInteger(summary.temp_blks_written_delta)} / ${formatSignedInteger(summary.temp_blks_read_delta)} blocks`;
  byId("pgss-statement-wal-delta").textContent =
      formatPgssSignedBytes(summary.wal_bytes_delta);
  byId("pgss-statement-delta-state").textContent =
      pgssStatementDeltaStateLabel(latestState);
  byId("pgss-statement-delta-state").dataset.state = latestState ?? "unavailable";
  byId("pgss-statement-delta-note").textContent =
      pgssStatementDeltaStateNote(latestState);

  renderPgssStatementChangeAnalysis(history.change_analysis);

  const body = byId("pgss-statement-history-list");
  body.replaceChildren();
  if (points.length === 0) {
    tableEmpty(body, 11, "Сохранённых sampled points для statement нет.");
  } else {
    for (const point of [...points].reverse()) {
      const row = document.createElement("tr");
      appendCells(row, [
        `#${point.snapshot_id}`,
        formatSnapshotTime(point.collection_started_at_ms),
        pgssStatementDeltaStateLabel(point.delta_state),
        formatInteger(point.row.calls),
        `${formatNumber(point.row.total_exec_time_ms, 1)} ms`,
        `${formatNumber(point.row.mean_exec_time_ms, 3)} ms`,
        formatSignedInteger(point.calls_delta),
        point.interval_mean_exec_time_ms === null
            ? "—"
            : `${formatNumber(point.interval_mean_exec_time_ms, 3)} ms`,
        formatSignedInteger(point.shared_blks_read_delta),
        point.temp_blks_written_delta === null && point.temp_blks_read_delta === null
            ? "—"
            : `${formatSignedInteger(point.temp_blks_written_delta)} / ${formatSignedInteger(point.temp_blks_read_delta)}`,
        formatPgssSignedBytes(point.wal_bytes_delta),
      ]);
      body.append(row);
    }
  }

  const limitations = history.limitations ?? [];
  const limitationsList = byId("pgss-statement-history-limitations-list");
  limitationsList.replaceChildren();
  for (const text of limitations) {
    const item = document.createElement("li");
    item.textContent = text;
    limitationsList.append(item);
  }
  byId("pgss-statement-history-limitations").hidden = limitations.length === 0;
  byId("pgss-statement-history-error").hidden = true;
}

function renderPgssQueryDetailError(errorText) {
  byId("pgss-query-detail").hidden = false;
  byId("pgss-query-detail-error").hidden = false;
  byId("pgss-query-detail-error").textContent =
      `Live drill-down недоступен: ${errorText}`;
  byId("pgss-query-detail-sql").textContent = "";
}

async function loadPgssStatementInvestigation(identity) {
  setPgssTab("statement");
  resetPgssStatementInvestigation();
  byId("pgss-statement-empty").hidden = true;
  byId("pgss-statement-content").hidden = false;
  byId("pgss-statement-meta").textContent =
      `queryid=${identity.queryid} · загрузка saved history и live detail…`;

  if (!Number.isInteger(pgssCurrentSnapshotId) || pgssCurrentSnapshotId <= 0) {
    const error = byId("pgss-statement-history-error");
    error.hidden = false;
    error.textContent =
        "Statement history требует сохранённый anchor snapshot. Соберите новый PGSS snapshot.";
    return;
  }

  const connectionId = Number(byId("pgss-connection").value);
  const historyPromise = invoke("pgss_statement_history", {
    snapshotId: pgssCurrentSnapshotId,
    userid: identity.userid,
    dbid: identity.dbid,
    queryid: identity.queryid,
    toplevel: identity.toplevel,
    limit: 100,
  })
      .then((history) => renderPgssStatementHistory(history))
      .catch((errorText) => {
        const error = byId("pgss-statement-history-error");
        error.hidden = false;
        error.textContent = `Saved statement history недоступна: ${String(errorText)}`;
      });

  byId("pgss-query-detail").hidden = false;
  byId("pgss-query-detail-error").hidden = true;
  byId("pgss-query-detail-sql").textContent = "Загрузка live SQL…";

  const detailPromise = invoke("pgss_query_detail", {
    connectionId,
    userid: identity.userid,
    dbid: identity.dbid,
    queryid: identity.queryid,
    toplevel: identity.toplevel,
  })
      .then((detail) => renderPgssQueryDetail(detail))
      .catch((errorText) => renderPgssQueryDetailError(String(errorText)));

  await Promise.all([historyPromise, detailPromise]);
}

function renderPgssSnapshot(snapshot, history = null) {
  const overview = snapshot.overview;
  const stats = snapshot.stats?.data;
  const capacity = snapshot.capacity?.data;

  pgssServerVersionNum = overview.server_version_num;
  pgssPerformanceSamples = snapshot.performance?.data ?? null;
  pgssCurrentSnapshotId = history?.id ?? null;
  resetPgssStatementInvestigation();

  byId("pgss-extension-state").textContent =
      overview.extension_installed
          ? (overview.preloaded ? "installed · preloaded" : "installed · preload?")
          : "not installed";

  byId("pgss-dealloc").textContent =
      stats ? formatInteger(stats.dealloc) : "—";

  byId("pgss-entries").textContent =
      capacity
          ? `${formatInteger(capacity.current_entries)} / ${formatInteger(capacity.max_entries)}`
          : "—";

  byId("pgss-fill").textContent =
      capacity ? formatPercent(capacity.fill_pct) : "—";

  byId("pgss-one-call").textContent =
      capacity
          ? `${formatInteger(capacity.one_call_entries)} (${formatPercent(capacity.one_call_pct)})`
          : "—";

  byId("pgss-stats-reset").textContent =
      stats ? formatTimestamp(stats.stats_reset) : "—";

  byId("pgss-snapshot-meta").textContent =
      `Снимок: ${formatTimestamp(overview.measured_at)} · ${overview.database} (OID ${overview.database_oid}) · PostgreSQL ${overview.server_version}.`;

  byId("pgss-extension-version").textContent =
      overview.extension_version ?? "—";

  byId("pgss-preload").textContent =
      `${overview.preloaded ? "pg_stat_statements найден" : "не найден"} · ${overview.shared_preload_libraries || "<empty>"}`;

  byId("pgss-compute-query-id").textContent =
      overview.compute_query_id ?? "—";

  byId("pgss-max-entries").textContent =
      formatInteger(overview.max_entries);

  byId("pgss-track").textContent =
      overview.track ?? "—";

  byId("pgss-track-utility").textContent =
      overview.track_utility === null
          ? "—"
          : yesNo(overview.track_utility);

  byId("pgss-track-planning").textContent =
      overview.track_planning === null
          ? "—"
          : yesNo(overview.track_planning);

  byId("pgss-save").textContent =
      overview.save === null
          ? "—"
          : yesNo(overview.save);

  byId("pgss-track-io-timing").textContent =
      yesNo(overview.track_io_timing);

  renderPgssSectionError(
      "pgss-stats-error",
      snapshot.stats,
  );

  renderPgssSectionError(
      "pgss-capacity-error",
      snapshot.capacity,
  );

  renderPgssInterpretation(
      snapshot.interpretation,
  );

  renderPgssRoleDatabase(
      snapshot.role_database,
  );

  renderPgssCommands(
      snapshot.commands,
  );

  renderPgssCallTargets(
      snapshot.call_targets,
  );

  renderPgssSectionError(
      "pgss-performance-error",
      snapshot.performance,
  );

  renderPgssPerformance();

  byId("pgss-health-empty").hidden = true;
  byId("pgss-health-content").hidden = false;
  byId("pgss-cardinality-empty").hidden = true;
  byId("pgss-cardinality-content").hidden = false;
  byId("pgss-performance-empty").hidden = true;
  byId("pgss-performance-content").hidden = false;

  renderPgssSources();
}

function resetPgssSnapshot() {
  pgssServerVersionNum = null;
  pgssPerformanceSamples = null;
  pgssCurrentSnapshotId = null;

  byId("pgss-health-empty").hidden = false;
  byId("pgss-health-content").hidden = true;
  byId("pgss-cardinality-empty").hidden = false;
  byId("pgss-cardinality-content").hidden = true;
  byId("pgss-performance-empty").hidden = false;
  byId("pgss-performance-content").hidden = true;
  resetPgssStatementInvestigation();

  renderPgssSources();
}

function pgssDocsVersion() {
  if (!pgssServerVersionNum) {
    return "current";
  }

  const major = Math.floor(
      Number(pgssServerVersionNum) / 10_000,
  );

  return major >= 10
      ? String(major)
      : "current";
}

function pgssDocsUrl(path) {
  return `https://www.postgresql.org/docs/${pgssDocsVersion()}/${path}`;
}

function renderPgssSources() {
  const list = byId("pgss-source-list");
  const versionElement = byId("pgss-source-version");

  list.replaceChildren();

  if (!pgssReference) {
    versionElement.textContent = "Справка ещё не загружена.";
    return;
  }

  const version = pgssDocsVersion();

  versionElement.textContent = version === "current"
      ? "До первого snapshot ссылки построены на current-документацию PostgreSQL."
      : `Ссылки построены для PostgreSQL ${version}.`;

  for (const source of pgssReference.sources ?? []) {
    const card = document.createElement("article");
    const title = document.createElement("strong");
    const purpose = document.createElement("p");
    const url = document.createElement("code");

    card.className = "source-card";
    title.textContent = source.title;
    purpose.textContent = source.purpose;
    url.className = "source-url";
    url.textContent = pgssDocsUrl(source.path);

    card.append(title, purpose, url);
    list.append(card);
  }
}

function renderPgssMethodology() {
  const list = byId("pgss-methodology-list");

  list.replaceChildren();

  if (!pgssReference) {
    const empty = document.createElement("p");

    empty.className = "empty-state";
    empty.textContent = "Методика недоступна.";

    list.append(empty);
    return;
  }

  pgssReference.methodology.forEach((step, index) => {
    const card = document.createElement("article");
    const heading = document.createElement("div");
    const number = document.createElement("span");
    const title = document.createElement("h3");
    const purpose = document.createElement("p");
    const items = document.createElement("ul");

    card.className = "reference-card";
    heading.className = "reference-heading";
    number.className = "reference-index";

    number.textContent = String(index + 1);
    title.textContent = step.title;
    purpose.textContent = step.purpose;

    heading.append(number, title);

    for (const text of step.how_to_read) {
      const item = document.createElement("li");

      item.textContent = text;
      items.append(item);
    }

    card.append(heading, purpose, items);
    list.append(card);
  });
}

function renderPgssQueries() {
  const list = byId("pgss-query-list");

  list.replaceChildren();

  if (!pgssReference) {
    const empty = document.createElement("p");

    empty.className = "empty-state";
    empty.textContent = "SQL reference недоступен.";

    list.append(empty);
    return;
  }

  for (const query of pgssReference.queries) {
    const details = document.createElement("details");
    const summary = document.createElement("summary");
    const purpose = document.createElement("p");
    const outputTitle = document.createElement("strong");
    const output = document.createElement("ul");
    const readTitle = document.createElement("strong");
    const read = document.createElement("ul");
    const sqlTitle = document.createElement("strong");
    const pre = document.createElement("pre");
    const code = document.createElement("code");

    details.className = "query-reference";
    summary.textContent = query.title;
    purpose.textContent = query.purpose;
    outputTitle.textContent = "На выходе";
    readTitle.textContent = "Как читать";
    sqlTitle.textContent = "SQL";

    for (const value of query.output) {
      const item = document.createElement("li");

      item.textContent = value;
      output.append(item);
    }

    for (const value of query.how_to_read) {
      const item = document.createElement("li");

      item.textContent = value;
      read.append(item);
    }

    code.textContent = query.sql;
    pre.className = "sql-block";
    pre.append(code);

    details.append(
        summary,
        purpose,
        outputTitle,
        output,
        readTitle,
        read,
        sqlTitle,
        pre,
    );

    list.append(details);
  }
}

function renderPgssReference() {
  renderPgssMethodology();
  renderPgssQueries();
  renderPgssSources();
}

async function loadPgssReference() {
  if (pgssReference) {
    renderPgssReference();
    return pgssReference;
  }

  if (pgssReferencePromise) {
    return pgssReferencePromise;
  }

  pgssReferencePromise = invoke("pgss_reference")
      .then((reference) => {
        pgssReference = reference;
        renderPgssReference();

        return reference;
      })
      .catch((error) => {
        const text =
            `Не удалось загрузить PGSS reference: ${String(error)}`;

        for (const id of [
          "pgss-methodology-list",
          "pgss-query-list",
        ]) {
          const target = byId(id);

          target.replaceChildren();

          const status = document.createElement("p");

          status.className = "section-error";
          status.textContent = text;

          target.append(status);
        }

        return null;
      })
      .finally(() => {
        pgssReferencePromise = null;
      });

  return pgssReferencePromise;
}

function pgssHistoryConnectionLabel(connectionId, historyRows) {
  const current = connectionsById.get(connectionId);

  if (current) {
    return `${current.name} — ${current.username}@${current.host}:${current.port}/${current.database}`;
  }

  const latest = historyRows.find(
      (row) => row.connection_id === connectionId,
  );

  if (!latest) {
    return `Подключение #${connectionId}`;
  }

  return `${latest.connection_name} — ${latest.username}@${latest.host}:${latest.port}/${latest.database} · сохранённая история`;
}

function renderPgssHistoryConnectionOptions(historyRows) {
  const select = byId("pgss-history-connection");
  const previous = select.value;

  const ids = new Set(
      Array.from(connectionsById.keys()),
  );

  for (const row of historyRows) {
    ids.add(row.connection_id);
  }

  select.replaceChildren();

  const sorted = Array.from(ids).sort((left, right) => {
    return pgssHistoryConnectionLabel(left, historyRows)
        .localeCompare(
            pgssHistoryConnectionLabel(right, historyRows),
            "ru",
        );
  });

  if (sorted.length === 0) {
    const option = document.createElement("option");

    option.value = "";
    option.textContent = "Нет подключений и PGSS history";

    select.append(option);
    select.disabled = true;
    byId("refresh-pgss-history").disabled = true;

    return;
  }

  select.disabled = false;
  byId("refresh-pgss-history").disabled = false;

  for (const connectionId of sorted) {
    const option = document.createElement("option");

    option.value = String(connectionId);
    option.textContent =
        pgssHistoryConnectionLabel(connectionId, historyRows);

    select.append(option);
  }

  if (
      previous
      && sorted.some((id) => String(id) === previous)
  ) {
    select.value = previous;
  }
}

function resetPgssComparison() {
  byId("pgss-comparison-content").hidden = true;
  byId("pgss-comparison-error").hidden = true;
  byId("pgss-comparison-error").textContent = "";
}

function renderPgssHistoryComparisonOptions(snapshots) {
  const first = byId("pgss-compare-first");
  const second = byId("pgss-compare-second");

  first.replaceChildren();
  second.replaceChildren();

  for (const snapshot of snapshots) {
    const text =
        `#${snapshot.id} · ${formatSnapshotTime(snapshot.collection_started_at_ms)} · dealloc ${formatInteger(snapshot.dealloc)}`;

    for (const select of [first, second]) {
      const option = document.createElement("option");

      option.value = String(snapshot.id);
      option.textContent = text;

      select.append(option);
    }
  }

  const canCompare = snapshots.length >= 2;

  first.disabled = !canCompare;
  second.disabled = !canCompare;
  byId("compare-pgss").disabled = !canCompare;

  if (canCompare) {
    first.value = String(snapshots[1].id);
    second.value = String(snapshots[0].id);
  }
}

function renderPgssHistory(snapshots) {
  pgssHistorySnapshots = snapshots;

  const empty = byId("pgss-history-empty");
  const content = byId("pgss-history-content");
  const body = byId("pgss-history-list");

  body.replaceChildren();

  const hasSnapshots = snapshots.length > 0;

  empty.hidden = hasSnapshots;
  content.hidden = !hasSnapshots;

  resetPgssComparison();
  renderPgssHistoryComparisonOptions(snapshots);

  if (!hasSnapshots) {
    return;
  }

  for (const snapshot of snapshots) {
    const row = document.createElement("tr");

    appendCells(
        row,
        [
          `#${snapshot.id}`,
          formatSnapshotTime(snapshot.collection_started_at_ms),
          `${snapshot.host}:${snapshot.port}/${snapshot.database} · db oid ${snapshot.database_oid} · PG ${snapshot.server_version}`,
          formatInteger(snapshot.dealloc),
          snapshot.current_entries === null
              ? "—"
              : `${formatInteger(snapshot.current_entries)} / ${formatInteger(snapshot.max_entries)}`,
          formatPercent(snapshot.fill_pct),
          formatInteger(snapshot.one_call_entries),
          formatTimestamp(snapshot.stats_reset),
          formatInteger(snapshot.unavailable_sections),
        ],
    );

    body.append(row);
  }
}

async function refreshPgssHistory() {
  const index = await invoke(
      "list_pgss_snapshots",
      {
        connectionId: null,
        limit: 500,
      },
  );

  renderPgssHistoryConnectionOptions(index);

  const connectionId = Number(
      byId("pgss-history-connection").value,
  );

  if (!Number.isInteger(connectionId) || connectionId <= 0) {
    renderPgssHistory([]);
    return;
  }

  const snapshots = await invoke(
      "list_pgss_snapshots",
      {
        connectionId,
        limit: 100,
      },
  );

  renderPgssHistory(snapshots);
}

function renderPgssDeltaTable(
    bodyId,
    rows,
    kind,
) {
  const body = byId(bodyId);

  body.replaceChildren();

  if (!rows || rows.length === 0) {
    tableEmpty(
        body,
        kind === "role" ? 8 : 7,
        "Нет comparable rows.",
    );

    return;
  }

  for (const item of rows) {
    const row = document.createElement("tr");
    const identity =
        `${item.role_name ?? "<unknown>"} / ${item.database ?? "<unknown>"}`;

    if (kind === "role") {
      appendCells(
          row,
          [
            identity,
            formatInteger(item.older_entries),
            formatInteger(item.newer_entries),
            formatSignedInteger(item.entries_delta),
            formatInteger(item.older_one_call_entries),
            formatInteger(item.newer_one_call_entries),
            formatSignedInteger(item.one_call_delta),
            relationSampleStateLabel(item.sample_state),
          ],
      );
    } else {
      appendCells(
          row,
          [
            kind === "command" ? item.command : item.call_tag.slice(0, 12),
            identity,
            formatInteger(item.older_entries),
            formatInteger(item.newer_entries),
            formatSignedInteger(item.entries_delta),
            formatSignedInteger(item.one_call_delta),
            relationSampleStateLabel(item.sample_state),
          ],
      );
    }

    body.append(row);
  }
}

function renderPgssComparisonInterpretation(interpretation) {
  const list = byId("pgss-comparison-interpretation-list");

  list.replaceChildren();

  const items = interpretation?.items ?? [];

  if (items.length === 0) {
    const empty = document.createElement("p");

    empty.className = "empty-state";
    empty.textContent =
        "Для этого интервала нет отдельной автоматической интерпретации.";

    list.append(empty);
    return;
  }

  for (const item of items) {
    const card = document.createElement("article");
    const title = document.createElement("h4");

    card.className = "interpretation-card";
    title.textContent = item.title;

    card.append(title);

    for (const [label, values] of [
      ["Что обнаружено", item.observed ?? []],
      ["Что это может означать", item.meaning ?? []],
      ["Что проверить следующим", item.next_steps ?? []],
    ]) {
      const group = document.createElement("div");
      const heading = document.createElement("strong");
      const listElement = document.createElement("ul");

      group.className = "interpretation-group";
      heading.textContent = label;

      for (const value of values) {
        const row = document.createElement("li");

        row.textContent = value;
        listElement.append(row);
      }

      group.append(
          heading,
          listElement,
      );

      card.append(group);
    }

    list.append(card);
  }
}

function renderPgssComparison(comparison) {
  byId("pgss-compare-interval").textContent =
      formatDuration(comparison.elapsed_seconds);

  byId("pgss-compare-stats-period").textContent =
      comparison.stats_period_continuous
          ? "непрерывный"
          : "reset/нет данных";

  byId("pgss-compare-dealloc").textContent =
      formatSignedInteger(comparison.dealloc_delta);

  byId("pgss-compare-dealloc-rate").textContent =
      comparison.dealloc_per_hour === null
          ? "не рассчитан"
          : formatNumber(comparison.dealloc_per_hour, 2);

  byId("pgss-compare-entries").textContent =
      formatSignedInteger(comparison.current_entries_delta);

  byId("pgss-compare-fill").textContent =
      comparison.fill_pct_delta === null
          ? "—"
          : `${comparison.fill_pct_delta > 0 ? "+" : ""}${formatNumber(comparison.fill_pct_delta, 1)} п.п.`;

  byId("pgss-compare-one-call").textContent =
      formatSignedInteger(comparison.one_call_entries_delta);

  byId("pgss-compare-up-to-5").textContent =
      formatSignedInteger(comparison.up_to_5_calls_entries_delta);

  byId("pgss-compare-meta").textContent =
      `#${comparison.older.id} (${formatSnapshotTime(comparison.older.collection_started_at_ms)}) → #${comparison.newer.id} (${formatSnapshotTime(comparison.newer.collection_started_at_ms)}) · ${comparison.newer.host}:${comparison.newer.port}/${comparison.newer.database}`;

  renderPgssComparisonInterpretation(
      comparison.interpretation,
  );

  renderPgssDeltaTable(
      "pgss-compare-role-list",
      comparison.role_database_changes,
      "role",
  );

  renderPgssDeltaTable(
      "pgss-compare-command-list",
      comparison.command_changes,
      "command",
  );

  renderPgssDeltaTable(
      "pgss-compare-call-list",
      comparison.call_target_changes,
      "call",
  );

  const limitations = byId("pgss-compare-limitations");

  limitations.replaceChildren();

  for (const value of comparison.limitations ?? []) {
    const row = document.createElement("li");

    row.textContent = value;
    limitations.append(row);
  }

  byId("pgss-comparison-error").hidden = true;
  byId("pgss-comparison-content").hidden = false;
}

function initPgssUi() {
  byId("collect-pgss").addEventListener("click", () => {
    const connectionId = Number(
        byId("pgss-connection").value,
    );

    if (!Number.isInteger(connectionId) || connectionId <= 0) {
      message(
          "Сначала сохраните и выберите подключение PostgreSQL.",
          true,
      );

      return;
    }

    void action(async () => {
      const result = await invoke(
          "collect_pgss_snapshot",
          {
            connectionId,
          },
      );

      renderPgssSnapshot(result.snapshot, result.history);

      message(
          `PGSS snapshot #${result.history.id} собран и сохранён.`,
      );
    });
  });

  byId("pgss-performance-kind").addEventListener("change", () => {
    byId("pgss-query-detail").hidden = true;
    renderPgssPerformance();
  });

  byId("pgss-performance-list").addEventListener("click", (event) => {
    const button = event.target.closest(
        "button[data-pgss-queryid]",
    );

    if (!button) {
      return;
    }

    const identity = {
      userid: Number(button.dataset.pgssUserid),
      dbid: Number(button.dataset.pgssDbid),
      queryid: button.dataset.pgssQueryid,
      toplevel: button.dataset.pgssToplevel === "true",
    };

    void action(
        async () => loadPgssStatementInvestigation(identity),
        {startMessage: null},
    );
  });

  byId("close-pgss-query-detail").addEventListener("click", () => {
    setPgssTab("performance");
  });

  byId("refresh-pgss-history").addEventListener("click", () => {
    void action(async () => {
      await refreshPgssHistory();

      message("История pg_stat_statements обновлена.");
    });
  });

  byId("pgss-history-connection").addEventListener("change", () => {
    void action(async () => {
      await refreshPgssHistory();

      message("История pg_stat_statements загружена.");
    });
  });

  byId("compare-pgss").addEventListener("click", () => {
    const firstId = Number(
        byId("pgss-compare-first").value,
    );

    const secondId = Number(
        byId("pgss-compare-second").value,
    );

    if (
        !Number.isInteger(firstId)
        || !Number.isInteger(secondId)
        || firstId <= 0
        || secondId <= 0
    ) {
      return;
    }

    if (firstId === secondId) {
      const error = byId("pgss-comparison-error");

      error.hidden = false;
      error.textContent = "Выберите два разных snapshots.";

      return;
    }

    resetPgssComparison();

    void action(
        async () => {
          const comparison = await invoke(
              "compare_pgss_snapshots",
              {
                firstId,
                secondId,
              },
          );

          renderPgssComparison(comparison);
        },
        {
          startMessage: null,
          onError: (errorText) => {
            const error = byId("pgss-comparison-error");

            error.hidden = false;
            error.textContent =
                `Не удалось сравнить PGSS snapshots: ${errorText}`;
          },
        },
    );
  });
  document.querySelectorAll(".pgss-tab").forEach((button) => {
    button.addEventListener("click", () => {
      const name = button.dataset.pgssTab;

      setPgssTab(name);

      if (name === "methodology" || name === "queries") {
        void loadPgssReference();
      }
    });
  });
}
