"use strict";

const PG_SETTINGS_AUTO_INTERVAL_MS = 24 * 60 * 60 * 1000;
const PG_SETTINGS_SCHEDULE_CHECK_MS = 60 * 60 * 1000;

let activeMonitoringTab = "activity";
let pgSettingsHistory = [];
let currentPgSettingsSnapshot = null;
let currentPgSettingsComparison = null;
let pgSettingsChangeEvents = [];
let pgSettingsRefreshInFlight = false;
let pgSettingsScheduleInFlight = false;
let pgSettingsScheduleTimer = null;

function setMonitoringTab(name) {
  activeMonitoringTab = name;

  document.querySelectorAll("[data-monitoring-tab]").forEach((button) => {
    button.classList.toggle(
        "active",
        button.dataset.monitoringTab === name,
    );
  });

  byId("monitoring-activity-panel").hidden = name !== "activity";
  byId("monitoring-locks-panel").hidden = name !== "locks";
  byId("monitoring-settings-panel").hidden = name !== "settings";

  if (name !== "activity" && typeof stopActivityMonitoring === "function") {
    stopActivityMonitoring(false);
  }

  if (name !== "locks" && typeof stopLocksMonitoring === "function") {
    stopLocksMonitoring(false);
  }
}

function selectedPgSettingsConnectionId() {
  const value = Number(byId("pg-settings-connection")?.value);
  return Number.isInteger(value) && value > 0 ? value : null;
}

function renderPgSettingsConnectionOptions() {
  const select = byId("pg-settings-connection");
  if (!select) {
    return;
  }

  const previous = select.value;
  const connections = Array.from(connectionsById.values())
      .sort((left, right) => left.name.localeCompare(right.name, "ru"));

  select.replaceChildren();

  for (const connection of connections) {
    const option = document.createElement("option");
    option.value = String(connection.id);
    option.textContent =
        `${connection.name} · ${connection.username}@${connection.host}:${connection.port}/${connection.database}`;
    select.append(option);
  }

  if (
    previous
    && connections.some((connection) => String(connection.id) === previous)
  ) {
    select.value = previous;
  }

  const enabled = connections.length > 0;
  byId("pg-settings-collect").disabled = !enabled;
  byId("pg-settings-refresh").disabled = !enabled;

  if (!enabled) {
    byId("pg-settings-status").textContent =
        "Сначала сохраните PostgreSQL connection.";
  }
}

function pgSettingValue(value, unit) {
  if (value === null || value === undefined) {
    return "—";
  }

  if (!unit) {
    return String(value);
  }

  return /^\d/.test(unit)
      ? `${value} × ${unit}`
      : `${value} ${unit}`;
}

function pgSettingLocation(
    source,
    sourcefile,
    sourceline,
    database = currentPgSettingsSnapshot?.database ?? "текущая БД",
    username = currentPgSettingsSnapshot?.username ?? "текущая роль",
) {
  if (sourcefile) {
    return sourceline ? `${sourcefile}:${sourceline}` : sourcefile;
  }

  return {
    "configuration file": "configuration file · путь скрыт PostgreSQL",
    "command line": "командная строка postgres",
    "environment variable": "окружение процесса postgres",
    global: "ALTER SYSTEM / postgresql.auto.conf",
    database: `ALTER DATABASE ${database}`,
    user: `ALTER ROLE ${username}`,
    "database user": `ALTER ROLE ${username} IN DATABASE ${database}`,
    client: "текущая PostgreSQL session",
    interactive: "interactive SET",
    override: "внутренний override PostgreSQL",
    test: "test override PostgreSQL",
    default: "встроенный default PostgreSQL",
    "dynamic default": "dynamic default PostgreSQL",
  }[source] ?? source ?? "—";
}

function pgSettingIsDefault(setting) {
  return ["default", "dynamic default"].includes(setting.source);
}

function pgSettingsChangedNames() {
  return new Set(
      (currentPgSettingsComparison?.changes ?? [])
          .map((change) => change.name),
  );
}

function pgSettingsSnapshotsSameTarget(older, newer) {
  if (!older || !newer) {
    return false;
  }

  return older.connection_id === newer.connection_id
      && older.host === newer.host
      && Number(older.port) === Number(newer.port)
      && Number(older.database_oid) === Number(newer.database_oid)
      && older.username === newer.username;
}

