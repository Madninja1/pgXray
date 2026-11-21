"use strict";

let autovacuumCurrentSnapshotId = null;
let autovacuumRelationHistory = null;
let autovacuumMaintenanceAdvice = null;
let autovacuumPolicyCandidates = null;

const AUTOVACUUM_PAGE_SIZES = [20, 50, 100, 500];
let autovacuumTablePage = 1;
let autovacuumTablePageSize = 20;
let autovacuumHistoryPage = 1;
let autovacuumHistoryPageSize = 20;
let autovacuumComparisonRows = [];
let autovacuumComparisonPage = 1;
let autovacuumComparisonPageSize = 20;

function renderAutovacuumConnectionOptions(connections) {
  const select = byId("autovacuum-connection");
  const previous = select.value;

  select.replaceChildren();

  if (connections.length === 0) {
    const option = document.createElement("option");
    option.value = "";
    option.textContent = "Нет сохранённых подключений";
    select.append(option);
    select.disabled = true;
    byId("collect-autovacuum").disabled = true;
    return;
  }

  select.disabled = false;
  byId("collect-autovacuum").disabled = false;

  for (const connection of connections) {
    const option = document.createElement("option");
    option.value = String(connection.id);
    option.textContent =
        `${connection.name} — ${connection.username}@${connection.host}:${connection.port}/${connection.database}`;
    select.append(option);
  }

  if (previous && connections.some((connection) => String(connection.id) === previous)) {
    select.value = previous;
  }
}

function setAutovacuumTab(name) {
  document.querySelectorAll(".autovacuum-tab").forEach((button) => {
    const active = button.dataset.autovacuumTab === name;
    button.classList.toggle("active", active);
    button.setAttribute("aria-selected", String(active));
  });

  for (const candidate of [
    "snapshot",
    "table",
    "progress",
    "policy",
    "methodology",
    "queries",
  ]) {
    byId(`autovacuum-${candidate}-tab`).hidden = candidate !== name;
  }
}

function autovacuumProfileLabel(value) {
  return {
    static_or_no_activity: "static / no activity",
    append_only_observed: "append-only",
    update_delete_heavy: "UPDATE / DELETE-heavy",
    mixed: "mixed",
  }[value] ?? value ?? "—";
}

function autovacuumTriggerLabel(current, trigger, usedPct, reached, applicable = true) {
  if (!applicable) {
    return "parent: N/A";
  }

  if (trigger === null || trigger === undefined) {
    return "disabled";
  }

  const prefix = reached ? "достигнут · " : "";
  return `${prefix}${formatInteger(current)} / ${formatInteger(trigger)} · ${formatPercent(usedPct)}`;
}

function renderAutovacuumInterpretationAt(
    listId,
    limitationsId,
    limitationsListId,
    interpretation,
) {
  const list = byId(listId);
  const limitations = byId(limitationsId);
  const limitationsList = byId(limitationsListId);

  list.replaceChildren();
  limitationsList.replaceChildren();

  const items = interpretation?.items ?? [];

  if (items.length === 0) {
    const empty = document.createElement("p");
    empty.className = "empty-state";
    empty.textContent = "Отдельных диагностических сигналов для этого снимка нет.";
    list.append(empty);
  } else {
    items.forEach((item, index) => {
      const card = document.createElement("details");
      const summary = document.createElement("summary");
      const title = document.createElement("strong");
      card.className = "interpretation-card autovacuum-interpretation-card";
      card.open = index === 0;
      title.textContent = item.title;
      summary.append(title);
      card.append(summary);

      for (const [label, values] of [
        ["Что обнаружено", item.observed ?? []],
        ["Что это может означать", item.meaning ?? []],
        ["Что проверить следующим", item.next_steps ?? []],
      ]) {
        const group = document.createElement("div");
        const heading = document.createElement("strong");
        const ul = document.createElement("ul");
        group.className = "interpretation-group";
        heading.textContent = label;
        for (const value of values) {
          const li = document.createElement("li");
          li.textContent = value;
          ul.append(li);
        }
        group.append(heading, ul);
        card.append(group);
      }

      list.append(card);
    });
  }

  const limitValues = interpretation?.limitations ?? [];
  for (const value of limitValues) {
    const li = document.createElement("li");
    li.textContent = value;
    limitationsList.append(li);
  }
  limitations.hidden = limitValues.length === 0;
}

function renderAutovacuumInterpretationCards(listId, interpretation) {
  const list = byId(listId);
  list.replaceChildren();
  const items = interpretation?.items ?? [];
  if (items.length === 0) {
    const empty = document.createElement("p");
    empty.className = "empty-state";
    empty.textContent = "Для этого интервала нет отдельной автоматической интерпретации.";
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
      const ul = document.createElement("ul");
      group.className = "interpretation-group";
      heading.textContent = label;
      for (const value of values) {
        const li = document.createElement("li");
        li.textContent = value;
        ul.append(li);
      }
      group.append(heading, ul);
      card.append(group);
    }
    list.append(card);
  }
}

function formatSignedBytes(value) {
  if (value === null || value === undefined) return "—";
  const number = Number(value);
  if (number === 0) return "0 B";
  return `${number > 0 ? "+" : "−"}${formatBytes(Math.abs(number))}`;
}

function autovacuumAttentionReasons(table) {
  const reasons = [];
  if (table.autovacuum_applicable && !table.autovacuum_enabled) {
    reasons.push("regular autovacuum выключен");
  }
  if (table.dead_trigger_reached) reasons.push("dead-tuple trigger достигнут");
  if (table.insert_trigger_reached) reasons.push("insert trigger достигнут");
  if (table.analyze_trigger_reached) reasons.push("ANALYZE trigger достигнут");
  if (table.toast?.dead_trigger_reached || table.toast?.insert_trigger_reached) {
    reasons.push("TOAST trigger достигнут");
  }
  return reasons;
}

function autovacuumAttentionScore(table) {
  if (table.autovacuum_applicable && !table.autovacuum_enabled) return Number.POSITIVE_INFINITY;
  return Math.max(
      Number(table.dead_trigger_used_pct ?? 0),
      Number(table.insert_trigger_used_pct ?? 0),
      Number(table.analyze_trigger_used_pct ?? 0),
      Number(table.toast?.dead_trigger_used_pct ?? 0),
      Number(table.toast?.insert_trigger_used_pct ?? 0),
  );
}

function renderAutovacuumFindings(tables) {
  const target = byId("autovacuum-finding-list");
  target.replaceChildren();

  const findings = (tables ?? [])
      .filter((table) => autovacuumAttentionReasons(table).length > 0)
      .sort((a, b) => autovacuumAttentionScore(b) - autovacuumAttentionScore(a));

  byId("autovacuum-attention-count").textContent = formatInteger(findings.length);

  if (findings.length === 0) {
    const empty = document.createElement("p");
    empty.className = "empty-state";
    empty.textContent = "В текущем snapshot нет relations с достигнутыми maintenance triggers или отключённым regular autovacuum.";
    target.append(empty);
    return;
  }

  for (const table of findings.slice(0, 8)) {
    const card = document.createElement("article");
    const main = document.createElement("div");
    const title = document.createElement("strong");
    const reason = document.createElement("p");
    const context = document.createElement("p");
    const button = document.createElement("button");

    card.className = "autovacuum-finding";
    main.className = "autovacuum-finding-main";
    title.textContent = `${table.schema_name}.${table.relation_name}`;
    reason.textContent = autovacuumAttentionReasons(table).join(" · ");
    reason.className = "autovacuum-finding-reason";
    context.className = "hint";
    context.textContent = `dead ${formatInteger(table.dead_tuples)} / ${formatInteger(table.dead_tuple_trigger)} · analyze ${formatInteger(table.modifications_since_analyze)} / ${formatInteger(table.analyze_trigger)} · last autovacuum ${formatTimestamp(table.last_autovacuum)}`;
    main.append(title, reason, context);

    button.type = "button";
    button.className = "secondary compact-button";
    button.textContent = "Разобрать relation";
    button.dataset.autovacuumRelationOid = String(table.relation_oid);
    card.append(main, button);
    target.append(card);
  }

  if (findings.length > 8) {
    const note = document.createElement("p");
    note.className = "hint";
    note.textContent = `Показаны первые 8 из ${findings.length}. Полный список — в Table health ниже.`;
    target.append(note);
  }
}

function autovacuumPageWindow(total, page, pageSize) {
  const totalPages = Math.max(1, Math.ceil(total / pageSize));
  const safePage = Math.min(Math.max(page, 1), totalPages);
  const start = (safePage - 1) * pageSize;
  const end = Math.min(start + pageSize, total);

  return {
    page: safePage,
    totalPages,
    start,
    end,
  };
}

