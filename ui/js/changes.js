"use strict";

function localDateTimeValue(milliseconds) {
  const date = new Date(milliseconds);
  const local = new Date(
      milliseconds - date.getTimezoneOffset() * 60_000,
  );
  return local.toISOString().slice(0, 16);
}

function parseLocalDateTime(id) {
  const value = byId(id).value;
  if (!value) {
    throw new Error("Заполните все границы baseline/incident periods.");
  }
  const timestamp = new Date(value).getTime();
  if (!Number.isFinite(timestamp) || timestamp <= 0) {
    throw new Error(`Некорректное время: ${value}`);
  }
  return timestamp;
}

function whatChangedPeriods() {
  const baselineFromMs = parseLocalDateTime("changes-baseline-from");
  const baselineToMs = parseLocalDateTime("changes-baseline-to");
  const incidentFromMs = parseLocalDateTime("changes-incident-from");
  const incidentToMs = parseLocalDateTime("changes-incident-to");

  if (baselineFromMs >= baselineToMs) {
    throw new Error("Baseline: начало должно быть раньше конца.");
  }
  if (incidentFromMs >= incidentToMs) {
    throw new Error("Incident: начало должно быть раньше конца.");
  }
  if (baselineToMs > incidentFromMs) {
    throw new Error("Baseline и incident periods не должны перекрываться.");
  }

  return {
    baselineFromMs,
    baselineToMs,
    incidentFromMs,
    incidentToMs,
  };
}

function initializeWhatChangedPeriods() {
  if (byId("changes-baseline-from").value) {
    return;
  }
  const now = Date.now();
  const hour = 60 * 60 * 1000;

  byId("changes-baseline-from").value = localDateTimeValue(now - 3 * hour);
  byId("changes-baseline-to").value = localDateTimeValue(now - 2 * hour);
  byId("changes-incident-from").value = localDateTimeValue(now - hour);
  byId("changes-incident-to").value = localDateTimeValue(now);
  byId("changes-marker-time").value = localDateTimeValue(now - 30 * 60 * 1000);
}

function selectedWhatChangedConnectionId() {
  const connectionId = Number(byId("changes-connection").value);
  if (!Number.isInteger(connectionId) || connectionId <= 0) {
    throw new Error("Выберите подключение с diagnostic history.");
  }
  return connectionId;
}

function whatChangedConnectionLabel(connectionId, historyRows) {
  const connection = connectionsById.get(connectionId);
  if (connection) {
    return `${connection.name} — ${connection.username}@${connection.host}:${connection.port}/${connection.database}`;
  }
  const history = historyRows.find((row) => row.connection_id === connectionId);
  return history
    ? `${history.connection_name} · сохранённая history · connection #${connectionId}`
    : `Connection #${connectionId}`;
}

async function refreshWhatChangedConnectionOptions() {
  const historyRows = await invoke("list_history_connections");
  whatChangedHistoryConnections = historyRows;
  const select = byId("changes-connection");
  const previous = select.value;
  const ids = new Set(Array.from(connectionsById.keys()));
  for (const row of historyRows) {
    ids.add(row.connection_id);
  }
  const sorted = Array.from(ids).sort((left, right) => {
    return whatChangedConnectionLabel(left, historyRows).localeCompare(
        whatChangedConnectionLabel(right, historyRows),
        "ru",
    );
  });

  select.replaceChildren();
  if (sorted.length === 0) {
    const option = document.createElement("option");
    option.value = "";
    option.textContent = "Нет diagnostic history";
    select.append(option);
    select.disabled = true;
    byId("compare-what-changed").disabled = true;
    return;
  }

  select.disabled = false;
  byId("compare-what-changed").disabled = false;
  for (const connectionId of sorted) {
    const option = document.createElement("option");
    option.value = String(connectionId);
    option.textContent = whatChangedConnectionLabel(connectionId, historyRows);
    select.append(option);
  }
  if (previous && sorted.some((id) => String(id) === previous)) {
    select.value = previous;
  }
}