function renderPgSettingsSummary() {
  const latest = pgSettingsHistory[0] ?? null;

  byId("pg-settings-total").textContent = latest
      ? formatInteger(latest.setting_count)
      : "—";
  byId("pg-settings-non-default").textContent = latest
      ? formatInteger(latest.non_default_count)
      : "—";
  byId("pg-settings-pending-restart").textContent = latest
      ? formatInteger(latest.pending_restart_count)
      : "—";
  byId("pg-settings-changed").textContent = latest
      ? formatInteger(latest.changed_from_previous)
      : "—";

  byId("pg-settings-snapshot-meta").textContent = latest
      ? `Последний snapshot: ${formatSnapshotTime(latest.collection_completed_at_ms)} · PostgreSQL ${latest.server_version}`
      : "Снимков пока нет.";

  const comparison = currentPgSettingsComparison;
  byId("pg-settings-change-window").textContent = comparison
      ? `Последнее сравнение: ${formatSnapshotTime(comparison.older.collection_completed_at_ms)} → ${formatSnapshotTime(comparison.newer.collection_completed_at_ms)}`
      : pgSettingsHistory.length >= 2
          ? "Последний snapshot относится к другому PostgreSQL target; сравнение начнётся со следующего среза этого target."
          : "Для diff нужны минимум два snapshots.";
}

function renderPgSettingsChanges() {
  const body = byId("pg-settings-changes");

  if (pgSettingsHistory.length < 2) {
    tableEmpty(body, 7, "Для сравнения нужны минимум два snapshots.");
    return;
  }

  if (pgSettingsChangeEvents.length === 0) {
    tableEmpty(
        body,
        7,
        "В сохранённой истории изменений настроек не обнаружено.",
    );
    return;
  }

  body.replaceChildren();

  for (const event of pgSettingsChangeEvents) {
    const change = event.change;
    const row = document.createElement("tr");

    const name = document.createElement("td");
    const nameCode = document.createElement("code");
    nameCode.textContent = change.name;
    name.append(nameCode);

    if (change.state !== "changed") {
      const state = document.createElement("small");
      state.className = "pg-settings-change-state";
      state.textContent = change.state === "added" ? "новый параметр" : "параметр исчез";
      name.append(state);
    }

    const older = document.createElement("td");
    older.textContent = pgSettingValue(change.older_value, change.older_unit);

    const newer = document.createElement("td");
    newer.textContent = pgSettingValue(change.newer_value, change.newer_unit);
    if (change.value_changed) {
      newer.classList.add("pg-settings-changed-value");
    }

    const source = document.createElement("td");
    const olderSource = change.older_source ?? "—";
    const newerSource = change.newer_source ?? "—";
    source.textContent = olderSource === newerSource
        ? newerSource
        : `${olderSource} → ${newerSource}`;

    const location = document.createElement("td");
    location.textContent = pgSettingLocation(
        change.newer_source,
        change.newer_sourcefile,
        change.newer_sourceline,
        event.database,
        event.username,
    );
    if (change.source_changed) {
      location.classList.add("pg-settings-changed-value");
    }

    const windowCell = document.createElement("td");
    windowCell.textContent =
        `${formatSnapshotTime(event.changed_after_ms)} → ${formatSnapshotTime(event.changed_before_ms)}`;
    windowCell.title =
        "Точное время внешнего изменения неизвестно: оно произошло между этими двумя snapshots.";

    const restart = document.createElement("td");
    restart.textContent = change.newer_pending_restart === null
        ? "—"
        : yesNo(change.newer_pending_restart);
    if (change.newer_pending_restart) {
      restart.classList.add("pg-settings-restart-required");
    }

    row.append(name, older, newer, source, location, windowCell, restart);
    body.append(row);
  }
}

function pgSettingsFilteredRows() {
  const snapshot = currentPgSettingsSnapshot;
  if (!snapshot) {
    return [];
  }

  const query = byId("pg-settings-search").value.trim().toLocaleLowerCase("ru");
  const filter = byId("pg-settings-filter").value;
  const changed = pgSettingsChangedNames();

  return snapshot.settings.filter((setting) => {
    if (filter === "changed" && !changed.has(setting.name)) {
      return false;
    }
    if (filter === "non_default" && pgSettingIsDefault(setting)) {
      return false;
    }
    if (filter === "pending_restart" && !setting.pending_restart) {
      return false;
    }

    if (!query) {
      return true;
    }

    return [
      setting.name,
      setting.category,
      setting.source,
      setting.context,
      setting.sourcefile,
    ]
        .filter(Boolean)
        .some((value) => String(value).toLocaleLowerCase("ru").includes(query));
  });
}