function renderAutovacuumPagers(kind, total, page, pageSize) {
  const range = autovacuumPageWindow(total, page, pageSize);

  document.querySelectorAll(`[data-autovacuum-pager="${kind}"]`).forEach((container) => {
    container.replaceChildren();
    container.hidden = total === 0;

    if (total === 0) {
      return;
    }

    const navigation = document.createElement("div");
    const previous = document.createElement("button");
    const status = document.createElement("span");
    const next = document.createElement("button");
    const sizeLabel = document.createElement("label");
    const sizeSelect = document.createElement("select");

    navigation.className = "history-pager-navigation";
    previous.type = "button";
    previous.className = "secondary";
    previous.dataset.autovacuumPagerKind = kind;
    previous.dataset.autovacuumPagerAction = "previous";
    previous.textContent = "← Назад";
    previous.disabled = range.page <= 1;

    status.className = "history-pager-status";
    status.textContent =
        `${formatInteger(range.start + 1)}–${formatInteger(range.end)} из ${formatInteger(total)} · страница ${formatInteger(range.page)}/${formatInteger(range.totalPages)}`;

    next.type = "button";
    next.className = "secondary";
    next.dataset.autovacuumPagerKind = kind;
    next.dataset.autovacuumPagerAction = "next";
    next.textContent = "Вперёд →";
    next.disabled = range.page >= range.totalPages;

    sizeLabel.className = "history-pager-size";
    sizeLabel.append("На странице ");
    sizeSelect.dataset.autovacuumPagerSize = kind;

    for (const size of AUTOVACUUM_PAGE_SIZES) {
      const option = document.createElement("option");
      option.value = String(size);
      option.textContent = size === 500 ? "500 (макс.)" : String(size);
      sizeSelect.append(option);
    }

    sizeSelect.value = String(pageSize);
    sizeLabel.append(sizeSelect);
    navigation.append(previous, status, next);
    container.append(navigation, sizeLabel);
  });
}

function scrollAutovacuumPagerBlock(kind) {
  const target = {
    table: byId("autovacuum-table-health-section"),
    history: byId("autovacuum-saved-snapshots"),
    comparison: byId("autovacuum-compare-tables-section"),
  }[kind];

  target?.scrollIntoView({
    behavior: "smooth",
    block: "start",
  });
}

function scrollAutovacuumTableDetail() {
  requestAnimationFrame(() => {
    byId("autovacuum-table-tab")?.scrollIntoView({
      behavior: "smooth",
      block: "start",
    });
  });
}

function autovacuumTableMatchesFilter(table) {
  const search = (byId("autovacuum-table-filter")?.value ?? "").trim().toLocaleLowerCase("ru");
  const mode = byId("autovacuum-table-mode")?.value ?? "all";
  const name = `${table.schema_name}.${table.relation_name}`.toLocaleLowerCase("ru");
  if (search && !name.includes(search) && !String(table.relation_oid).includes(search)) return false;

  if (mode === "attention") return autovacuumAttentionReasons(table).length > 0;
  if (mode === "vacuum") return table.dead_trigger_reached || table.insert_trigger_reached;
  if (mode === "analyze") return table.analyze_trigger_reached;
  if (mode === "disabled") return table.autovacuum_applicable && !table.autovacuum_enabled;
  return true;
}

function renderAutovacuumTables(section) {
  renderSectionError("autovacuum-tables-error", section);
  const body = byId("autovacuum-table-list");
  const allRows = (section?.data ?? []).filter(autovacuumTableMatchesFilter);
  const range = autovacuumPageWindow(
      allRows.length,
      autovacuumTablePage,
      autovacuumTablePageSize,
  );

  autovacuumTablePage = range.page;
  body.replaceChildren();

  if (allRows.length === 0) {
    tableEmpty(body, 11, section?.data == null
      ? "Сбор таблиц не выполнен — смотри причину выше."
      : "По выбранному фильтру таблиц не найдено.");
    renderAutovacuumPagers("table", 0, 1, autovacuumTablePageSize);
    return;
  }

  for (const table of allRows.slice(range.start, range.end)) {
    const row = document.createElement("tr");
    row.classList.toggle("autovacuum-row-attention", autovacuumAttentionReasons(table).length > 0);
    appendCells(row, [
      `${table.schema_name}.${table.relation_name}`,
      autovacuumProfileLabel(table.workload_profile),
      formatBytes(table.total_size_bytes),
      `${formatInteger(table.live_tuples)} / ${formatInteger(table.dead_tuples)}`,
      formatPercent(table.dead_tuple_pct),
      autovacuumTriggerLabel(
          table.dead_tuples,
          table.dead_tuple_trigger,
          table.dead_trigger_used_pct,
          table.dead_trigger_reached,
          table.autovacuum_applicable,
      ),
      autovacuumTriggerLabel(
          table.inserts_since_vacuum,
          table.insert_trigger,
          table.insert_trigger_used_pct,
          table.insert_trigger_reached,
          table.autovacuum_applicable,
      ),
      autovacuumTriggerLabel(
          table.modifications_since_analyze,
          table.analyze_trigger,
          table.analyze_trigger_used_pct,
          table.analyze_trigger_reached,
          table.autovacuum_applicable,
      ),
      formatPercent(table.hot_update_pct),
      formatTimestamp(table.last_autovacuum),
    ]);

    const actionCell = document.createElement("td");
    const button = document.createElement("button");
    button.type = "button";
    button.className = "secondary compact-button";
    button.textContent = "Подробнее";
    button.dataset.autovacuumRelationOid = String(table.relation_oid);
    actionCell.append(button);
    row.append(actionCell);
    body.append(row);
  }

  renderAutovacuumPagers(
      "table",
      allRows.length,
      autovacuumTablePage,
      autovacuumTablePageSize,
  );
}

function renderAutovacuumProgress(snapshot) {
  renderSectionError("autovacuum-vacuums-error", snapshot.vacuums);
  renderSectionError("autovacuum-analyzes-error", snapshot.analyzes);

  const vacuumBody = byId("autovacuum-vacuum-list");
  const vacuums = snapshot.vacuums?.data ?? [];
  vacuumBody.replaceChildren();
  if (vacuums.length === 0) {
    tableEmpty(vacuumBody, 10, snapshot.vacuums?.data == null
      ? "Не удалось проверить активные VACUUM."
      : "Активных VACUUM сейчас нет.");
  } else {
    for (const vacuum of vacuums) {
      const row = document.createElement("tr");
      appendCells(row, [
        String(vacuum.pid),
        `${vacuum.schema_name}.${vacuum.relation_name}`,
        vacuum.is_autovacuum ? "autovacuum" : "manual",
        vacuum.phase,
        formatPercent(vacuum.scan_pct),
        formatPercent(vacuum.vacuum_pct),
        formatInteger(vacuum.index_vacuum_count),
        vacuum.dead_tuple_bytes != null && vacuum.max_dead_tuple_bytes != null
          ? `${formatBytes(vacuum.dead_tuple_bytes)} / ${formatBytes(vacuum.max_dead_tuple_bytes)} (${formatInteger(vacuum.num_dead_item_ids)} item IDs)`
          : `${formatInteger(vacuum.num_dead_tuples)} / ${formatInteger(vacuum.max_dead_tuples)} tuples`,
        formatDuration(vacuum.query_age_seconds),
        [vacuum.wait_event_type, vacuum.wait_event].filter(Boolean).join(" / ") || "—",
      ]);
      vacuumBody.append(row);
    }
  }

  const analyzeBody = byId("autovacuum-analyze-list");
  const analyzes = snapshot.analyzes?.data ?? [];
  analyzeBody.replaceChildren();
  if (analyzes.length === 0) {
    tableEmpty(analyzeBody, 9, snapshot.analyzes?.data == null
      ? "Не удалось проверить активные ANALYZE."
      : "Активных ANALYZE сейчас нет.");
  } else {
    for (const analyze of analyzes) {
      const row = document.createElement("tr");
      appendCells(row, [
        String(analyze.pid),
        `${analyze.schema_name}.${analyze.relation_name}`,
        analyze.is_autoanalyze ? "autoanalyze" : "manual",
        analyze.phase,
        `${formatInteger(analyze.sample_blks_scanned)} / ${formatInteger(analyze.sample_blks_total)} · ${formatPercent(analyze.sample_pct)}`,
        `${formatInteger(analyze.ext_stats_computed)} / ${formatInteger(analyze.ext_stats_total)}`,
        `${formatInteger(analyze.child_tables_done)} / ${formatInteger(analyze.child_tables_total)}`,
        formatDuration(analyze.query_age_seconds),
        [analyze.wait_event_type, analyze.wait_event].filter(Boolean).join(" / ") || "—",
      ]);
      analyzeBody.append(row);
    }
  }
}

function renderAutovacuumPolicyRelations(tables) {
  const select = byId("autovacuum-policy-relation");
  const previous = select.value;
  select.replaceChildren();

  const candidates = (tables ?? []).filter((table) => table.autovacuum_applicable);
  if (candidates.length === 0) {
    const option = document.createElement("option");
    option.value = "";
    option.textContent = "Нет обычных tables";
    select.append(option);
    select.disabled = true;
    byId("simulate-autovacuum-policy").disabled = true;
    return;
  }

  select.disabled = false;
  byId("simulate-autovacuum-policy").disabled = false;
  for (const table of candidates) {
    const option = document.createElement("option");
    option.value = String(table.relation_oid);
    option.textContent = `${table.schema_name}.${table.relation_name} · trigger ${formatInteger(table.dead_tuple_trigger)}`;
    select.append(option);
  }
  if (previous && candidates.some((table) => String(table.relation_oid) === previous)) {
    select.value = previous;
  }
}

// Keep collection failures in the main diagnostic flow, not only in a
// collapsed technical panel or transient toast.
function autovacuumCollectionIssues(snapshot) {
  return [
    ["Таблицы и пороги VACUUM/ANALYZE", snapshot.tables],
    ["Активные VACUUM", snapshot.vacuums],
    ["Активные ANALYZE", snapshot.analyzes],
  ].filter(([, section]) => !Array.isArray(section?.data)).map(([label, section]) => ({
    label,
    error: section?.error || "Раздел не был собран; причина не указана.",
  }));
}

