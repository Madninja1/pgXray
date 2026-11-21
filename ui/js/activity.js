"use strict";

let currentActivitySnapshot = null;
let activitySamples = [];
let activityMonitorTimer = null;
let activityMonitorDeadline = null;
let activityMonitorStartedAt = null;
let activityMonitorEndedAt = null;
let activityRefreshInFlight = false;
let activityMonitoring = false;
// Invalidate completed IPC responses whenever the selected server or UI lifecycle changes.
let activityRevision = 0;

function selectedActivityConnectionId() {
  const value = Number(byId("activity-connection").value);
  return Number.isInteger(value) && value > 0 ? value : null;
}

function renderActivityConnectionOptions() {
  const select = byId("activity-connection");
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

  const hasConnection = connections.length > 0;
  byId("activity-start").disabled = !hasConnection || activityMonitoring;
  byId("activity-refresh").disabled = !hasConnection || activityMonitoring;

  if (!hasConnection) {
    byId("activity-status").textContent =
        "Сначала сохраните PostgreSQL connection.";
  }
}

function activityStateLabel(backend) {
  const state = backend.state || "—";
  const type = backend.backend_type;

  if (type && type !== "client backend") {
    return `${state} · ${type}`;
  }

  return state;
}

function compactActivityQuery(query) {
  if (!query) {
    return "—";
  }

  const compact = String(query).replace(/\s+/g, " ").trim();
  return compact.length > 500
      ? `${compact.slice(0, 497)}…`
      : compact;
}

function renderActivitySummary() {
  const snapshot = currentActivitySnapshot;

  byId("activity-total").textContent = snapshot
      ? formatInteger(snapshot.total_connections)
      : "—";
  byId("activity-active").textContent = snapshot
      ? formatInteger(snapshot.active_connections)
      : "—";
  byId("activity-waiting").textContent = snapshot
      ? formatInteger(snapshot.waiting_active_connections)
      : "—";
  byId("activity-idle-xact").textContent = snapshot
      ? formatInteger(snapshot.idle_in_transaction_connections)
      : "—";
  byId("activity-pgxray").textContent = snapshot
      ? formatInteger(snapshot.pgxray_connections)
      : "—";
  byId("activity-captured-at").textContent = snapshot
      ? `Snapshot: ${formatSnapshotTime(snapshot.captured_at_ms)}`
      : "";
}

function renderActivitySampleStats() {
  const element = byId("activity-sample-stats");

  if (activitySamples.length === 0) {
    element.textContent = "Сессия мониторинга ещё не запускалась.";
    return;
  }

  const totalActive = activitySamples.reduce(
      (sum, sample) => sum + sample.active,
      0,
  );
  const averageActive = totalActive / activitySamples.length;
  const maxActive = Math.max(...activitySamples.map((sample) => sample.active));
  const maxWaiting = Math.max(...activitySamples.map((sample) => sample.waiting));
  const maxConnections = Math.max(...activitySamples.map((sample) => sample.total));
  const elapsedUntil = activityMonitoring
      ? Date.now()
      : (activityMonitorEndedAt ?? Date.now());
  const elapsedMs = activityMonitorStartedAt === null
      ? 0
      : Math.max(0, elapsedUntil - activityMonitorStartedAt);

  element.textContent =
      `Samples: ${formatInteger(activitySamples.length)} · `
      + `avg active: ${formatNumber(averageActive, 1)} · `
      + `max active: ${formatInteger(maxActive)} · `
      + `max waiting: ${formatInteger(maxWaiting)} · `
      + `max connections: ${formatInteger(maxConnections)} · `
      + `elapsed: ${formatDuration(elapsedMs / 1000)}`;
}