function renderPgSettingsCurrent() {
  const body = byId("pg-settings-current");
  const rows = pgSettingsFilteredRows();

  if (!currentPgSettingsSnapshot) {
    tableEmpty(body, 7, "Сначала снимите или загрузите pg_settings snapshot.");
    return;
  }

  if (rows.length === 0) {
    tableEmpty(body, 7, "Параметры по выбранному фильтру не найдены.");
    return;
  }

  const changed = pgSettingsChangedNames();
  body.replaceChildren();

  for (const setting of rows) {
    const row = document.createElement("tr");
    if (changed.has(setting.name)) {
      row.classList.add("pg-settings-row-changed");
    }

    const name = document.createElement("td");
    const code = document.createElement("code");
    code.textContent = setting.name;
    name.append(code);

    const value = document.createElement("td");
    value.textContent = pgSettingValue(setting.setting, setting.unit);
    value.title = `reset_val: ${pgSettingValue(setting.reset_val, setting.unit)} · boot_val: ${pgSettingValue(setting.boot_val, setting.unit)}`;

    const category = document.createElement("td");
    category.textContent = setting.category;

    const source = document.createElement("td");
    source.textContent = setting.source;
    if (!pgSettingIsDefault(setting)) {
      source.classList.add("pg-settings-non-default");
    }

    const context = document.createElement("td");
    context.textContent = setting.context;

    const location = document.createElement("td");
    location.textContent = pgSettingLocation(
        setting.source,
        setting.sourcefile,
        setting.sourceline,
    );

    const restart = document.createElement("td");
    restart.textContent = yesNo(setting.pending_restart);
    if (setting.pending_restart) {
      restart.classList.add("pg-settings-restart-required");
    }

    row.append(name, value, category, source, context, location, restart);
    body.append(row);
  }
}

function renderPgSettingsHistory() {
  const body = byId("pg-settings-history");

  if (pgSettingsHistory.length === 0) {
    tableEmpty(body, 5, "pg_settings snapshots пока нет.");
    return;
  }

  body.replaceChildren();

  for (const snapshot of pgSettingsHistory) {
    const row = document.createElement("tr");

    appendCells(row, [
      formatSnapshotTime(snapshot.collection_completed_at_ms),
      snapshot.server_version,
      formatInteger(snapshot.setting_count),
      formatInteger(snapshot.changed_from_previous),
      formatInteger(snapshot.pending_restart_count),
    ]);

    body.append(row);
  }
}

function renderPgSettingsMonitoring() {
  renderPgSettingsSummary();
  renderPgSettingsChanges();
  renderPgSettingsCurrent();
  renderPgSettingsHistory();
}

async function refreshPgSettingsMonitoring() {
  if (pgSettingsRefreshInFlight) {
    return false;
  }

  const connectionId = selectedPgSettingsConnectionId();
  if (connectionId === null) {
    currentPgSettingsSnapshot = null;
    currentPgSettingsComparison = null;
    pgSettingsChangeEvents = [];
    pgSettingsHistory = [];
    renderPgSettingsMonitoring();
    return false;
  }

  pgSettingsRefreshInFlight = true;

  try {
    pgSettingsHistory = await invoke(
        "list_pg_settings_snapshots",
        {
          connectionId,
          limit: 30,
        },
    );

    pgSettingsChangeEvents = await invoke(
        "list_pg_settings_changes",
        {
          connectionId,
          limit: 100,
        },
    );

    if (pgSettingsHistory.length === 0) {
      currentPgSettingsSnapshot = null;
      currentPgSettingsComparison = null;
      renderPgSettingsMonitoring();
      byId("pg-settings-status").className = "inline-status";
      byId("pg-settings-status").textContent =
          "Snapshots ещё нет. Снимите первый snapshot; дальше pgXray будет проверять его раз в 24 часа.";
      return true;
    }

    currentPgSettingsSnapshot = await invoke(
        "load_pg_settings_snapshot",
        { snapshotId: pgSettingsHistory[0].id },
    );

    const previous = pgSettingsHistory[1] ?? null;
    const latest = pgSettingsHistory[0];
    currentPgSettingsComparison = pgSettingsSnapshotsSameTarget(previous, latest)
        ? await invoke(
            "compare_pg_settings_snapshots",
            {
              firstId: previous.id,
              secondId: latest.id,
            },
        )
        : null;

    renderPgSettingsMonitoring();
    byId("pg-settings-status").className = "inline-status success";
    byId("pg-settings-status").textContent =
        `История загружена. Snapshots: ${pgSettingsHistory.length}.`;
    return true;
  } finally {
    pgSettingsRefreshInFlight = false;
  }
}

