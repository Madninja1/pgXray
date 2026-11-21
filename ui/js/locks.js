"use strict";

let currentLockSnapshot = null;
let lastLockIncidentCapture = null;
let lockIncidentActive = false;
let locksMonitorTimer = null;
let locksRefreshInFlight = false;
let locksMonitoring = false;

function selectedLocksConnectionId() {
  const value = Number(byId("locks-connection")?.value);
  return Number.isInteger(value) && value > 0 ? value : null;
}

function renderLocksConnectionOptions() {
  const select = byId("locks-connection");
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
  byId("locks-start").disabled = !enabled || locksMonitoring;
  byId("locks-refresh").disabled = !enabled || locksMonitoring;

  if (!enabled) {
    byId("locks-status").textContent = "Сначала сохраните PostgreSQL connection.";
  }

  void refreshLocksCases();
}

async function refreshLocksCases() {
  const select = byId("locks-case");
  if (!select) {
    return;
  }

  const connectionId = selectedLocksConnectionId();
  const previous = select.value;
  select.replaceChildren();

  const create = document.createElement("option");
  create.value = "";
  create.textContent = "Создать новый incident Case";
  select.append(create);

  if (connectionId === null) {
    select.disabled = true;
    return;
  }

  try {
    const cases = await invoke("list_investigation_cases", { connectionId });
    select.disabled = false;

    for (const item of cases) {
      const option = document.createElement("option");
      option.value = String(item.id);
      option.textContent = `#${item.id} · ${item.kind === "incident" ? "incident" : "manual"} · ${item.title}`;
      select.append(option);
    }

    if (
      previous
      && cases.some((item) => String(item.id) === previous)
    ) {
      select.value = previous;
    }
  } catch (error) {
    select.disabled = true;
    byId("locks-status").textContent = `Cases недоступны: ${String(error)}`;
    byId("locks-status").className = "inline-status error";
  }
}

function locksBackendMap() {
  return new Map(
      (currentLockSnapshot?.backends ?? []).map((backend) => [backend.pid, backend]),
  );
}

function lockWaitLabel(backend) {
  if (!backend) {
    return "—";
  }

  const values = [backend.wait_event_type, backend.wait_event].filter(Boolean);
  return values.length > 0 ? values.join(" / ") : "—";
}

function lockRoleDatabase(backend) {
  return `${backend?.username ?? "—"} / ${backend?.database ?? "—"}`;
}

function lockApplication(backend) {
  if (!backend) {
    return "—";
  }

  return [backend.application_name || "—", backend.client_addr]
      .filter(Boolean)
      .join(" · ");
}

function lockAge(value) {
  return value === null || value === undefined
      ? "—"
      : formatDuration(Number(value));
}

function renderLocksSummary() {
  const summary = currentLockSnapshot?.summary;

  byId("locks-blocked").textContent = summary
      ? formatInteger(summary.blocked_backends)
      : "—";
  byId("locks-blockers").textContent = summary
      ? formatInteger(summary.direct_blockers)
      : "—";
  byId("locks-roots").textContent = summary
      ? formatInteger(summary.root_blockers)
      : "—";
  byId("locks-idle-roots").textContent = summary
      ? formatInteger(summary.idle_in_transaction_roots)
      : "—";
  byId("locks-depth").textContent = summary
      ? formatInteger(summary.longest_chain_depth)
      : "—";

  const incidentSnapshot = lastLockIncidentCapture?.snapshot ?? null;
  byId("locks-save-case").disabled = !incidentSnapshot;
  byId("locks-capture-state").textContent = incidentSnapshot
      ? `Сохранён in-memory incident snapshot: ${formatSnapshotTime(incidentSnapshot.captured_at_ms)} · blocked ${formatInteger(incidentSnapshot.summary.blocked_backends)}.`
      : "Incident snapshot ещё не захвачен.";
}