async function refreshWhatChangedCases() {
  const select = byId("changes-case");
  select.replaceChildren();

  let connectionId;
  try {
    connectionId = selectedWhatChangedConnectionId();
  } catch (_) {
    select.disabled = true;
    byId("save-what-changed").disabled = true;
    byId("add-change-marker").disabled = true;
    return;
  }

  const hasLiveConnection = connectionsById.has(connectionId);
  byId("save-what-changed").disabled = !hasLiveConnection;
  byId("add-change-marker").disabled = !hasLiveConnection;

  const cases = await invoke(
      "list_investigation_cases",
      {connectionId},
  );
  investigationCases = [...cases].sort(
      (left, right) => Number(right.created_at_ms) - Number(left.created_at_ms),
  );

  const automatic = document.createElement("option");
  automatic.value = "";
  automatic.textContent = hasLiveConnection
    ? "Создать incident Case автоматически"
    : "Для Save to Case нужно восстановить сохранённое connection";
  select.append(automatic);

  for (const item of investigationCases) {
    const option = document.createElement("option");
    option.value = String(item.id);
    option.textContent = `#${item.id} · ${item.title} · ${item.kind}`;
    select.append(option);
  }
  select.disabled = false;
}

function renderWhatChangedMarkers(markers) {
  whatChangedMarkers = markers;
  const target = byId("changes-marker-list");
  target.replaceChildren();

  if (markers.length === 0) {
    const empty = document.createElement("p");
    empty.className = "empty-state";
    empty.textContent = "В выбранном временном диапазоне manual markers нет.";
    target.append(empty);
    return;
  }

  for (const marker of markers) {
    const row = document.createElement("article");
    const meta = document.createElement("div");
    const content = document.createElement("div");
    const title = document.createElement("strong");
    const note = document.createElement("p");
    const remove = document.createElement("button");

    row.className = "changes-marker-item";
    meta.className = "changes-marker-meta";
    meta.textContent = `${formatSnapshotTime(marker.occurred_at_ms)} · ${marker.kind}`;
    title.textContent = marker.title;
    note.textContent = marker.note || "Без дополнительного описания.";
    content.append(title, note);
    remove.type = "button";
    remove.className = "secondary";
    remove.textContent = "Удалить";
    remove.dataset.changeMarkerId = String(marker.id);
    row.append(meta, content, remove);
    target.append(row);
  }
}

async function refreshWhatChangedMarkers() {
  let connectionId;
  try {
    connectionId = selectedWhatChangedConnectionId();
  } catch (_) {
    renderWhatChangedMarkers([]);
    return;
  }

  let fromMs = null;
  let toMs = null;
  try {
    const periods = whatChangedPeriods();
    fromMs = periods.baselineFromMs;
    toMs = periods.incidentToMs;
  } catch (_) {
    // The marker editor can still work before valid periods are selected.
  }

  const markers = await invoke(
      "list_change_markers",
      {connectionId, fromMs, toMs},
  );
  renderWhatChangedMarkers(markers);
}

function whatChangedQualityLabel(quality) {
  if (!quality) {
    return "—";
  }
  const coverage = quality.safe_time_coverage_pct === null
    ? "coverage —"
    : `${formatNumber(quality.safe_time_coverage_pct, 1)}% safe time`;
  return `${quality.state} · ${quality.snapshot_count} snapshots · ${quality.safe_interval_count} safe · ${coverage}`;
}

function whatChangedStatementStateLabel(state) {
  if (state === "sampled_in_both") return "sampled в обоих periods";
  if (state === "sampled_in_incident_only") return "sampled только incident";
  if (state === "sampled_in_baseline_only") return "не sampled в incident";
  return state;
}

function metricPair(baseline, incident, key, formatter) {
  const older = baseline?.[key];
  const newer = incident?.[key];
  if (older === null || older === undefined || newer === null || newer === undefined) {
    if (older !== null && older !== undefined) return `${formatter(older)} → —`;
    if (newer !== null && newer !== undefined) return `— → ${formatter(newer)}`;
    return "—";
  }
  return `${formatter(older)} → ${formatter(newer)}`;
}