function renderAutovacuumCollectionStatus(snapshot) {
  const issues = autovacuumCollectionIssues(snapshot);
  const panel = byId("autovacuum-collection-status");
  const list = byId("autovacuum-collection-issues");
  list.replaceChildren();
  for (const issue of issues) {
    const item = document.createElement("li");
    const title = document.createElement("strong");
    title.textContent = `${issue.label}: `;
    item.append(title, document.createTextNode(issue.error));
    list.append(item);
  }
  panel.hidden = issues.length === 0;
}

function renderAutovacuumSnapshot(snapshot, history = null) {
  autovacuumSnapshot = snapshot;
  autovacuumCurrentSnapshotId = history?.id ?? null;
  autovacuumRelationHistory = null;
  autovacuumMaintenanceAdvice = null;
  autovacuumPolicyCandidates = null;
  autovacuumServerVersionNum = snapshot.overview.server_version_num;
  autovacuumTablePage = 1;

  const tables = snapshot.tables?.data ?? [];
  const totalSize = tables.reduce((sum, table) => sum + Number(table.total_size_bytes || 0), 0);
  const totalDead = tables.reduce((sum, table) => sum + Number(table.dead_tuples || 0), 0);
  const deadReached = tables.filter((table) => table.dead_trigger_reached).length;
  const analyzeReached = tables.filter((table) => table.analyze_trigger_reached).length;
  const activeVacuum = snapshot.vacuums?.data?.length ?? 0;
  const activeAnalyze = snapshot.analyzes?.data?.length ?? 0;

  // Never equate an unavailable section with zero tables or zero activity.
  const hasTables = Array.isArray(snapshot.tables?.data);
  byId("autovacuum-table-count").textContent = hasTables ? formatInteger(tables.length) : "—";
  byId("autovacuum-total-size").textContent = hasTables ? formatBytes(totalSize) : "—";
  byId("autovacuum-total-dead").textContent = hasTables ? formatInteger(totalDead) : "—";
  byId("autovacuum-dead-reached").textContent = hasTables ? formatInteger(deadReached) : "—";
  byId("autovacuum-analyze-reached").textContent = hasTables ? formatInteger(analyzeReached) : "—";
  byId("autovacuum-active-vacuum").textContent = Array.isArray(snapshot.vacuums?.data) ? formatInteger(activeVacuum) : "—";
  byId("autovacuum-active-analyze").textContent = Array.isArray(snapshot.analyzes?.data) ? formatInteger(activeAnalyze) : "—";
  renderAutovacuumCollectionStatus(snapshot);

  const o = snapshot.overview;
  byId("autovacuum-snapshot-meta").textContent =
      `${o.username}@${o.database} · PostgreSQL ${o.server_version} · ${formatTimestamp(o.measured_at)}`;
  byId("autovacuum-global-enabled").textContent = yesNo(o.autovacuum);
  byId("autovacuum-track-counts").textContent = yesNo(o.track_counts);
  byId("autovacuum-max-workers").textContent = formatInteger(o.autovacuum_max_workers);
  byId("autovacuum-naptime").textContent = o.autovacuum_naptime;
  byId("autovacuum-global-vacuum-trigger").textContent =
      `${formatInteger(o.autovacuum_vacuum_threshold)} + ${formatNumber(o.autovacuum_vacuum_scale_factor, 4)} × reltuples`;
  byId("autovacuum-global-insert-trigger").textContent =
      `${formatInteger(o.autovacuum_vacuum_insert_threshold)} + ${formatNumber(o.autovacuum_vacuum_insert_scale_factor, 4)} × reltuples`;
  byId("autovacuum-global-analyze-trigger").textContent =
      `${formatInteger(o.autovacuum_analyze_threshold)} + ${formatNumber(o.autovacuum_analyze_scale_factor, 4)} × reltuples`;
  byId("autovacuum-global-cost").textContent =
      `${formatInteger(o.autovacuum_vacuum_cost_limit)} / ${o.autovacuum_vacuum_cost_delay}`;
  byId("autovacuum-stats-reset").textContent = o.stats_reset
      ? formatTimestamp(o.stats_reset)
      : "not explicitly reset";

  renderAutovacuumFindings(tables);
  if (!hasTables) {
    byId("autovacuum-attention-count").textContent = "—";
    const warning = document.createElement("p");
    warning.className = "section-error";
    warning.textContent = "Состояние таблиц не проверено: нельзя утверждать, что проблем нет.";
    byId("autovacuum-finding-list").replaceChildren(warning);
  }
  renderAutovacuumInterpretationAt(
      "autovacuum-interpretation-list",
      "autovacuum-interpretation-limitations",
      "autovacuum-interpretation-limitations-list",
      snapshot.interpretation,
  );
  renderAutovacuumTables(snapshot.tables);
  renderAutovacuumProgress(snapshot);
  renderAutovacuumPolicyRelations(tables);
  renderAutovacuumSources();

  byId("autovacuum-empty").hidden = true;
  byId("autovacuum-content").hidden = false;
}

function resetAutovacuumSnapshot() {
  autovacuumSnapshot = null;
  autovacuumCurrentSnapshotId = null;
  autovacuumRelationHistory = null;
  autovacuumMaintenanceAdvice = null;
  autovacuumPolicyCandidates = null;
  selectedAutovacuumRelationOid = null;
  autovacuumServerVersionNum = null;
  byId("autovacuum-empty").hidden = false;
  byId("autovacuum-content").hidden = true;
  byId("autovacuum-collection-status").hidden = true;
  byId("autovacuum-collect-error").hidden = true;
  byId("autovacuum-table-empty").hidden = false;
  byId("autovacuum-table-content").hidden = true;
  byId("autovacuum-policy-result").hidden = true;
  byId("autovacuum-policy-rollback").hidden = true;
  byId("autovacuum-policy-rollback-sql").textContent = "";
  autovacuumTablePage = 1;
  renderAutovacuumPagers("table", 0, 1, autovacuumTablePageSize);
  renderAutovacuumPolicyRelations([]);
  renderAutovacuumSources();
}

function autovacuumRateText(value) {
  return value === null || value === undefined ? "—" : `${formatNumber(value, 0)}/h`;
}

function autovacuumHistorySpanText(seconds) {
  if (seconds === null || seconds === undefined) return "—";
  return formatDuration(Math.round(seconds));
}

function autovacuumHistoryDmlDelta(point, older) {
  if (!older || point.stats_reset !== older.stats_reset) return "—";
  const values = [
    point.table.tuples_inserted - older.table.tuples_inserted,
    point.table.tuples_updated - older.table.tuples_updated,
    point.table.tuples_deleted - older.table.tuples_deleted,
  ];
  if (values.some((value) => value < 0)) return "—";
  return values.map((value) => formatSignedInteger(value)).join(" / ");
}

function renderAutovacuumRelationHistory(history) {
  autovacuumRelationHistory = history;
  const summary = history?.summary ?? {};
  const points = history?.points ?? [];
  const body = byId("autovacuum-relation-history-list");
  const error = byId("autovacuum-relation-history-error");
  const limitations = byId("autovacuum-relation-history-limitations-list");

  error.hidden = true;
  error.textContent = "";
  body.replaceChildren();
  limitations.replaceChildren();

  byId("autovacuum-relation-history-span").textContent =
      `${formatInteger(summary.history_points ?? 0)} · ${autovacuumHistorySpanText(summary.history_span_seconds)}`;
  byId("autovacuum-relation-history-dml").textContent =
      `I ${autovacuumRateText(summary.inserts_per_hour)} · U ${autovacuumRateText(summary.updates_per_hour)} · D ${autovacuumRateText(summary.deletes_per_hour)}`;
  byId("autovacuum-relation-history-dead").textContent = summary.dead_growth_per_hour === null || summary.dead_growth_per_hour === undefined
      ? "не рассчитывается"
      : `${formatNumber(summary.dead_growth_per_hour, 0)}/h`;
  byId("autovacuum-relation-history-vacuum").textContent = summary.vacuum_cycles === null || summary.vacuum_cycles === undefined
      ? "—"
      : formatInteger(summary.vacuum_cycles);
  byId("autovacuum-relation-history-analyze").textContent = summary.analyze_cycles === null || summary.analyze_cycles === undefined
      ? "—"
      : formatInteger(summary.analyze_cycles);
  byId("autovacuum-relation-history-hot").textContent = formatPercent(summary.hot_update_pct);
  byId("autovacuum-relation-history-note").textContent = summary.stats_period_continuous
      ? "Cumulative DML и maintenance counters относятся к одному stats period."
      : "DML rates отключены: history содержит reset/неизвестный stats period или пока недостаточно точек.";

  if (points.length === 0) {
    tableEmpty(body, 10, "Для relation ещё нет сохранённых Autovacuum snapshots.");
  } else {
    points.forEach((point, index) => {
      const older = points[index + 1] ?? null;
      const table = point.table;
      const active = [
        point.active_vacuum ? `VACUUM: ${point.active_vacuum.phase}` : null,
        point.active_analyze ? `ANALYZE: ${point.active_analyze.phase}` : null,
      ].filter(Boolean).join(" · ") || "—";
      const row = document.createElement("tr");
      appendCells(row, [
        `#${point.snapshot_id}`,
        formatSnapshotTime(point.collection_started_at_ms),
        `${formatInteger(table.live_tuples)} / ${formatInteger(table.dead_tuples)}`,
        autovacuumTriggerLabel(table.dead_tuples, table.dead_tuple_trigger, table.dead_trigger_used_pct, table.dead_trigger_reached, table.autovacuum_applicable),
        autovacuumTriggerLabel(table.inserts_since_vacuum, table.insert_trigger, table.insert_trigger_used_pct, table.insert_trigger_reached, table.autovacuum_applicable),
        autovacuumTriggerLabel(table.modifications_since_analyze, table.analyze_trigger, table.analyze_trigger_used_pct, table.analyze_trigger_reached, table.autovacuum_applicable),
        autovacuumHistoryDmlDelta(point, older),
        `${formatInteger(table.vacuum_count)} / ${formatInteger(table.autovacuum_count)}`,
        `${formatInteger(table.analyze_count)} / ${formatInteger(table.autoanalyze_count)}`,
        active,
      ]);
      body.append(row);
    });
  }

  for (const value of history?.limitations ?? []) {
    const li = document.createElement("li");
    li.textContent = value;
    limitations.append(li);
  }
  byId("autovacuum-relation-history-limitations").hidden = limitations.children.length === 0;
}