function activityBackendActions(backend) {
  const container = document.createElement("div");
  container.className = "activity-actions";

  const clientBackend =
      !backend.backend_type || backend.backend_type === "client backend";

  if (!clientBackend) {
    const unavailable = document.createElement("span");
    unavailable.className = "hint";
    unavailable.textContent = "Не client backend";
    container.append(unavailable);
    return container;
  }

  if (!Number.isInteger(backend.backend_start_ms) || backend.backend_start_ms <= 0) {
    const unavailable = document.createElement("span");
    unavailable.className = "hint";
    unavailable.textContent = "Нет backend_start";
    container.append(unavailable);
    return container;
  }

  const cancel = document.createElement("button");
  cancel.type = "button";
  cancel.className = "secondary";
  cancel.textContent = "Cancel query";
  cancel.disabled = backend.state !== "active";
  cancel.dataset.activitySignal = "cancel";
  cancel.dataset.activityPid = String(backend.pid);
  cancel.dataset.activityBackendStartMs = String(backend.backend_start_ms);

  const terminate = document.createElement("button");
  terminate.type = "button";
  terminate.className = "danger";
  terminate.textContent = "Terminate backend";
  terminate.dataset.activitySignal = "terminate";
  terminate.dataset.activityPid = String(backend.pid);
  terminate.dataset.activityBackendStartMs = String(backend.backend_start_ms);

  container.append(cancel, terminate);
  return container;
}

function renderActivityBackends() {
  const body = byId("activity-backends");
  const snapshot = currentActivitySnapshot;

  if (!snapshot) {
    tableEmpty(body, 9, "Snapshot ещё не получен.");
    return;
  }

  const hideIdle = byId("activity-hide-idle").checked;
  const backends = snapshot.backends.filter(
      (backend) => !(hideIdle && backend.state === "idle"),
  );

  if (backends.length === 0) {
    tableEmpty(body, 9, "Подходящих backend processes нет.");
    return;
  }

  body.replaceChildren();

  for (const backend of backends) {
    const row = document.createElement("tr");
    if (backend.is_pgxray) {
      row.classList.add("activity-pgxray-row");
    }

    const pid = document.createElement("td");
    pid.textContent = String(backend.pid);

    const identity = document.createElement("td");
    identity.textContent =
        `${backend.database || "—"} / ${backend.username || "—"}`;

    const application = document.createElement("td");
    application.textContent =
        `${backend.application_name || "—"} / ${backend.client_addr || "local"}`;

    const state = document.createElement("td");
    state.textContent = activityStateLabel(backend);
    if (backend.state === "active") {
      state.classList.add("activity-state-active");
    } else if (backend.state?.startsWith("idle in transaction")) {
      state.classList.add("activity-state-idle-xact");
    }

    const queryAge = document.createElement("td");
    queryAge.textContent = formatDuration(backend.query_age_seconds);

    const xactAge = document.createElement("td");
    xactAge.textContent = formatDuration(backend.xact_age_seconds);

    const wait = document.createElement("td");
    wait.textContent = backend.wait_event_type
        ? `${backend.wait_event_type}: ${backend.wait_event || "—"}`
        : "—";
    if (backend.wait_event_type === "Lock") {
      wait.classList.add("activity-wait-lock");
    }

    const query = document.createElement("td");
    const queryCode = document.createElement("code");
    queryCode.className = "activity-query";
    queryCode.textContent = compactActivityQuery(backend.query);
    if (backend.query) {
      queryCode.title = backend.query;
    }
    query.append(queryCode);

    const actions = document.createElement("td");
    actions.append(activityBackendActions(backend));

    row.append(
        pid,
        identity,
        application,
        state,
        queryAge,
        xactAge,
        wait,
        query,
        actions,
    );
    body.append(row);
  }
}

function renderActivitySnapshot() {
  renderActivitySummary();
  renderActivitySampleStats();
  renderActivityBackends();
}

function recordActivitySample(snapshot) {
  activitySamples.push({
    total: snapshot.total_connections,
    active: snapshot.active_connections,
    waiting: snapshot.waiting_active_connections,
  });
}