function renderWhatChangedPgss(result) {
  const body = byId("changes-pgss-list");
  body.replaceChildren();
  const changes = result.pgss?.statement_changes ?? [];

  if (changes.length === 0) {
    tableEmpty(body, 10, "Нет sampled statement changes для выбранных periods.");
    return;
  }

  for (const change of changes) {
    const row = document.createElement("tr");
    const identity = `${change.queryid} · ${change.role_name ?? "<unknown>"}/${change.database ?? "<unknown>"}${change.toplevel ? " · top" : " · nested"}`;
    appendCells(row, [
      identity,
      whatChangedStatementStateLabel(change.state),
      metricPair(change.baseline, change.incident, "calls_per_hour", (value) => formatNumber(value, 1)),
      metricPair(change.baseline, change.incident, "exec_ms_per_call", (value) => `${formatNumber(value, 3)} ms`),
      metricPair(change.baseline, change.incident, "total_exec_time_per_hour_ms", (value) => `${formatNumber(value / 1000, 1)} s/h`),
      metricPair(change.baseline, change.incident, "shared_reads_per_call", (value) => formatNumber(value, 2)),
      metricPair(change.baseline, change.incident, "temp_writes_per_call", (value) => formatNumber(value, 2)),
      metricPair(change.baseline, change.incident, "wal_bytes_per_call", (value) => formatBytes(value)),
      metricPair(change.baseline, change.incident, "exec_workload_share_pct", (value) => formatPercent(value)),
      change.evidence_level,
    ]);
    body.append(row);
  }
}

function settingValue(value, unit) {
  if (value === null || value === undefined) return "<missing>";
  return unit ? `${value} ${unit}` : String(value);
}

function renderWhatChangedSettings(result) {
  const body = byId("changes-settings-list");
  body.replaceChildren();
  const changes = result.settings?.changes ?? [];

  if (changes.length === 0) {
    tableEmpty(body, 6, "Между selected settings snapshots изменений не найдено или данных недостаточно.");
    return;
  }

  for (const item of changes) {
    const change = item.change;
    const row = document.createElement("tr");
    const source = `${change.older_source ?? "—"} → ${change.newer_source ?? "—"}`;
    const restart = `${yesNo(change.older_pending_restart)} → ${yesNo(change.newer_pending_restart)}`;
    appendCells(row, [
      change.name,
      settingValue(change.older_value, change.older_unit),
      settingValue(change.newer_value, change.newer_unit),
      source,
      restart,
      item.first_observed_at_ms === null
        ? "не локализовано"
        : formatSnapshotTime(item.first_observed_at_ms),
    ]);
    body.append(row);
  }
}

function renderWhatChangedTimeline(result) {
  const target = byId("changes-timeline");
  target.replaceChildren();
  const events = result.timeline ?? [];

  if (events.length === 0) {
    const empty = document.createElement("p");
    empty.className = "empty-state";
    empty.textContent = "Timeline events для выбранного диапазона не найдены.";
    target.append(empty);
    return;
  }

  for (const event of events) {
    const row = document.createElement("article");
    const meta = document.createElement("div");
    const content = document.createElement("div");
    const title = document.createElement("strong");
    const badge = document.createElement("span");
    const detail = document.createElement("p");
    const source = document.createElement("span");

    row.className = "changes-timeline-item";
    row.dataset.phase = event.phase;
    meta.className = "changes-timeline-meta";
    meta.textContent = `${formatSnapshotTime(event.occurred_at_ms)} · ${event.phase}`;
    title.textContent = event.title;
    badge.className = "changes-evidence-badge";
    badge.textContent = event.evidence_level;
    detail.textContent = event.detail || "—";
    content.append(title, badge, detail);
    source.className = "changes-timeline-meta";
    source.textContent = event.source;
    row.append(meta, content, source);
    target.append(row);
  }
}

function renderLimitations(id, values) {
  const list = byId(id);
  list.replaceChildren();
  const items = values ?? [];
  if (items.length === 0) {
    const row = document.createElement("li");
    row.textContent = "Дополнительных ограничений нет.";
    list.append(row);
    return;
  }
  for (const value of items) {
    const row = document.createElement("li");
    row.textContent = value;
    list.append(row);
  }
}


function whatChangedMaintenanceQualityLabel(quality) {
  if (!quality) return "—";
  return `${quality.state} · ${quality.snapshot_count} snapshots · ${quality.safe_interval_count} safe`;
}