function renderAutovacuumRelationHistoryError(text) {
  autovacuumRelationHistory = null;
  const error = byId("autovacuum-relation-history-error");
  error.textContent = text;
  error.hidden = false;
  byId("autovacuum-relation-history-list").replaceChildren();
  tableEmpty(byId("autovacuum-relation-history-list"), 10, "History недоступна.");
  for (const id of [
    "autovacuum-relation-history-span",
    "autovacuum-relation-history-dml",
    "autovacuum-relation-history-dead",
    "autovacuum-relation-history-vacuum",
    "autovacuum-relation-history-analyze",
    "autovacuum-relation-history-hot",
  ]) byId(id).textContent = "—";
}

function autovacuumAdvisorStatusLabel(status) {
  return {
    investigate: "investigate",
    candidate: "candidate",
    review: "review",
    insufficient_quality: "data quality",
    insufficient_history: "need history",
    observe: "observe",
    working: "working",
  }[status] ?? status ?? "—";
}

function autovacuumAdvisorAreaLabel(area) {
  return {
    vacuum: "VACUUM",
    insert: "INSERT VACUUM",
    analyze: "ANALYZE",
    toast: "TOAST",
    hot: "HOT / indexes",
    history: "History",
  }[area] ?? area ?? "Maintenance";
}

function renderAutovacuumMaintenanceAdvice(advice) {
  autovacuumMaintenanceAdvice = advice;
  const status = byId("autovacuum-advisor-status");
  const target = byId("autovacuum-advisor-findings");
  const limitations = byId("autovacuum-advisor-limitations-list");
  const error = byId("autovacuum-advisor-error");

  error.hidden = true;
  error.textContent = "";
  status.textContent = autovacuumAdvisorStatusLabel(advice?.status);
  status.className = `autovacuum-advisor-status status-${advice?.status ?? "observe"}`;
  byId("autovacuum-advisor-title").textContent = advice?.title ?? "Maintenance Advisor";
  byId("autovacuum-advisor-summary").textContent = advice?.summary ?? "—";
  byId("autovacuum-advisor-anchor").textContent = advice
    ? `Snapshot-anchored evidence · ${formatInteger(advice.history_points)} points · ${autovacuumHistorySpanText(advice.history_span_seconds)} · stats ${advice.stats_period_continuous ? "continuous" : "not comparable"}`
    : "—";

  target.replaceChildren();
  const findings = advice?.findings ?? [];
  if (findings.length === 0) {
    const empty = document.createElement("p");
    empty.className = "empty-state";
    empty.textContent = "Advisor не сформировал отдельных findings.";
    target.append(empty);
  } else {
    findings.forEach((finding, index) => {
      const card = document.createElement("details");
      const summary = document.createElement("summary");
      const area = document.createElement("span");
      const title = document.createElement("strong");
      const findingStatus = document.createElement("span");
      const text = document.createElement("p");
      card.className = `autovacuum-advisor-finding status-${finding.status}`;
      card.open = index === 0;
      area.className = "autovacuum-advisor-area";
      area.textContent = autovacuumAdvisorAreaLabel(finding.area);
      title.textContent = finding.title;
      findingStatus.className = `autovacuum-advisor-finding-status status-${finding.status}`;
      findingStatus.textContent = autovacuumAdvisorStatusLabel(finding.status);
      summary.append(area, title, findingStatus);
      card.append(summary);
      text.textContent = finding.summary;
      card.append(text);

      if ((finding.observed ?? []).length > 0) {
        const heading = document.createElement("strong");
        const ul = document.createElement("ul");
        heading.textContent = "Evidence";
        for (const value of finding.observed) {
          const li = document.createElement("li");
          li.textContent = value;
          ul.append(li);
        }
        card.append(heading, ul);
      }

      if ((finding.policy_parameters ?? []).length > 0) {
        const heading = document.createElement("strong");
        const parameters = document.createElement("code");
        heading.textContent = "Policy levers";
        parameters.className = "autovacuum-advisor-parameters";
        parameters.textContent = finding.policy_parameters.join(" · ");
        card.append(heading, parameters);
      }

      if ((finding.next_steps ?? []).length > 0) {
        const heading = document.createElement("strong");
        const ul = document.createElement("ul");
        heading.textContent = "Следующий шаг";
        for (const value of finding.next_steps) {
          const li = document.createElement("li");
          li.textContent = value;
          ul.append(li);
        }
        card.append(heading, ul);
      }

      if (
        finding.status === "candidate" &&
        finding.area === "vacuum" &&
        (finding.policy_parameters ?? []).includes("autovacuum_vacuum_scale_factor")
      ) {
        const button = document.createElement("button");
        button.type = "button";
        button.className = "secondary compact-button";
        button.textContent = "Открыть Manual calculator";
        button.dataset.autovacuumOpenManualCalculator = String(advice.relation_oid);
        card.append(button);
      }

      target.append(card);
    });
  }

  limitations.replaceChildren();
  for (const value of advice?.limitations ?? []) {
    const li = document.createElement("li");
    li.textContent = value;
    limitations.append(li);
  }
  byId("autovacuum-advisor-limitations").hidden = limitations.children.length === 0;
}

function renderAutovacuumMaintenanceAdviceError(text) {
  autovacuumMaintenanceAdvice = null;
  const error = byId("autovacuum-advisor-error");
  error.textContent = text;
  error.hidden = false;
  byId("autovacuum-advisor-status").textContent = "unavailable";
  byId("autovacuum-advisor-status").className = "autovacuum-advisor-status status-insufficient_quality";
  byId("autovacuum-advisor-title").textContent = "Maintenance Advisor недоступен";
  byId("autovacuum-advisor-summary").textContent = "History-based analysis не выполнен.";
  byId("autovacuum-advisor-anchor").textContent = "—";
  byId("autovacuum-advisor-findings").replaceChildren();
  byId("autovacuum-advisor-limitations-list").replaceChildren();
  byId("autovacuum-advisor-limitations").hidden = true;
}

function autovacuumPolicyScenarioLabel(value) {
  return {
    conservative: "75% trigger",
    earlier: "50% trigger",
  }[value] ?? value ?? "scenario";
}

function autovacuumPolicyAreaLabel(value) {
  return {
    vacuum: "Dead-tuple VACUUM",
    insert: "Insert-driven VACUUM",
    analyze: "ANALYZE",
    toast_vacuum: "TOAST dead-tuple VACUUM",
    toast_insert: "TOAST insert VACUUM",
  }[value] ?? value ?? "Policy";
}

async function applyAutovacuumPolicyCandidate(candidate) {
  const connectionId = Number(byId("autovacuum-connection").value);
  const snapshotId = Number(autovacuumCurrentSnapshotId);
  const relationOid = Number(selectedAutovacuumRelationOid);

  if (
    !Number.isInteger(connectionId) || connectionId <= 0 ||
    !Number.isInteger(snapshotId) || snapshotId <= 0 ||
    !Number.isInteger(relationOid) || relationOid <= 0
  ) {
    message("Для apply нужен сохранённый Autovacuum snapshot и выбранная relation.", true);
    return;
  }

  const confirmed = window.confirm([
    "Применить Autovacuum policy?",
    "",
    `Target: ${candidate.target_relation}`,
    `${candidate.parameter}: ${formatNumber(candidate.current_scale_factor, 8)} (${candidate.current_source}) → ${formatNumber(candidate.candidate_scale_factor, 8)} (table override)`,
    `Absolute trigger at anchor: ${formatInteger(candidate.current_trigger)} → ${formatInteger(candidate.candidate_trigger)}`,
    "",
    "Перед ALTER pgXray повторно проверит connection/database identity, relation/TOAST identity, current effective policy, absolute trigger и отсутствие active VACUUM/ANALYZE. При несовпадении изменение не выполняется.",
    "",
    candidate.sql_preview,
  ].join("\n"));

  if (!confirmed) return;

  byId("autovacuum-policy-rollback").hidden = true;
  byId("autovacuum-policy-rollback-sql").textContent = "";
  await action(async () => {
    const result = await invoke("apply_autovacuum_policy_candidate", {
      connectionId,
      snapshotId,
      relationOid,
      candidateId: candidate.id,
    });

    if (result.snapshot && result.history) {
      renderAutovacuumSnapshot(result.snapshot, result.history);
      await loadAutovacuumTableDetail(relationOid);
      void refreshAutovacuumHistory();
    } else {
      renderAutovacuumPolicyCandidatesError(
          "Policy применена и verified, но post-change snapshot не сохранён. Соберите новый snapshot вручную перед следующим apply.",
      );
    }

    const rollback = result.verification?.rollback_sql;
    const rollbackSection = byId("autovacuum-policy-rollback");
    rollbackSection.hidden = !rollback;
    byId("autovacuum-policy-rollback-sql").textContent = rollback ?? "";
    byId("autovacuum-policy-rollback-note").textContent =
        result.verification?.rollback_note ?? "Для отката проверьте настройки вручную.";

    const suffix = result.warning
      ? ` ${result.warning}`
      : ` Post-change snapshot #${result.history.id} сохранён.`;
    message(
        `Autovacuum policy применена: ${result.candidate.parameter} ${formatNumber(result.verification.before.scale_factor, 8)} → ${formatNumber(result.verification.after.scale_factor, 8)}.${suffix}`,
    );
  }, {
    startMessage: "Проверка guardrails и применение Autovacuum policy…",
  });
}