function renderLocksChains() {
  const body = byId("locks-chain-list");
  body.replaceChildren();

  if (!currentLockSnapshot || currentLockSnapshot.waits.length === 0) {
    tableEmpty(body, 6, "Blocking chains в текущем snapshot не найдены.");
    return;
  }

  const backends = locksBackendMap();

  for (const wait of currentLockSnapshot.waits) {
    const blocked = backends.get(wait.blocked_pid);

    for (const path of wait.paths) {
      const row = document.createElement("tr");
      const rootPid = path.pids[path.pids.length - 1];
      const root = backends.get(rootPid);
      const pathText = path.pids.map((pid, index) => {
        if (index === 0) {
          return `${pid} blocked`;
        }
        if (index === path.pids.length - 1) {
          return `${pid} root`;
        }
        return `${pid} blocker`;
      }).join(" → ");

      appendCells(row, [
        path.cycle_detected ? `${pathText} · cycle` : pathText,
        `${wait.blocked_pid} · ${lockRoleDatabase(blocked)} · ${blocked?.application_name || "—"}`,
        lockWaitLabel(blocked),
        root ? `${root.pid} · ${lockRoleDatabase(root)} · ${root.application_name || "—"}` : String(rootPid),
        root
          ? `${root.state ?? "—"} · state ${lockAge(root.state_age_seconds)}`
          : "backend disappeared from sample",
        lockAge(root?.xact_age_seconds),
      ]);

      if (root?.state?.startsWith("idle in transaction")) {
        row.classList.add("locks-idle-root-row");
      }

      body.append(row);
    }
  }
}

function renderLocksBackends() {
  const body = byId("locks-backend-list");
  body.replaceChildren();

  const backends = currentLockSnapshot?.backends ?? [];
  if (backends.length === 0) {
    tableEmpty(body, 10, "Relevant backends в текущем snapshot отсутствуют.");
    return;
  }

  const rootPids = new Set(
      (currentLockSnapshot.waits ?? [])
          .flatMap((wait) => wait.root_blocker_pids ?? []),
  );

  for (const backend of backends) {
    const row = document.createElement("tr");
    const queryCell = document.createElement("td");
    const actionsCell = document.createElement("td");
    const query = document.createElement("code");

    appendCells(row, [
      String(backend.pid),
      lockRoleDatabase(backend),
      lockApplication(backend),
      `${backend.state ?? "—"} · ${lockAge(backend.state_age_seconds)}`,
      lockAge(backend.xact_age_seconds),
      lockAge(backend.query_age_seconds),
      lockWaitLabel(backend),
      backend.query_id ?? "—",
    ]);

    query.className = "activity-query";
    query.textContent = compactActivityQuery(backend.query);
    queryCell.append(query);
    actionsCell.append(activityBackendActions(backend));
    row.append(queryCell, actionsCell);

    if (rootPids.has(backend.pid)) {
      row.classList.add("locks-root-row");
    }
    if (backend.state?.startsWith("idle in transaction")) {
      row.classList.add("activity-state-idle-xact-row");
    }

    body.append(row);
  }
}

function lockObjectLabel(lock) {
  if (lock.relation_name) {
    return `${lock.schema_name ?? "?"}.${lock.relation_name} · oid ${lock.relation_oid}`;
  }
  if (lock.transactionid) {
    return `transactionid ${lock.transactionid}`;
  }
  if (lock.virtualxid) {
    return `virtualxid ${lock.virtualxid}`;
  }
  if (lock.classid !== null && lock.classid !== undefined) {
    return `object class=${lock.classid} obj=${lock.objid ?? "—"} sub=${lock.objsubid ?? "—"}`;
  }
  return "—";
}

function renderLocksDetails() {
  const body = byId("locks-detail-list");
  const limitations = byId("locks-limitations");
  body.replaceChildren();
  limitations.replaceChildren();

  if (!currentLockSnapshot) {
    byId("locks-meta").textContent = "Snapshot ещё не получен.";
    tableEmpty(body, 6);
    return;
  }

  byId("locks-meta").textContent =
      `Snapshot ${formatSnapshotTime(currentLockSnapshot.captured_at_ms)} · `
      + `${currentLockSnapshot.database} (OID ${currentLockSnapshot.database_oid}) · `
      + `PostgreSQL ${currentLockSnapshot.server_version} · `
      + `lock rows ${formatInteger(currentLockSnapshot.locks.length)} / ${formatInteger(currentLockSnapshot.lock_count_total)}.`;

  for (const value of currentLockSnapshot.limitations ?? []) {
    const item = document.createElement("li");
    item.textContent = value;
    limitations.append(item);
  }

  if (currentLockSnapshot.locks.length === 0) {
    tableEmpty(body, 6, "Relevant pg_locks rows отсутствуют.");
    return;
  }

  for (const lock of currentLockSnapshot.locks) {
    appendCellsAndAppend(body, [
      String(lock.pid),
      lock.granted ? "yes" : "WAITING",
      lock.lock_type,
      lock.mode,
      lockObjectLabel(lock),
      lock.wait_start_ms ? formatSnapshotTime(lock.wait_start_ms) : "—",
    ]);
  }
}