function whatChangedXidQualityLabel(period) {
  if (!period) return "—";
  const age = period.latest_database_xid_age === null || period.latest_database_xid_age === undefined
    ? "db age —"
    : `db age ${formatInteger(period.latest_database_xid_age)}`;
  const rate = period.xid_per_hour === null || period.xid_per_hour === undefined
    ? "rate —"
    : `${formatNumber(period.xid_per_hour, 1)} xid/h`;
  return `${period.state} · ${period.snapshot_count} snapshots · ${age} · ${rate}`;
}

function autovacuumStateLabel(state) {
  if (state === "observed_in_both") return "observed в обоих periods";
  if (state === "observed_in_incident_only") return "observed только incident";
  if (state === "observed_in_baseline_only") return "не observed в incident";
  return state;
}

function optionalPair(baseline, incident, key, formatter) {
  return metricPair(baseline, incident, key, formatter);
}

function renderWhatChangedAutovacuum(result) {
  const body = byId("changes-autovacuum-list");
  body.replaceChildren();
  const changes = result.autovacuum?.relation_changes ?? [];

  if (changes.length === 0) {
    tableEmpty(body, 10, "Нет Autovacuum/Table Health evidence для selected periods.");
    return;
  }

  for (const change of changes) {
    const row = document.createElement("tr");
    const baseline = change.baseline;
    const incident = change.incident;
    const dml = [
      optionalPair(baseline, incident, "inserts_per_hour", (value) => formatNumber(value, 1)),
      optionalPair(baseline, incident, "updates_per_hour", (value) => formatNumber(value, 1)),
      optionalPair(baseline, incident, "deletes_per_hour", (value) => formatNumber(value, 1)),
    ].join(" / ");
    const cycles = `${optionalPair(baseline, incident, "vacuum_cycles", (value) => formatInteger(value))} · ${optionalPair(baseline, incident, "analyze_cycles", (value) => formatInteger(value))}`;
    const policy = change.policy_changed
      ? change.policy_changes.join("; ")
      : "без observed policy change";

    appendCells(row, [
      change.relation_name,
      autovacuumStateLabel(change.state),
      dml,
      optionalPair(baseline, incident, "hot_update_pct", (value) => formatPercent(value)),
      optionalPair(baseline, incident, "latest_dead_tuples", (value) => formatInteger(value)),
      optionalPair(baseline, incident, "latest_dead_trigger_used_pct", (value) => formatPercent(value)),
      cycles,
      optionalPair(baseline, incident, "latest_total_size_bytes", (value) => formatBytes(value)),
      policy,
      change.evidence_level,
    ]);
    body.append(row);
  }
}

function renderWhatChangedXid(result) {
  const body = byId("changes-xid-list");
  body.replaceChildren();
  const changes = result.xid?.relation_changes ?? [];

  if (changes.length === 0) {
    tableEmpty(body, 8, "Нет XID relation evidence для selected periods.");
    return;
  }

  for (const change of changes) {
    const row = document.createElement("tr");
    const baseline = change.baseline;
    const incident = change.incident;
    appendCells(row, [
      change.relation_name,
      autovacuumStateLabel(change.state),
      optionalPair(baseline, incident, "latest_xid_age", (value) => formatInteger(value)),
      optionalPair(baseline, incident, "latest_pct_of_freeze_max", (value) => formatPercent(value)),
      optionalPair(baseline, incident, "xid_age_delta", (value) => formatSignedInteger(value)),
      optionalPair(baseline, incident, "latest_effective_freeze_max_age", (value) => formatInteger(value)),
      optionalPair(baseline, incident, "latest_freeze_max_source", (value) => String(value)),
      change.evidence_level,
    ]);
    body.append(row);
  }
}


function whatChangedStorageQualityLabel(quality) {
  if (!quality) return "—";
  return `${quality.state} · ${quality.snapshot_count} snapshots · DB ${quality.database_safe_intervals} / WAL ${quality.wal_safe_intervals} / CP ${quality.checkpoint_safe_intervals} safe`;
}

function changesSignedBytes(value) {
  if (value === null || value === undefined) return "—";
  if (value === 0) return "0 B";
  return `${value > 0 ? "+" : "−"}${formatBytes(Math.abs(value))}`;
}