async function collectPgSettingsNow() {
  const connectionId = selectedPgSettingsConnectionId();
  if (connectionId === null) {
    throw new Error("Выберите PostgreSQL connection.");
  }

  byId("pg-settings-status").className = "inline-status pending";
  byId("pg-settings-status").textContent = "Снимаю pg_settings snapshot…";

  const collected = await invoke(
      "collect_pg_settings_snapshot",
      { connectionId },
  );

  await refreshPgSettingsMonitoring();

  const changes = collected.history.changed_from_previous;
  byId("pg-settings-status").className = "inline-status success";
  byId("pg-settings-status").textContent = changes > 0
      ? `Snapshot сохранён. Изменений с прошлого среза: ${changes}.`
      : "Snapshot сохранён. Изменений с прошлого среза нет.";
}

function pgSettingsSnapshotIsDue(latest, connection, now = Date.now()) {
  if (!latest) {
    return true;
  }

  const sameTarget =
      latest.host === connection.host
      && Number(latest.port) === Number(connection.port)
      && latest.database === connection.database
      && latest.username === connection.username;

  if (!sameTarget) {
    return true;
  }

  return now - Number(latest.collection_completed_at_ms) >= PG_SETTINGS_AUTO_INTERVAL_MS;
}

async function runPgSettingsScheduleCheck() {
  if (pgSettingsScheduleInFlight || !invoke || !currentUsername) {
    return;
  }

  pgSettingsScheduleInFlight = true;
  let selectedUpdated = false;

  try {
    for (const connection of connectionsById.values()) {
      try {
        const snapshots = await invoke(
            "list_pg_settings_snapshots",
            {
              connectionId: connection.id,
              limit: 1,
            },
        );
        const latest = snapshots[0] ?? null;

        if (!pgSettingsSnapshotIsDue(latest, connection)) {
          continue;
        }

        await invoke(
            "collect_pg_settings_snapshot",
            { connectionId: connection.id },
        );

        if (connection.id === selectedPgSettingsConnectionId()) {
          selectedUpdated = true;
        }
      } catch (error) {
        console.warn(
            `Automatic pg_settings snapshot failed for connection #${connection.id}:`,
            error,
        );
      }
    }

    if (selectedUpdated && activeMonitoringTab === "settings") {
      await refreshPgSettingsMonitoring();
    }
  } finally {
    pgSettingsScheduleInFlight = false;
  }
}

function startPgSettingsSchedule() {
  stopPgSettingsSchedule();

  void runPgSettingsScheduleCheck();
  pgSettingsScheduleTimer = window.setInterval(
      () => void runPgSettingsScheduleCheck(),
      PG_SETTINGS_SCHEDULE_CHECK_MS,
  );
}

function stopPgSettingsSchedule() {
  if (pgSettingsScheduleTimer !== null) {
    window.clearInterval(pgSettingsScheduleTimer);
    pgSettingsScheduleTimer = null;
  }
}

function resetPgSettingsView() {
  pgSettingsHistory = [];
  currentPgSettingsSnapshot = null;
  currentPgSettingsComparison = null;
  pgSettingsChangeEvents = [];
  renderPgSettingsMonitoring();
  byId("pg-settings-status").className = "inline-status";
  byId("pg-settings-status").textContent = "Выберите connection.";
}

function initPgSettingsUi() {
  document.querySelectorAll("[data-monitoring-tab]").forEach((button) => {
    button.addEventListener("click", () => {
      const name = button.dataset.monitoringTab;
      setMonitoringTab(name);

      if (name === "settings") {
        renderPgSettingsConnectionOptions();
        void action(
            () => refreshPgSettingsMonitoring(),
            { startMessage: null },
        );
      } else if (name === "locks") {
        renderLocksConnectionOptions();
        void refreshLocksCases();
      }
    });
  });

  renderPgSettingsConnectionOptions();
  renderPgSettingsMonitoring();

  byId("pg-settings-collect").addEventListener("click", () => {
    void action(
        collectPgSettingsNow,
        { startMessage: null },
    );
  });

  byId("pg-settings-refresh").addEventListener("click", () => {
    void action(
        () => refreshPgSettingsMonitoring(),
        { startMessage: null },
    );
  });

  byId("pg-settings-connection").addEventListener("change", () => {
    currentPgSettingsSnapshot = null;
    currentPgSettingsComparison = null;
    pgSettingsChangeEvents = [];
    pgSettingsHistory = [];
    renderPgSettingsMonitoring();

    void action(
        () => refreshPgSettingsMonitoring(),
        { startMessage: null },
    );
  });

  byId("pg-settings-search").addEventListener("input", () => {
    renderPgSettingsCurrent();
  });

  byId("pg-settings-filter").addEventListener("change", () => {
    renderPgSettingsCurrent();
  });
}