async function refreshActivitySnapshot({ recordSample = false } = {}) {
  if (activityRefreshInFlight) {
    return false;
  }

  const connectionId = selectedActivityConnectionId();
  if (connectionId === null) {
    throw new Error("Выберите PostgreSQL connection для мониторинга.");
  }

  activityRefreshInFlight = true;
  const revision = activityRevision;

  try {
    const snapshot = await invoke(
        "postgres_activity_snapshot",
        { connectionId },
    );

    if (revision !== activityRevision || selectedActivityConnectionId() !== connectionId) {
      return false;
    }

    currentActivitySnapshot = snapshot;
    if (recordSample) {
      recordActivitySample(snapshot);
    }
    renderActivitySnapshot();
    return true;
  } finally {
    activityRefreshInFlight = false;
  }
}

function setActivityMonitoringUi(running) {
  const hasConnection = selectedActivityConnectionId() !== null;

  byId("activity-start").disabled = running || !hasConnection;
  byId("activity-stop").disabled = !running;
  byId("activity-refresh").disabled = running || !hasConnection;
  byId("activity-connection").disabled = running;
  byId("activity-interval").disabled = running;
  byId("activity-duration").disabled = running;
}

function stopActivityMonitoring(showMessage = true) {
  activityRevision += 1;
  if (activityMonitorTimer !== null) {
    clearTimeout(activityMonitorTimer);
    activityMonitorTimer = null;
  }

  const wasRunning = activityMonitoring;
  if (wasRunning) {
    activityMonitorEndedAt = Date.now();
  }
  activityMonitoring = false;
  activityMonitorDeadline = null;
  setActivityMonitoringUi(false);

  if (showMessage && wasRunning) {
    byId("activity-status").textContent =
        `Мониторинг остановлен. Получено samples: ${activitySamples.length}.`;
    byId("activity-status").className = "inline-status";
  }
}

async function activityMonitorTick() {
  if (!activityMonitoring) {
    return;
  }

  try {
    await refreshActivitySnapshot({ recordSample: true });
    if (!activityMonitoring) {
      return;
    }
    byId("activity-status").textContent =
        `Мониторинг активен · samples: ${activitySamples.length}.`;
    byId("activity-status").className = "inline-status success";
  } catch (error) {
    if (!activityMonitoring) {
      return;
    }
    byId("activity-status").textContent =
        `Snapshot не получен: ${String(error)}`;
    byId("activity-status").className = "inline-status error";
  }

  if (activityMonitorDeadline !== null && Date.now() >= activityMonitorDeadline) {
    stopActivityMonitoring(false);
    byId("activity-status").textContent =
        `Мониторинг завершён по duration. Получено samples: ${activitySamples.length}.`;
    byId("activity-status").className = "inline-status success";
    return;
  }

  const interval = Number(byId("activity-interval").value);
  activityMonitorTimer = setTimeout(
      () => void activityMonitorTick(),
      interval,
  );
}

function startActivityMonitoring() {
  const connectionId = selectedActivityConnectionId();
  if (connectionId === null) {
    message("Выберите PostgreSQL connection для мониторинга.", true);
    return;
  }

  stopActivityMonitoring(false);
  activitySamples = [];
  activityMonitorStartedAt = Date.now();
  activityMonitorEndedAt = null;
  activityMonitorDeadline =
      activityMonitorStartedAt + Number(byId("activity-duration").value);
  activityMonitoring = true;
  setActivityMonitoringUi(true);
  renderActivitySampleStats();

  byId("activity-status").textContent =
      "Мониторинг запущен. Получаю первый snapshot…";
  byId("activity-status").className = "inline-status pending";

  void activityMonitorTick();
}