function renderWhatChangedStorage(result) {
  const body = byId("changes-storage-metrics");
  body.replaceChildren();
  const baseline = result.storage?.baseline ?? {};
  const incident = result.storage?.incident ?? {};
  const rows = [
    ["Temp bytes / hour", "temp_bytes_per_hour", (value) => formatBytes(value)],
    ["Temp files / hour", "temp_files_per_hour", (value) => formatNumber(value, 2)],
    ["Database blocks read / hour", "database_blocks_read_per_hour", (value) => formatNumber(value, 1)],
    ["Database blocks hit / hour", "database_blocks_hit_per_hour", (value) => formatNumber(value, 1)],
    ["WAL bytes / hour", "wal_bytes_per_hour", (value) => formatBytes(value)],
    ["WAL records / hour", "wal_records_per_hour", (value) => formatNumber(value, 1)],
    ["WAL buffers full / hour", "wal_buffers_full_per_hour", (value) => formatNumber(value, 2)],
    ["WAL write ms / hour", "wal_write_time_ms_per_hour", (value) => `${formatNumber(value, 1)} ms`],
    ["WAL sync ms / hour", "wal_sync_time_ms_per_hour", (value) => `${formatNumber(value, 1)} ms`],
    ["Checkpoints requested / hour", "checkpoints_requested_per_hour", (value) => formatNumber(value, 2)],
    ["Checkpoints timed / hour", "checkpoints_timed_per_hour", (value) => formatNumber(value, 2)],
    ["Checkpoint write ms / hour", "checkpoint_write_time_ms_per_hour", (value) => `${formatNumber(value, 1)} ms`],
    ["Checkpoint sync ms / hour", "checkpoint_sync_time_ms_per_hour", (value) => `${formatNumber(value, 1)} ms`],
    ["Checkpoint buffers written / hour", "checkpoint_buffers_written_per_hour", (value) => formatNumber(value, 1)],
    ["Database size", "latest_database_size_bytes", (value) => formatBytes(value)],
    ["checkpoint_timeout", "checkpoint_timeout", (value) => String(value)],
    ["checkpoint_completion_target", "checkpoint_completion_target", (value) => formatNumber(value, 2)],
    ["min_wal_size", "min_wal_size", (value) => String(value)],
    ["max_wal_size", "max_wal_size", (value) => String(value)],
    ["track_io_timing", "track_io_timing", (value) => yesNo(value)],
    ["track_wal_io_timing", "track_wal_io_timing", (value) => yesNo(value)],
  ];
  for (const [label, key, formatter] of rows) {
    const row = document.createElement("tr");
    appendCells(row, [
      label,
      baseline[key] === null || baseline[key] === undefined ? "—" : formatter(baseline[key]),
      incident[key] === null || incident[key] === undefined ? "—" : formatter(incident[key]),
    ]);
    body.append(row);
  }

  const sizes = byId("changes-size-list");
  sizes.replaceChildren();
  const changes = result.storage?.relation_size_changes ?? [];
  if (changes.length === 0) {
    tableEmpty(sizes, 9, "Нет comparable relation-size changes для selected periods.");
  } else {
    for (const change of changes) {
      const row = document.createElement("tr");
      appendCells(row, [
        change.relation_name,
        change.state,
        change.baseline_total_size_bytes === null ? "—" : formatBytes(change.baseline_total_size_bytes),
        change.incident_total_size_bytes === null ? "—" : formatBytes(change.incident_total_size_bytes),
        changesSignedBytes(change.total_size_delta_bytes),
        changesSignedBytes(change.heap_size_delta_bytes),
        changesSignedBytes(change.indexes_size_delta_bytes),
        changesSignedBytes(change.toast_size_delta_bytes),
        change.evidence_level,
      ]);
      sizes.append(row);
    }
  }
}

function renderWhatChangedIndexHealth(result) {
  const body = byId("changes-index-list");
  body.replaceChildren();
  const changes = result.index_health?.changes ?? [];
  if (changes.length === 0) {
    tableEmpty(body, 8, "Нет Index Health evidence changes для selected periods.");
    return;
  }
  for (const change of changes) {
    const row = document.createElement("tr");
    appendCells(row, [
      change.relation_name,
      change.index_name,
      change.state,
      `${(change.baseline_classifications ?? []).join(", ") || "—"} → ${(change.incident_classifications ?? []).join(", ") || "—"}`,
      change.scan_delta === null ? "—" : formatSignedInteger(change.scan_delta),
      change.relation_dml_delta === null ? "—" : formatSignedInteger(change.relation_dml_delta),
      changesSignedBytes(change.size_delta_bytes),
      change.evidence_level,
    ]);
    body.append(row);
  }
}