function renderAutovacuumPolicyCandidates(result) {
  autovacuumPolicyCandidates = result;
  const target = byId("autovacuum-concrete-candidates");
  const limitations = byId("autovacuum-concrete-candidate-limitations-list");
  const error = byId("autovacuum-concrete-candidates-error");
  error.hidden = true;
  error.textContent = "";
  target.replaceChildren();
  limitations.replaceChildren();

  byId("autovacuum-concrete-candidates-summary").textContent = result?.summary ?? "—";
  const candidates = result?.candidates ?? [];
  if (candidates.length === 0) {
    const empty = document.createElement("p");
    empty.className = "empty-state";
    empty.textContent = result?.status === "insufficient_evidence"
      ? "Есть policy area для review, но clean rate evidence пока недостаточно для числового candidate."
      : "Для этого anchor snapshot concrete policy candidate не требуется.";
    target.append(empty);
  } else {
    for (const [index, candidate] of candidates.entries()) {
      const card = document.createElement("details");
      const summary = document.createElement("summary");
      const area = document.createElement("span");
      const scenario = document.createElement("strong");
      const parameter = document.createElement("code");
      card.className = "autovacuum-concrete-candidate";
      card.open = index === 0;
      area.className = "autovacuum-advisor-area";
      area.textContent = autovacuumPolicyAreaLabel(candidate.area);
      scenario.textContent = autovacuumPolicyScenarioLabel(candidate.scenario);
      parameter.textContent = candidate.parameter;
      summary.append(area, scenario, parameter);
      card.append(summary);

      const metrics = document.createElement("div");
      metrics.className = "autovacuum-candidate-metrics";
      for (const [label, value] of [
        ["Scale factor", `${formatNumber(candidate.current_scale_factor, 8)} → ${formatNumber(candidate.candidate_scale_factor, 8)}`],
        ["Absolute trigger", `${formatInteger(candidate.current_trigger)} → ${formatInteger(candidate.candidate_trigger)}`],
        ["Observed rate", `${formatNumber(candidate.observed_rate_per_hour, 0)}/h`],
        ["Volume-equivalent cadence", `${formatNumber(candidate.current_volume_hours, 2)} h → ${formatNumber(candidate.candidate_volume_hours, 2)} h`],
      ]) {
        const item = document.createElement("div");
        const span = document.createElement("span");
        const strong = document.createElement("strong");
        span.textContent = label;
        strong.textContent = value;
        item.append(span, strong);
        metrics.append(item);
      }
      card.append(metrics);

      const targetRelation = document.createElement("p");
      targetRelation.className = "hint";
      targetRelation.textContent = `Target: ${candidate.target_relation} · source ${candidate.current_source} · fixed threshold ${formatNumber(candidate.fixed_threshold, 2)} unchanged`;
      card.append(targetRelation);

      const rationaleTitle = document.createElement("strong");
      const rationale = document.createElement("ul");
      rationaleTitle.textContent = "Почему рассчитано так";
      for (const value of candidate.rationale ?? []) {
        const li = document.createElement("li");
        li.textContent = value;
        rationale.append(li);
      }
      card.append(rationaleTitle, rationale);

      const sqlTitle = document.createElement("strong");
      const pre = document.createElement("pre");
      const code = document.createElement("code");
      sqlTitle.textContent = "SQL preview";
      pre.className = "sql-block";
      code.textContent = candidate.sql_preview;
      pre.append(code);

      const actions = document.createElement("div");
      const applyButton = document.createElement("button");
      actions.className = "autovacuum-candidate-actions";
      applyButton.type = "button";
      applyButton.className = "danger";
      applyButton.textContent = "Применить policy";
      applyButton.addEventListener("click", () => {
        void applyAutovacuumPolicyCandidate(candidate);
      });
      actions.append(applyButton);

      card.append(sqlTitle, pre, actions);
      target.append(card);
    }
  }

  for (const value of result?.limitations ?? []) {
    const li = document.createElement("li");
    li.textContent = value;
    limitations.append(li);
  }
  byId("autovacuum-concrete-candidate-limitations").hidden = limitations.children.length === 0;
}

function renderAutovacuumPolicyCandidatesError(text) {
  autovacuumPolicyCandidates = null;
  const error = byId("autovacuum-concrete-candidates-error");
  error.textContent = text;
  error.hidden = false;
  byId("autovacuum-concrete-candidates-summary").textContent = "Concrete candidates недоступны.";
  byId("autovacuum-concrete-candidates").replaceChildren();
  byId("autovacuum-concrete-candidate-limitations-list").replaceChildren();
  byId("autovacuum-concrete-candidate-limitations").hidden = true;
}

function renderAutovacuumTableDetail(detail) {
  const table = detail.table;
  selectedAutovacuumRelationOid = table.relation_oid;

  byId("autovacuum-table-title").textContent = `${table.schema_name}.${table.relation_name}`;
  byId("autovacuum-table-meta").textContent =
      `OID ${table.relation_oid} · ${table.relation_type} · estimated rows ${formatInteger(table.estimated_rows)} · XID age ${formatInteger(table.xid_age)} · live detail`;
  byId("autovacuum-detail-profile").textContent = autovacuumProfileLabel(table.workload_profile);
  byId("autovacuum-detail-dead").textContent = `${formatInteger(table.dead_tuples)} · ${formatPercent(table.dead_tuple_pct)}`;
  byId("autovacuum-detail-dead-trigger").textContent = autovacuumTriggerLabel(
      table.dead_tuples, table.dead_tuple_trigger, table.dead_trigger_used_pct, table.dead_trigger_reached, table.autovacuum_applicable,
  );
  byId("autovacuum-detail-hot").textContent = formatPercent(table.hot_update_pct);
  byId("autovacuum-detail-size").textContent = formatBytes(table.total_size_bytes);
  byId("autovacuum-detail-index-count").textContent = formatInteger(table.index_count);

  byId("autovacuum-detail-enabled").textContent = table.autovacuum_applicable ? yesNo(table.autovacuum_enabled) : "partitioned parent: N/A";
  byId("autovacuum-detail-vacuum-threshold").textContent = formatNumber(table.effective_vacuum_threshold, 2);
  byId("autovacuum-detail-vacuum-scale").textContent = formatNumber(table.effective_vacuum_scale_factor, 6);
  byId("autovacuum-detail-vacuum-trigger").textContent = formatInteger(table.dead_tuple_trigger);
  byId("autovacuum-detail-insert-policy").textContent = `${formatNumber(table.effective_insert_threshold, 2)} + ${formatNumber(table.effective_insert_scale_factor, 6)} × reltuples`;
  byId("autovacuum-detail-insert-trigger").textContent = formatInteger(table.insert_trigger);
  byId("autovacuum-detail-analyze-policy").textContent = `${formatNumber(table.effective_analyze_threshold, 2)} + ${formatNumber(table.effective_analyze_scale_factor, 6)} × reltuples`;
  byId("autovacuum-detail-analyze-trigger").textContent = formatInteger(table.analyze_trigger);
  byId("autovacuum-detail-cost").textContent = `${formatInteger(table.effective_vacuum_cost_limit)} / ${formatNumber(table.effective_vacuum_cost_delay_ms, 3)} ms`;
  byId("autovacuum-detail-freeze-max").textContent = formatInteger(table.effective_freeze_max_age);
  byId("autovacuum-detail-reloptions").textContent = (table.reloptions ?? []).join(", ") || "global defaults";
  byId("autovacuum-detail-last-vacuum").textContent = `${formatTimestamp(table.last_vacuum)} / ${formatTimestamp(table.last_autovacuum)}`;
  byId("autovacuum-detail-vacuum-count").textContent = `${formatInteger(table.vacuum_count)} / ${formatInteger(table.autovacuum_count)}`;
  byId("autovacuum-detail-last-analyze").textContent = `${formatTimestamp(table.last_analyze)} / ${formatTimestamp(table.last_autoanalyze)}`;
  byId("autovacuum-detail-analyze-count").textContent = `${formatInteger(table.analyze_count)} / ${formatInteger(table.autoanalyze_count)}`;
  byId("autovacuum-detail-inserts-since-vacuum").textContent = formatInteger(table.inserts_since_vacuum);
  byId("autovacuum-detail-mod-since-analyze").textContent = formatInteger(table.modifications_since_analyze);

  renderAutovacuumInterpretationAt(
      "autovacuum-table-interpretation-list",
      "autovacuum-table-interpretation-limitations",
      "autovacuum-table-interpretation-limitations-list",
      detail.interpretation,
  );

  const toast = table.toast;
  byId("autovacuum-toast-section").hidden = !toast;
  if (toast) {
    byId("autovacuum-toast-name").textContent = `${toast.schema_name}.${toast.relation_name} · OID ${toast.relation_oid}`;
    byId("autovacuum-toast-size").textContent = formatBytes(toast.total_size_bytes);
    byId("autovacuum-toast-tuples").textContent = `${formatInteger(toast.live_tuples)} / ${formatInteger(toast.dead_tuples)}`;
    byId("autovacuum-toast-dead-trigger").textContent = autovacuumTriggerLabel(
        toast.dead_tuples,
        toast.dead_tuple_trigger,
        toast.dead_trigger_used_pct,
        toast.dead_trigger_reached,
        true,
    );
    byId("autovacuum-toast-insert-trigger").textContent = autovacuumTriggerLabel(
        toast.inserts_since_vacuum,
        toast.insert_trigger,
        toast.insert_trigger_used_pct,
        toast.insert_trigger_reached,
        true,
    );
    byId("autovacuum-toast-last-vacuum").textContent = `${formatTimestamp(toast.last_autovacuum)} / ${formatInteger(toast.autovacuum_count)}`;
    byId("autovacuum-toast-xid-age").textContent = formatInteger(toast.xid_age);
    byId("autovacuum-toast-reloptions").textContent = (toast.reloptions ?? []).join(", ") || "global defaults";
  }

  renderSectionError("autovacuum-indexes-error", detail.indexes);
  const indexBody = byId("autovacuum-index-list");
  const indexes = detail.indexes?.data ?? [];
  indexBody.replaceChildren();
  if (indexes.length === 0) {
    tableEmpty(indexBody, 5);
  } else {
    for (const index of indexes) {
      const flags = [
        index.is_primary ? "PK" : null,
        index.is_unique ? "UNIQUE" : null,
        index.has_expression ? "expression" : null,
        index.is_partial ? "partial" : null,
      ].filter(Boolean).join(", ") || "—";
      const row = document.createElement("tr");
      appendCells(row, [
        index.index_name,
        flags,
        formatBytes(index.index_size_bytes),
        [
          (index.indexed_columns ?? []).join(", "),
          (index.included_columns ?? []).length ? `INCLUDE (${index.included_columns.join(", ")})` : null,
        ].filter(Boolean).join(" · ") || "—",
        index.predicate ?? "—",
      ]);
      indexBody.append(row);
    }
  }

  renderSectionError("autovacuum-updates-error", detail.update_statements);
  const updateBody = byId("autovacuum-update-list");
  const updates = detail.update_statements?.data ?? [];
  updateBody.replaceChildren();
  if (updates.length === 0) {
    tableEmpty(updateBody, 6, "UPDATE patterns не найдены или недоступны.");
  } else {
    for (const item of updates) {
      const row = document.createElement("tr");
      appendCells(row, [
        item.queryid ?? "—",
        formatInteger(item.calls),
        `${formatNumber(item.mean_exec_time_ms, 2)} ms`,
        `${formatNumber(item.total_exec_time_ms, 1)} ms`,
        formatInteger(item.rows),
        item.query,
      ]);
      updateBody.append(row);
    }
  }

  const candidateBody = byId("autovacuum-scale-candidate-list");
  candidateBody.replaceChildren();
  for (const candidate of detail.fixed_scale_candidates ?? []) {
    const row = document.createElement("tr");
    appendCells(row, [
      formatNumber(candidate.scale_factor, 4),
      formatInteger(candidate.resulting_dead_tuple_trigger),
    ]);
    candidateBody.append(row);
  }

  byId("autovacuum-table-empty").hidden = true;
  byId("autovacuum-table-content").hidden = false;
  setAutovacuumTab("table");
}