function appendCellsAndAppend(body, values) {
  const row = document.createElement("tr");
  appendCells(row, values);
  body.append(row);
}

function renderLocksSnapshot() {
  renderLocksSummary();
  renderLocksChains();
  renderLocksBackends();
  renderLocksDetails();
}

function shouldReplaceLockIncidentCapture(current, candidate) {
  if (!current) {
    return true;
  }

  const currentSummary = current.snapshot?.summary ?? {};
  const candidateSummary = candidate.snapshot?.summary ?? {};
  const currentBlocked = Number(currentSummary.blocked_backends ?? 0);
  const candidateBlocked = Number(candidateSummary.blocked_backends ?? 0);

  if (candidateBlocked !== currentBlocked) {
    return candidateBlocked > currentBlocked;
  }

  const currentDepth = Number(currentSummary.longest_chain_depth ?? 0);
  const candidateDepth = Number(candidateSummary.longest_chain_depth ?? 0);

  if (candidateDepth !== currentDepth) {
    return candidateDepth > currentDepth;
  }

  const currentHasActivity = Boolean(current.activity_snapshot);
  const candidateHasActivity = Boolean(candidate.activity_snapshot);

  if (candidateHasActivity !== currentHasActivity) {
    return candidateHasActivity;
  }

  return Number(candidate.snapshot?.captured_at_ms ?? 0)
      > Number(current.snapshot?.captured_at_ms ?? 0);
}

function retainLockIncidentCapture(capture) {
  const blocked = Number(capture?.snapshot?.summary?.blocked_backends ?? 0);

  if (blocked <= 0) {
    lockIncidentActive = false;
    return;
  }

  if (!lockIncidentActive) {
    lastLockIncidentCapture = capture;
    lockIncidentActive = true;
    return;
  }

  if (shouldReplaceLockIncidentCapture(lastLockIncidentCapture, capture)) {
    lastLockIncidentCapture = capture;
  }
}

async function refreshLocksSnapshot() {
  if (locksRefreshInFlight) {
    return false;
  }

  const connectionId = selectedLocksConnectionId();
  if (connectionId === null) {
    throw new Error("Выберите PostgreSQL connection.");
  }

  locksRefreshInFlight = true;
  try {
    const capture = await invoke("postgres_lock_incident_capture", { connectionId });
    currentLockSnapshot = capture.snapshot;
    retainLockIncidentCapture(capture);
    renderLocksSnapshot();
    return true;
  } finally {
    locksRefreshInFlight = false;
  }
}

function setLocksMonitoringUi(running) {
  const hasConnection = selectedLocksConnectionId() !== null;
  byId("locks-start").disabled = running || !hasConnection;
  byId("locks-stop").disabled = !running;
  byId("locks-refresh").disabled = running || !hasConnection;
  byId("locks-connection").disabled = running;
  byId("locks-interval").disabled = running;
}

function stopLocksMonitoring(showMessage = true) {
  if (locksMonitorTimer !== null) {
    clearTimeout(locksMonitorTimer);
    locksMonitorTimer = null;
  }

  const wasRunning = locksMonitoring;
  locksMonitoring = false;
  setLocksMonitoringUi(false);

  if (showMessage && wasRunning) {
    byId("locks-status").textContent = "Locks / Waits live monitor остановлен.";
    byId("locks-status").className = "inline-status";
  }
}

async function locksMonitorTick() {
  if (!locksMonitoring) {
    return;
  }

  try {
    await refreshLocksSnapshot();
    if (!locksMonitoring) {
      return;
    }

    const blocked = currentLockSnapshot?.summary?.blocked_backends ?? 0;
    byId("locks-status").textContent = blocked > 0
        ? `Live · blocked backends: ${formatInteger(blocked)} · incident snapshot удерживается in-memory для Save to Case.`
        : (lastLockIncidentCapture
          ? `Live · текущая chain исчезла; snapshot ${formatSnapshotTime(lastLockIncidentCapture.snapshot.captured_at_ms)} удерживается для Save to Case.`
          : "Live · blocking chains сейчас не обнаружены.");
    byId("locks-status").className = blocked > 0
        ? "inline-status error"
        : "inline-status success";
  } catch (error) {
    if (!locksMonitoring) {
      return;
    }
    byId("locks-status").textContent = `Locks snapshot не получен: ${String(error)}`;
    byId("locks-status").className = "inline-status error";
  }

  const interval = Number(byId("locks-interval").value);
  locksMonitorTimer = setTimeout(() => void locksMonitorTick(), interval);
}