function renderWhatChanged(result) {
  whatChangedComparison = result;
  byId("changes-empty").hidden = true;
  byId("changes-error").hidden = true;
  byId("changes-content").hidden = false;

  byId("changes-target").textContent = `${result.target.host}:${result.target.port}/${result.target.database} · oid ${result.target.database_oid}`;
  byId("changes-pgss-baseline-quality").textContent = whatChangedQualityLabel(result.pgss.baseline_quality);
  byId("changes-pgss-incident-quality").textContent = whatChangedQualityLabel(result.pgss.incident_quality);
  byId("changes-settings-quality").textContent = result.settings.state;
  byId("changes-autovacuum-baseline-quality").textContent = whatChangedMaintenanceQualityLabel(result.autovacuum?.baseline_quality);
  byId("changes-autovacuum-incident-quality").textContent = whatChangedMaintenanceQualityLabel(result.autovacuum?.incident_quality);
  byId("changes-xid-baseline-quality").textContent = whatChangedXidQualityLabel(result.xid?.baseline);
  byId("changes-xid-incident-quality").textContent = whatChangedXidQualityLabel(result.xid?.incident);
  byId("changes-storage-baseline-quality").textContent = whatChangedStorageQualityLabel(result.storage?.baseline_quality);
  byId("changes-storage-incident-quality").textContent = whatChangedStorageQualityLabel(result.storage?.incident_quality);
  byId("changes-index-health-quality").textContent = result.index_health?.state ?? "—";
  byId("changes-statement-count").textContent = formatInteger(result.pgss.statement_changes.length);
  byId("changes-setting-count").textContent = formatInteger(result.settings.changes.length);
  byId("changes-autovacuum-count").textContent = formatInteger(result.autovacuum?.relation_changes?.length ?? 0);
  byId("changes-xid-count").textContent = formatInteger(result.xid?.relation_changes?.length ?? 0);
  byId("changes-size-count").textContent = formatInteger(result.storage?.relation_size_changes?.length ?? 0);
  byId("changes-index-count").textContent = formatInteger(result.index_health?.changes?.length ?? 0);
  byId("changes-period-meta").textContent = `Baseline ${formatSnapshotTime(result.baseline.from_ms)} → ${formatSnapshotTime(result.baseline.to_ms)} · Incident ${formatSnapshotTime(result.incident.from_ms)} → ${formatSnapshotTime(result.incident.to_ms)} · PostgreSQL ${result.target.server_version}.`;

  renderWhatChangedTimeline(result);
  renderWhatChangedPgss(result);
  renderWhatChangedSettings(result);
  renderWhatChangedAutovacuum(result);
  renderWhatChangedXid(result);
  renderWhatChangedStorage(result);
  renderWhatChangedIndexHealth(result);
  renderLimitations(
      "changes-pgss-baseline-limitations",
      result.pgss.baseline_quality.limitations,
  );
  renderLimitations(
      "changes-pgss-incident-limitations",
      result.pgss.incident_quality.limitations,
  );
  renderLimitations(
      "changes-autovacuum-limitations",
      result.autovacuum?.limitations ?? [],
  );
  renderLimitations(
      "changes-xid-limitations",
      result.xid?.limitations ?? [],
  );
  renderLimitations(
      "changes-storage-limitations",
      [
        ...(result.storage?.baseline_quality?.limitations ?? []),
        ...(result.storage?.incident_quality?.limitations ?? []),
        ...(result.storage?.limitations ?? []),
      ],
  );
  renderLimitations(
      "changes-index-limitations",
      result.index_health?.limitations ?? [],
  );
  renderLimitations(
      "changes-limitations",
      [
        ...(result.settings.limitations ?? []),
        ...(result.limitations ?? []),
      ],
  );
}