async function loadAutovacuumTableDetail(
    relationOid,
    {scrollToDetail = true} = {},
) {
  const connectionId = Number(byId("autovacuum-connection").value);
  if (!Number.isInteger(connectionId) || connectionId <= 0) {
    throw new Error("Сначала выберите подключение PostgreSQL.");
  }

  const detailPromise = invoke("autovacuum_table_detail", {
    connectionId,
    relationOid,
  });
  const historyPromise = autovacuumCurrentSnapshotId
      ? invoke("autovacuum_relation_history", {
          snapshotId: autovacuumCurrentSnapshotId,
          relationOid,
          limit: 100,
        }).then((value) => ({value}), (error) => ({error: String(error)}))
      : Promise.resolve({error: "У текущего diagnostics snapshot нет history id."});

  const advicePromise = autovacuumCurrentSnapshotId
      ? invoke("autovacuum_maintenance_advice", {
          snapshotId: autovacuumCurrentSnapshotId,
          relationOid,
          limit: 100,
        }).then((value) => ({value}), (error) => ({error: String(error)}))
      : Promise.resolve({error: "У текущего diagnostics snapshot нет history id."});

  const candidatesPromise = autovacuumCurrentSnapshotId
      ? invoke("autovacuum_policy_candidates", {
          snapshotId: autovacuumCurrentSnapshotId,
          relationOid,
          limit: 100,
        }).then((value) => ({value}), (error) => ({error: String(error)}))
      : Promise.resolve({error: "У текущего diagnostics snapshot нет history id."});

  const [detail, historyResult, adviceResult, candidatesResult] = await Promise.all([
    detailPromise,
    historyPromise,
    advicePromise,
    candidatesPromise,
  ]);
  renderAutovacuumTableDetail(detail);
  if (historyResult.value) {
    renderAutovacuumRelationHistory(historyResult.value);
  } else {
    renderAutovacuumRelationHistoryError(historyResult.error ?? "Relation history недоступна.");
  }
  if (adviceResult.value) {
    renderAutovacuumMaintenanceAdvice(adviceResult.value);
  } else {
    renderAutovacuumMaintenanceAdviceError(adviceResult.error ?? "Maintenance Advisor недоступен.");
  }
  if (candidatesResult.value) {
    renderAutovacuumPolicyCandidates(candidatesResult.value);
  } else {
    renderAutovacuumPolicyCandidatesError(candidatesResult.error ?? "Concrete policy candidates недоступны.");
  }

  if (scrollToDetail) {
    scrollAutovacuumTableDetail();
  }
}

function renderAutovacuumPolicyResult(result) {
  byId("autovacuum-policy-current-trigger").textContent = formatInteger(result.current_dead_tuple_trigger);
  byId("autovacuum-policy-target-result").textContent = formatInteger(result.target_dead_tuples);
  byId("autovacuum-policy-scale").textContent = result.candidate_scale_factor === null
      ? "не рассчитан"
      : formatNumber(result.candidate_scale_factor, 8);
  byId("autovacuum-policy-result-trigger").textContent = formatInteger(result.resulting_dead_tuple_trigger);

  const notes = byId("autovacuum-policy-notes");
  notes.replaceChildren();
  for (const text of result.notes ?? []) {
    const li = document.createElement("li");
    li.textContent = text;
    notes.append(li);
  }
  byId("autovacuum-policy-result").hidden = false;
}

function autovacuumDocsVersion() {
  if (!autovacuumServerVersionNum) {
    return "14";
  }
  const major = Math.floor(Number(autovacuumServerVersionNum) / 10000);
  return major >= 10 ? String(major) : "14";
}

function autovacuumDocsUrl(path) {
  return `https://www.postgresql.org/docs/${autovacuumDocsVersion()}/${path}`;
}

function renderAutovacuumMethodology() {
  const target = byId("autovacuum-methodology-content");
  target.replaceChildren();
  if (!autovacuumReference) {
    const empty = document.createElement("p");
    empty.className = "empty-state";
    empty.textContent = "Методика недоступна.";
    target.append(empty);
    return;
  }

  autovacuumReference.methodology.forEach((step, index) => {
    const card = document.createElement("article");
    const heading = document.createElement("div");
    const number = document.createElement("span");
    const title = document.createElement("h3");
    const purpose = document.createElement("p");
    const ul = document.createElement("ul");
    card.className = "reference-card";
    heading.className = "reference-heading";
    number.className = "reference-index";
    number.textContent = String(index + 1);
    title.textContent = step.title;
    purpose.textContent = step.purpose;
    heading.append(number, title);
    for (const value of step.how_to_read ?? []) {
      const li = document.createElement("li");
      li.textContent = value;
      ul.append(li);
    }
    card.append(heading, purpose, ul);
    target.append(card);
  });
}

function renderAutovacuumQueries() {
  const target = byId("autovacuum-query-content");
  target.replaceChildren();
  if (!autovacuumReference) {
    const empty = document.createElement("p");
    empty.className = "empty-state";
    empty.textContent = "SQL reference недоступен.";
    target.append(empty);
    return;
  }

  for (const query of autovacuumReference.queries ?? []) {
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
    for (const value of query.output ?? []) {
      const li = document.createElement("li"); li.textContent = value; output.append(li);
    }
    for (const value of query.how_to_read ?? []) {
      const li = document.createElement("li"); li.textContent = value; read.append(li);
    }
    code.textContent = query.sql;
    pre.className = "sql-block";
    pre.append(code);
    details.append(summary, purpose, outputTitle, output, readTitle, read, sqlTitle, pre);
    target.append(details);
  }
}

function renderAutovacuumSources() {
  const target = byId("autovacuum-source-list");
  target.replaceChildren();
  if (!autovacuumReference) {
    const empty = document.createElement("p");
    empty.className = "empty-state";
    empty.textContent = "Primary sources ещё не загружены.";
    target.append(empty);
    return;
  }

  for (const source of autovacuumReference.sources ?? []) {
    const card = document.createElement("article");
    const title = document.createElement("strong");
    const purpose = document.createElement("p");
    const url = document.createElement("code");
    card.className = "source-card";
    title.textContent = source.title;
    purpose.textContent = source.purpose;
    url.className = "source-url";
    url.textContent = autovacuumDocsUrl(source.path);
    card.append(title, purpose, url);
    target.append(card);
  }
}