function startLocksMonitoring() {
  if (selectedLocksConnectionId() === null) {
    message("Выберите PostgreSQL connection.", true);
    return;
  }

  stopLocksMonitoring(false);
  locksMonitoring = true;
  setLocksMonitoringUi(true);
  byId("locks-status").textContent = "Locks / Waits live monitor запущен…";
  byId("locks-status").className = "inline-status pending";
  void locksMonitorTick();
}

async function saveCurrentLockIncident() {
  const connectionId = selectedLocksConnectionId();
  const incidentCapture = lastLockIncidentCapture;
  const incidentSnapshot = incidentCapture?.snapshot ?? null;

  if (connectionId === null || !incidentSnapshot) {
    throw new Error("Сначала дождитесь blocking chain или получите lock snapshot.");
  }

  const caseValue = byId("locks-case").value;
  const caseId = caseValue ? Number(caseValue) : null;

  const result = await invoke("save_lock_incident_to_case", {
    connectionId,
    caseId,
    snapshot: incidentSnapshot,
    activitySnapshot: incidentCapture.activity_snapshot ?? null,
    activityError: incidentCapture.activity_error ?? null,
  });

  await refreshLocksCases();
  byId("locks-case").value = String(result.case.id);

  message(
      `Lock incident сохранён: Case #${result.case.id}, artifact #${result.artifact.id}.`,
  );
}

async function signalLockBackend(pid, backendStartMs, actionName) {
  const connectionId = selectedLocksConnectionId();
  if (connectionId === null) {
    throw new Error("Выберите PostgreSQL connection.");
  }

  const backend = currentLockSnapshot?.backends.find((item) => item.pid === pid);
  const terminate = actionName === "terminate";
  const identity = backend
      ? `${backend.username || "—"}@${backend.database || "—"} · ${backend.application_name || "—"}`
      : `PID ${pid}`;
  const question = terminate
      ? `Завершить PostgreSQL backend PID ${pid}?\n\n${identity}\n\nОткрытая транзакция будет rollback.`
      : `Отменить текущий query PostgreSQL backend PID ${pid}?\n\n${identity}`;

  if (!window.confirm(question)) {
    return;
  }

  const result = await invoke("signal_postgres_backend", {
    connectionId,
    pid,
    backendStartMs,
    action: actionName,
  });

  message(
      result.signaled
          ? `${terminate ? "Terminate" : "Cancel"} отправлен PID ${pid}.`
          : `Backend PID ${pid} уже исчез или signal не был отправлен.`,
      !result.signaled,
  );

  await refreshLocksSnapshot();
}

function resetLocksView() {
  stopLocksMonitoring(false);
  currentLockSnapshot = null;
  lastLockIncidentCapture = null;
  lockIncidentActive = false;
  renderLocksSnapshot();
  byId("locks-status").textContent =
      "Connection изменён. Получите новый Locks / Waits snapshot.";
  byId("locks-status").className = "inline-status";
  void refreshLocksCases();
}

function initLocksUi() {
  renderLocksConnectionOptions();
  renderLocksSnapshot();

  byId("locks-start").addEventListener("click", startLocksMonitoring);
  byId("locks-stop").addEventListener("click", () => stopLocksMonitoring());

  byId("locks-refresh").addEventListener("click", () => {
    void action(async () => {
      await refreshLocksSnapshot();
      const blocked = currentLockSnapshot?.summary?.blocked_backends ?? 0;
      byId("locks-status").textContent = blocked > 0
          ? `Snapshot обновлён · blocked: ${formatInteger(blocked)}.`
          : "Snapshot обновлён · blocking chains не обнаружены.";
      byId("locks-status").className = blocked > 0
          ? "inline-status error"
          : "inline-status success";
    }, { startMessage: null });
  });

  byId("locks-save-case").addEventListener("click", () => {
    void action(saveCurrentLockIncident, { startMessage: "Сохраняю lock incident evidence…" });
  });

  byId("locks-connection").addEventListener("change", resetLocksView);

  byId("locks-backend-list").addEventListener("click", (event) => {
    const button = event.target.closest("button[data-activity-signal]");
    if (!button) {
      return;
    }

    const pid = Number(button.dataset.activityPid);
    const backendStartMs = Number(button.dataset.activityBackendStartMs);
    const signal = button.dataset.activitySignal;

    void action(
        () => signalLockBackend(pid, backendStartMs, signal),
        { startMessage: null },
    );
  });
}