function resetWhatChangedResult() {
  whatChangedComparison = null;
  byId("changes-empty").hidden = false;
  byId("changes-content").hidden = true;
  byId("changes-error").hidden = true;
  byId("changes-error").textContent = "";
}

async function loadWhatChangedComparison() {
  const connectionId = selectedWhatChangedConnectionId();
  const periods = whatChangedPeriods();
  const result = await invoke(
      "compare_what_changed",
      {connectionId, ...periods},
  );
  renderWhatChanged(result);
  await refreshWhatChangedMarkers();
  await refreshWhatChangedCases();
  return result;
}

async function refreshWhatChangedView() {
  initializeWhatChangedPeriods();
  await refreshWhatChangedConnectionOptions();
  await Promise.all([
    refreshWhatChangedMarkers(),
    refreshWhatChangedCases(),
  ]);
}

function initChangesUi() {
  initializeWhatChangedPeriods();

  byId("compare-what-changed").addEventListener("click", () => {
    void action(
        async () => {
          await loadWhatChangedComparison();
          message("What Changed? comparison построен.");
        },
        {
          onError: (errorText) => {
            const error = byId("changes-error");
            error.hidden = false;
            error.textContent = errorText;
            byId("changes-content").hidden = true;
          },
        },
    );
  });

  byId("changes-connection").addEventListener("change", () => {
    resetWhatChangedResult();
    void action(
        async () => {
          await Promise.all([
            refreshWhatChangedMarkers(),
            refreshWhatChangedCases(),
          ]);
        },
        {startMessage: null},
    );
  });

  for (const id of [
    "changes-baseline-from",
    "changes-baseline-to",
    "changes-incident-from",
    "changes-incident-to",
  ]) {
    byId(id).addEventListener("change", () => {
      resetWhatChangedResult();
      void action(refreshWhatChangedMarkers, {startMessage: null});
    });
  }

  byId("add-change-marker").addEventListener("click", () => {
    void action(async () => {
      const connectionId = selectedWhatChangedConnectionId();
      const occurredAtMs = parseLocalDateTime("changes-marker-time");
      const kind = byId("changes-marker-kind").value;
      const title = byId("changes-marker-title").value;
      const note = byId("changes-marker-note").value;

      await invoke(
          "create_change_marker",
          {connectionId, occurredAtMs, kind, title, note},
      );
      byId("changes-marker-title").value = "";
      byId("changes-marker-note").value = "";
      byId("changes-marker-error").hidden = true;
      await refreshWhatChangedMarkers();
      if (whatChangedComparison) {
        await loadWhatChangedComparison();
      }
      message("Manual marker сохранён.");
    }, {
      onError: (errorText) => {
        const error = byId("changes-marker-error");
        error.hidden = false;
        error.textContent = errorText;
      },
    });
  });

  byId("changes-marker-list").addEventListener("click", (event) => {
    const button = event.target.closest("button[data-change-marker-id]");
    if (!button) return;
    const markerId = Number(button.dataset.changeMarkerId);
    if (!Number.isInteger(markerId) || markerId <= 0) return;

    void action(async () => {
      await invoke("delete_change_marker", {connectionId: selectedWhatChangedConnectionId(), markerId});
      await refreshWhatChangedMarkers();
      if (whatChangedComparison) {
        await loadWhatChangedComparison();
      }
      message("Manual marker удалён.");
    });
  });

  byId("save-what-changed").addEventListener("click", () => {
    void action(async () => {
      if (!whatChangedComparison) {
        throw new Error("Сначала выполните What Changed? comparison.");
      }
      const connectionId = selectedWhatChangedConnectionId();
      const periods = whatChangedPeriods();
      const caseValue = byId("changes-case").value;
      const caseId = caseValue ? Number(caseValue) : null;
      const saved = await invoke(
          "save_what_changed_to_case",
          {connectionId, caseId, ...periods},
      );
      byId("changes-save-error").hidden = true;
      await refreshWhatChangedCases();
      message(`What Changed? сохранён в Case #${saved.case.id}, artifact #${saved.artifact.id}.`);
    }, {
      onError: (errorText) => {
        const error = byId("changes-save-error");
        error.hidden = false;
        error.textContent = errorText;
      },
    });
  });
}