function renderAutovacuumReference() {
  renderAutovacuumMethodology();
  renderAutovacuumQueries();
  renderAutovacuumSources();
}

async function loadAutovacuumReference() {
  if (autovacuumReference) {
    renderAutovacuumReference();
    return autovacuumReference;
  }
  if (autovacuumReferencePromise) {
    return autovacuumReferencePromise;
  }

  autovacuumReferencePromise = invoke("autovacuum_reference")
      .then((reference) => {
        autovacuumReference = reference;
        renderAutovacuumReference();
        return reference;
      })
      .catch((error) => {
        const text = `Не удалось загрузить Autovacuum reference: ${String(error)}`;
        for (const id of ["autovacuum-methodology-content", "autovacuum-query-content", "autovacuum-source-list"]) {
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
        autovacuumReferencePromise = null;
      });

  return autovacuumReferencePromise;
}

function autovacuumHistoryConnectionLabel(connectionId, historyRows) {
  const current = connectionsById.get(connectionId);
  if (current) {
    return `${current.name} — ${current.username}@${current.host}:${current.port}/${current.database}`;
  }
  const latest = historyRows.find((row) => row.connection_id === connectionId);
  if (!latest) {
    return `Подключение #${connectionId}`;
  }
  return `${latest.connection_name} — ${latest.username}@${latest.host}:${latest.port}/${latest.database} · сохранённая история`;
}

function renderAutovacuumHistoryConnectionOptions(historyRows) {
  const select = byId("autovacuum-history-connection");
  const previous = select.value;
  const ids = new Set(Array.from(connectionsById.keys()));
  for (const row of historyRows) ids.add(row.connection_id);
  select.replaceChildren();
  const sorted = Array.from(ids).sort((a, b) =>
      autovacuumHistoryConnectionLabel(a, historyRows).localeCompare(autovacuumHistoryConnectionLabel(b, historyRows), "ru"),
  );
  if (sorted.length === 0) {
    const option = document.createElement("option");
    option.value = "";
    option.textContent = "Нет подключений и Autovacuum history";
    select.append(option);
    select.disabled = true;
    byId("refresh-autovacuum-history").disabled = true;
    return;
  }
  select.disabled = false;
  byId("refresh-autovacuum-history").disabled = false;
  for (const id of sorted) {
    const option = document.createElement("option");
    option.value = String(id);
    option.textContent = autovacuumHistoryConnectionLabel(id, historyRows);
    select.append(option);
  }
  if (previous && sorted.some((id) => String(id) === previous)) select.value = previous;
}

function resetAutovacuumComparison() {
  autovacuumComparisonRows = [];
  autovacuumComparisonPage = 1;
  byId("autovacuum-comparison-content").hidden = true;
  byId("autovacuum-comparison-error").hidden = true;
  byId("autovacuum-comparison-error").textContent = "";
  byId("autovacuum-compare-table-list").replaceChildren();
  renderAutovacuumPagers(
      "comparison",
      0,
      1,
      autovacuumComparisonPageSize,
  );
}

function renderAutovacuumHistoryComparisonOptions(snapshots) {
  const first = byId("autovacuum-compare-first");
  const second = byId("autovacuum-compare-second");
  first.replaceChildren();
  second.replaceChildren();
  for (const snapshot of snapshots) {
    const text = `#${snapshot.id} · ${formatSnapshotTime(snapshot.collection_started_at_ms)} · dead ${formatInteger(snapshot.total_dead_tuples)}`;
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
  byId("compare-autovacuum").disabled = !canCompare;
  if (canCompare) {
    first.value = String(snapshots[1].id);
    second.value = String(snapshots[0].id);
  }
}

function renderAutovacuumHistoryPage() {
  const body = byId("autovacuum-history-list");
  const range = autovacuumPageWindow(
      autovacuumHistorySnapshots.length,
      autovacuumHistoryPage,
      autovacuumHistoryPageSize,
  );

  autovacuumHistoryPage = range.page;
  body.replaceChildren();

  for (const snapshot of autovacuumHistorySnapshots.slice(range.start, range.end)) {
    const row = document.createElement("tr");
    appendCells(row, [
      `#${snapshot.id}`,
      formatSnapshotTime(snapshot.collection_started_at_ms),
      `${snapshot.host}:${snapshot.port}/${snapshot.database}`,
      formatInteger(snapshot.table_count),
      formatInteger(snapshot.total_dead_tuples),
      formatInteger(snapshot.dead_trigger_reached_count),
      formatInteger(snapshot.insert_trigger_reached_count),
      formatInteger(snapshot.analyze_trigger_reached_count),
      `${formatInteger(snapshot.active_vacuum_count)} / ${formatInteger(snapshot.active_analyze_count)}`,
      formatTimestamp(snapshot.stats_reset),
      formatInteger(snapshot.unavailable_sections),
    ]);
    body.append(row);
  }

  renderAutovacuumPagers(
      "history",
      autovacuumHistorySnapshots.length,
      autovacuumHistoryPage,
      autovacuumHistoryPageSize,
  );
}

function renderAutovacuumHistory(snapshots) {
  autovacuumHistorySnapshots = snapshots;
  const hasSnapshots = snapshots.length > 0;
  byId("autovacuum-history-empty").hidden = hasSnapshots;
  byId("autovacuum-history-content").hidden = !hasSnapshots;
  resetAutovacuumComparison();
  renderAutovacuumHistoryComparisonOptions(snapshots);

  if (!hasSnapshots) {
    byId("autovacuum-history-list").replaceChildren();
    renderAutovacuumPagers("history", 0, 1, autovacuumHistoryPageSize);
    return;
  }

  renderAutovacuumHistoryPage();
}

async function refreshAutovacuumHistory() {
  const index = await invoke("list_autovacuum_snapshots", {connectionId: null, limit: 500});
  renderAutovacuumHistoryConnectionOptions(index);
  const connectionId = Number(byId("autovacuum-history-connection").value);
  if (!Number.isInteger(connectionId) || connectionId <= 0) {
    renderAutovacuumHistory([]);
    return;
  }
  const snapshots = await invoke("list_autovacuum_snapshots", {connectionId, limit: 500});
  renderAutovacuumHistory(snapshots);
}

function renderAutovacuumComparisonPage() {
  const body = byId("autovacuum-compare-table-list");
  const range = autovacuumPageWindow(
      autovacuumComparisonRows.length,
      autovacuumComparisonPage,
      autovacuumComparisonPageSize,
  );

  autovacuumComparisonPage = range.page;
  body.replaceChildren();

  if (autovacuumComparisonRows.length === 0) {
    tableEmpty(body, 13, "Нет comparable table rows.");
    renderAutovacuumPagers(
        "comparison",
        0,
        1,
        autovacuumComparisonPageSize,
    );
    return;
  }

  for (const item of autovacuumComparisonRows.slice(range.start, range.end)) {
    const row = document.createElement("tr");
    const vacuumDelta = (item.vacuum_count_delta ?? 0) + (item.autovacuum_count_delta ?? 0);
    const analyzeDelta = (item.analyze_count_delta ?? 0) + (item.autoanalyze_count_delta ?? 0);
    appendCells(row, [
      item.newer_name ?? item.older_name ?? `OID ${item.relation_oid}`,
      autovacuumProfileLabel(item.interval_workload_profile ?? item.newer_workload_profile ?? item.older_workload_profile),
      `${formatSignedInteger(item.inserts_delta)} / ${formatSignedInteger(item.updates_delta)} / ${formatSignedInteger(item.deletes_delta)}`,
      formatSignedInteger(item.hot_updates_delta),
      `${formatInteger(item.older_dead_tuples)} → ${formatInteger(item.newer_dead_tuples)}`,
      item.older_toast_dead_tuples === null && item.newer_toast_dead_tuples === null
          ? "—"
          : `${formatInteger(item.older_toast_dead_tuples)} → ${formatInteger(item.newer_toast_dead_tuples)} / ${formatSignedInteger(item.toast_autovacuum_count_delta)}`,
      item.dead_growth_per_day === null ? "—" : formatNumber(item.dead_growth_per_day, 0),
      item.estimated_days_to_dead_trigger === null ? "—" : formatNumber(item.estimated_days_to_dead_trigger, 2),
      `${formatPercent(item.older_dead_trigger_used_pct)} → ${formatPercent(item.newer_dead_trigger_used_pct)}`,
      item.vacuum_count_delta === null && item.autovacuum_count_delta === null ? "—" : `+${formatInteger(vacuumDelta)}`,
      item.analyze_count_delta === null && item.autoanalyze_count_delta === null ? "—" : `+${formatInteger(analyzeDelta)}`,
      formatSignedBytes(item.total_size_delta_bytes),
      relationSampleStateLabel(item.sample_state),
    ]);
    body.append(row);
  }

  renderAutovacuumPagers(
      "comparison",
      autovacuumComparisonRows.length,
      autovacuumComparisonPage,
      autovacuumComparisonPageSize,
  );
}

function renderAutovacuumComparison(comparison) {
  byId("autovacuum-compare-interval").textContent = formatDuration(comparison.elapsed_seconds);
  byId("autovacuum-compare-stats-period").textContent = comparison.stats_period_continuous ? "непрерывный" : "reset / разные periods";
  byId("autovacuum-compare-total-dead").textContent =
      `${formatInteger(comparison.older.total_dead_tuples)} → ${formatInteger(comparison.newer.total_dead_tuples)}`;
  byId("autovacuum-compare-dead-reached").textContent =
      `${formatInteger(comparison.older.dead_trigger_reached_count)} → ${formatInteger(comparison.newer.dead_trigger_reached_count)}`;
  const vacuumCycles = (comparison.table_changes ?? []).filter((row) => row.vacuum_cycle_observed).length;
  const analyzeCycles = (comparison.table_changes ?? []).filter((row) => row.analyze_cycle_observed).length;
  byId("autovacuum-compare-vacuum-cycles").textContent = formatInteger(vacuumCycles);
  byId("autovacuum-compare-analyze-cycles").textContent = formatInteger(analyzeCycles);
  byId("autovacuum-compare-meta").textContent =
      `#${comparison.older.id} (${formatSnapshotTime(comparison.older.collection_started_at_ms)}) → #${comparison.newer.id} (${formatSnapshotTime(comparison.newer.collection_started_at_ms)}) · ${comparison.newer.host}:${comparison.newer.port}/${comparison.newer.database}`;

  renderAutovacuumInterpretationCards(
      "autovacuum-comparison-interpretation-list",
      comparison.interpretation,
  );

  autovacuumComparisonRows = comparison.table_changes ?? [];
  autovacuumComparisonPage = 1;
  renderAutovacuumComparisonPage();

  const limitations = byId("autovacuum-compare-limitations");
  limitations.replaceChildren();
  for (const value of comparison.limitations ?? []) {
    const li = document.createElement("li"); li.textContent = value; limitations.append(li);
  }
  byId("autovacuum-comparison-error").hidden = true;
  byId("autovacuum-comparison-content").hidden = false;
}

function initAutovacuumUi() {
  byId("collect-autovacuum").addEventListener("click", () => {
    const connectionId = Number(byId("autovacuum-connection").value);
    if (!Number.isInteger(connectionId) || connectionId <= 0) {
      message("Сначала сохраните и выберите подключение PostgreSQL.", true);
      return;
    }

    void action(async () => {
      const relationOid = selectedAutovacuumRelationOid;
      const topError = byId("autovacuum-collect-error");
      topError.hidden = true;
      try {
        const result = await invoke("collect_autovacuum_snapshot", {connectionId});
        renderAutovacuumSnapshot(result.snapshot, result.history);

        let relationRefreshError = null;
        if (Number.isInteger(relationOid) && relationOid > 0) {
          try {
            await loadAutovacuumTableDetail(
                relationOid,
                {scrollToDetail: false},
            );
          } catch (error) {
            relationRefreshError = String(error);
          }
        }

        void refreshAutovacuumHistory();
        if (relationRefreshError) {
          message(
              `Autovacuum snapshot #${result.history.id} сохранён, но relation detail не обновился: ${relationRefreshError}`,
              true,
          );
        } else {
          message(`Autovacuum snapshot #${result.history.id} собран и сохранён.`);
        }
      } catch (error) {
        topError.textContent = `Не удалось собрать Autovacuum: ${String(error)}. Повтори проверку и посмотри права подключения и нагрузку PostgreSQL. При повторении ошибки сообщи о ней через Issues репозитория pgXray (без паролей).`;
        topError.hidden = false;
        throw error;
      }
    });
  });

  byId("autovacuum-connection").addEventListener("change", () => {
    resetAutovacuumSnapshot();
  });

  byId("autovacuum-table-list").addEventListener("click", (event) => {
    const button = event.target.closest("button[data-autovacuum-relation-oid]");
    if (!button) return;
    const relationOid = Number(button.dataset.autovacuumRelationOid);
    void action(
        async () => loadAutovacuumTableDetail(relationOid),
        {startMessage: null},
    );
  });

  byId("autovacuum-finding-list").addEventListener("click", (event) => {
    const button = event.target.closest("button[data-autovacuum-relation-oid]");
    if (!button) return;
    const relationOid = Number(button.dataset.autovacuumRelationOid);
    void action(
        async () => loadAutovacuumTableDetail(relationOid),
        {startMessage: null},
    );
  });

  for (const id of ["autovacuum-table-filter", "autovacuum-table-mode"]) {
    byId(id).addEventListener(id.endsWith("filter") ? "input" : "change", () => {
      autovacuumTablePage = 1;
      renderAutovacuumTables(autovacuumSnapshot?.tables);
    });
  }

  byId("autovacuum-advisor-findings").addEventListener("click", (event) => {
    const button = event.target.closest("button[data-autovacuum-open-manual-calculator]");
    if (!button) return;
    const relationOid = Number(button.dataset.autovacuumOpenManualCalculator);
    renderAutovacuumPolicyRelations(autovacuumSnapshot?.tables?.data ?? []);
    const select = byId("autovacuum-policy-relation");
    if (Number.isInteger(relationOid) && relationOid > 0) {
      select.value = String(relationOid);
    }
    setAutovacuumTab("policy");
    byId("autovacuum-policy-target").focus();
  });

  byId("simulate-autovacuum-policy").addEventListener("click", () => {
    const connectionId = Number(byId("autovacuum-connection").value);
    const relationOid = Number(byId("autovacuum-policy-relation").value);
    const targetDeadTuples = Number(byId("autovacuum-policy-target").value);
    const error = byId("autovacuum-policy-error");
    error.hidden = true;
    error.textContent = "";
    byId("autovacuum-policy-result").hidden = true;

    if (!Number.isInteger(connectionId) || connectionId <= 0 || !Number.isInteger(relationOid) || relationOid <= 0) {
      error.hidden = false;
      error.textContent = "Сначала соберите snapshot и выберите relation.";
      return;
    }
    if (!Number.isInteger(targetDeadTuples) || targetDeadTuples <= 0) {
      error.hidden = false;
      error.textContent = "Target dead tuples должен быть положительным целым числом.";
      return;
    }

    void action(
        async () => {
          const result = await invoke("simulate_autovacuum_policy", {
            connectionId,
            relationOid,
            targetDeadTuples,
          });
          renderAutovacuumPolicyResult(result);
        },
        {
          startMessage: null,
          onError: (text) => {
            error.hidden = false;
            error.textContent = `Calculator недоступен: ${text}`;
          },
        },
    );
  });

  byId("refresh-autovacuum-history").addEventListener("click", () => {
    void action(async () => {
      await refreshAutovacuumHistory();
      message("История Autovacuum обновлена.");
    });
  });

  byId("autovacuum-history-connection").addEventListener("change", () => {
    autovacuumHistoryPage = 1;

    void action(async () => {
      await refreshAutovacuumHistory();
      message("История Autovacuum загружена.");
    });
  });

  for (const panelId of ["autovacuum-diagnostic", "autovacuum-history-panel"]) {
    byId(panelId).addEventListener("click", (event) => {
      const button = event.target.closest(
          "button[data-autovacuum-pager-action]",
      );

      if (!button) {
        return;
      }

      const kind = button.dataset.autovacuumPagerKind;
      const direction =
          button.dataset.autovacuumPagerAction === "next" ? 1 : -1;

      if (kind === "table") {
        autovacuumTablePage += direction;
        renderAutovacuumTables(autovacuumSnapshot?.tables);
      } else if (kind === "history") {
        autovacuumHistoryPage += direction;
        renderAutovacuumHistoryPage();
      } else if (kind === "comparison") {
        autovacuumComparisonPage += direction;
        renderAutovacuumComparisonPage();
      } else {
        return;
      }

      scrollAutovacuumPagerBlock(kind);
    });

    byId(panelId).addEventListener("change", (event) => {
      const select = event.target.closest(
          "select[data-autovacuum-pager-size]",
      );

      if (!select) {
        return;
      }

      const size = Number(select.value);
      if (!AUTOVACUUM_PAGE_SIZES.includes(size)) {
        return;
      }

      const kind = select.dataset.autovacuumPagerSize;

      if (kind === "table") {
        autovacuumTablePageSize = size;
        autovacuumTablePage = 1;
        renderAutovacuumTables(autovacuumSnapshot?.tables);
      } else if (kind === "history") {
        autovacuumHistoryPageSize = size;
        autovacuumHistoryPage = 1;
        renderAutovacuumHistoryPage();
      } else if (kind === "comparison") {
        autovacuumComparisonPageSize = size;
        autovacuumComparisonPage = 1;
        renderAutovacuumComparisonPage();
      } else {
        return;
      }

      scrollAutovacuumPagerBlock(kind);
    });
  }

  byId("compare-autovacuum").addEventListener("click", () => {
    const firstId = Number(byId("autovacuum-compare-first").value);
    const secondId = Number(byId("autovacuum-compare-second").value);
    const error = byId("autovacuum-comparison-error");
    if (!Number.isInteger(firstId) || !Number.isInteger(secondId) || firstId <= 0 || secondId <= 0) return;
    if (firstId === secondId) {
      error.hidden = false;
      error.textContent = "Выберите два разных snapshots.";
      return;
    }
    resetAutovacuumComparison();
    void action(
        async () => {
          const comparison = await invoke("compare_autovacuum_snapshots", {firstId, secondId});
          renderAutovacuumComparison(comparison);
        },
        {
          startMessage: null,
          onError: (text) => {
            error.hidden = false;
            error.textContent = `Не удалось сравнить Autovacuum snapshots: ${text}`;
          },
        },
    );
  });
  document.querySelectorAll(".autovacuum-tab").forEach((button) => {
    button.addEventListener("click", () => {
      const name = button.dataset.autovacuumTab;

      setAutovacuumTab(name);

      if (name === "methodology" || name === "queries") {
        void loadAutovacuumReference();
      }
    });
  });
}