async function signalActivityBackend(pid, backendStartMs, actionName) {
  const connectionId = selectedActivityConnectionId();
  if (connectionId === null) {
    throw new Error("Выберите PostgreSQL connection.");
  }

  const snapshot = currentActivitySnapshot;
  const backend = snapshot?.backends.find(
      (item) => item.pid === pid && item.backend_start_ms === backendStartMs,
  );
  const ageMs = snapshot ? Date.now() - snapshot.captured_at_ms : Infinity;
  if (!backend || !Number.isInteger(backendStartMs) || backendStartMs <= 0
      || !Number.isFinite(ageMs) || ageMs < 0 || ageMs > 15000
      || (backend.backend_type && backend.backend_type !== "client backend")
      || (actionName === "cancel" && backend.state !== "active")) {
    throw new Error("Activity snapshot устарел или backend изменился. Обновите snapshot перед действием.");
  }
  const revision = activityRevision;
  const identity = backend
      ? `${backend.username || "—"}@${backend.database || "—"} · ${backend.application_name || "—"}`
      : `PID ${pid}`;

  const terminate = actionName === "terminate";
  const question = terminate
      ? `Завершить PostgreSQL backend PID ${pid}?\n\n${identity}\n\nСоединение будет оборвано. Открытая транзакция будет rollback.`
      : `Отменить текущий query PostgreSQL backend PID ${pid}?\n\n${identity}\n\nСоединение останется открытым.`;

  if (!window.confirm(question)) {
    return;
  }
  // Never combine a PID from one snapshot with a different connection or user view.
  if (revision !== activityRevision || selectedActivityConnectionId() !== connectionId
      || currentActivitySnapshot !== snapshot || Date.now() - snapshot.captured_at_ms > 15000) {
    throw new Error("Activity snapshot изменился во время подтверждения. Обновите snapshot.");
  }

  const result = await invoke(
      "signal_postgres_backend",
      {
        connectionId,
        pid,
        backendStartMs,
        action: actionName,
      },
  );

  if (revision !== activityRevision || selectedActivityConnectionId() !== connectionId) {
    return;
  }

  if (!result.signaled) {
    message(
        `PostgreSQL не отправил ${actionName} для PID ${pid}: backend уже исчез или не является client backend.`,
        true,
    );
  } else {
    message(
        terminate
            ? `Terminate отправлен backend PID ${pid}.`
            : `Cancel отправлен query backend PID ${pid}.`,
    );
  }

  await refreshActivitySnapshot();
}

function resetActivityView() {
  stopActivityMonitoring(false);
  currentActivitySnapshot = null;
  activitySamples = [];
  activityMonitorStartedAt = null;
  activityMonitorEndedAt = null;
  renderActivitySnapshot();
  byId("activity-status").className = "inline-status";
  byId("activity-status").textContent =
      "Выберите connection и запустите мониторинг.";
}

function initActivityUi() {
  renderActivityConnectionOptions();
  renderActivitySnapshot();

  byId("activity-start").addEventListener("click", () => {
    startActivityMonitoring();
  });

  byId("activity-stop").addEventListener("click", () => {
    stopActivityMonitoring();
  });

  byId("activity-refresh").addEventListener("click", () => {
    void action(
        async () => {
          await refreshActivitySnapshot();
          byId("activity-status").textContent = "Snapshot обновлён.";
          byId("activity-status").className = "inline-status success";
        },
        { startMessage: null },
    );
  });

  byId("activity-hide-idle").addEventListener("change", () => {
    renderActivityBackends();
  });

  byId("activity-connection").addEventListener("change", () => {
    stopActivityMonitoring(false);
    currentActivitySnapshot = null;
    activitySamples = [];
    activityMonitorStartedAt = null;
    activityMonitorEndedAt = null;
    renderActivitySnapshot();
    byId("activity-status").className = "inline-status";
    byId("activity-status").textContent =
        "Connection изменён. Получите новый snapshot или запустите мониторинг.";
  });

  byId("activity-backends").addEventListener("click", (event) => {
    const button = event.target.closest("button[data-activity-signal]");
    if (!button) {
      return;
    }

    const pid = Number(button.dataset.activityPid);
    const backendStartMs = Number(button.dataset.activityBackendStartMs);
    const signal = button.dataset.activitySignal;

    void action(
        () => signalActivityBackend(pid, backendStartMs, signal),
        { startMessage: null },
    );
  });
}
