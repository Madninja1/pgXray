"use strict";

const invoke = window.pgxrayInvoke ?? window.__TAURI__?.core?.invoke;
const Channel = window.__TAURI__?.core?.Channel;
const byId = (id) => document.getElementById(id);

let busy = false;
let currentUsername = null;

let connectionsById = new Map();
let connectionChecks = new Map();
let connectionStatusTimers = new Map();
let connectionEditorStatusTimer = null;

let sessionsById = new Map();
let terminalViews = new Map();
let sessionChannels = new Map();
let activeSessionId = null;

let xidReference = null;
let xidReferencePromise = null;
let xidServerVersionNum = null;
let xidHistorySnapshots = [];

let pgssReference = null;
let pgssReferencePromise = null;
let pgssServerVersionNum = null;
let pgssPerformanceSamples = null;
let pgssHistorySnapshots = [];

let ioWalReference = null;
let ioWalReferencePromise = null;
let ioWalCurrentSnapshot = null;
let ioWalServerVersionNum = null;

let indexHealthReference = null;
let indexHealthReferencePromise = null;
let indexHealthCurrentSnapshot = null;
let indexHealthCurrentConnectionId = null;
let indexHealthServerVersionNum = null;

let autovacuumSnapshot = null;
let autovacuumReference = null;
let autovacuumReferencePromise = null;
let autovacuumServerVersionNum = null;
let autovacuumHistorySnapshots = [];
let selectedAutovacuumRelationOid = null;

let activeDiagnosticModule = "xid";
let activeHistoryModule = "xid";

let whatChangedComparison = null;
let whatChangedMarkers = [];
let whatChangedHistoryConnections = [];

let investigationCases = [];
let casePrivacyScopes = [];
let privacyScopes = [];
let caseArtifacts = [];
let pev2Artifacts = [];
let activeCaseId = null;
let activeToolsTab = "cases";
let pendingPev2Payload = null;
let pev2Ready = false;

let terminal = null;
let fitAddon = null;
let terminalResizeObserver = null;

function message(text, error = false) {
  byId("message").textContent = text;
  byId("message").classList.toggle("error", error);
}

function screen(name) {
  for (const candidate of ["registration", "login", "workspace"]) {
    byId(`${candidate}-screen`).hidden = candidate !== name;
  }

  byId(`${name}-screen`).querySelector("input")?.focus();
}

function setMainView(name) {
  document.querySelectorAll(".main-nav-item").forEach((button) => {
    const active = button.dataset.view === name;

    button.classList.toggle("active", active);
    button.setAttribute("aria-selected", String(active));
  });

  for (const candidate of [
    "connections",
    "diagnostics",
    "activity",
    "history",
    "changes",
    "tools",
    "sessions",
    "settings",
  ]) {
    byId(`${candidate}-view`).hidden = candidate !== name;
  }

  if (name !== "activity" && typeof stopActivityMonitoring === "function") {
    stopActivityMonitoring(false);
  }

  if (name !== "activity" && typeof stopLocksMonitoring === "function") {
    stopLocksMonitoring(false);
  }

  if (name === "sessions" && activeSessionId !== null) {
    requestAnimationFrame(() => {
      fitTerminal();
      terminal?.focus();
    });
  }
}

function setBusy(value) {
  document.querySelectorAll("button").forEach((button) => {
    if (value) {
      button.dataset.busyPreviousDisabled = button.disabled ? "1" : "0";
      button.disabled = true;
      return;
    }

    if (button.dataset.busyPreviousDisabled !== undefined) {
      button.disabled = button.dataset.busyPreviousDisabled === "1";
      delete button.dataset.busyPreviousDisabled;
    }
  });
}

async function action(
    operation,
    {
      onError = null,
      startMessage = "Выполняется…",
    } = {},
) {
  if (busy) {
    return false;
  }

  busy = true;
  setBusy(true);

  if (startMessage) {
    message(startMessage);
  }

  try {
    if (!invoke) {
      throw new Error(
          "Откройте приложение через cargo tauri dev, а не как HTML-файл.",
      );
    }

    await operation();
    return true;
  } catch (error) {
    const text = String(error);

    if (onError) {
      onError(text);
    } else {
      message(text, true);
    }

    return false;
  } finally {
    busy = false;
    setBusy(false);
  }
}

function clearPasswordInputs(form) {
  form
      .querySelectorAll('input[type="password"]')
      .forEach((input) => {
        input.value = "";
      });
}

function setSession(session) {
  currentUsername = session.username;
  byId("identity").textContent = `Пользователь pgXray: ${session.username}`;
}

async function enterWorkspace(session) {
  setSession(session);
  setMainView("connections");
  screen("workspace");

  await Promise.all([
    refreshConnections(),
    refreshSessions(),
  ]);

  // Configuration snapshots are manual by default. Legacy UI no longer
  // starts an implicit all-connections background collection schedule.
}

function formatInteger(value) {
  if (value === null || value === undefined) {
    return "—";
  }

  return new Intl.NumberFormat("ru-RU").format(value);
}

function formatNumber(value, digits = 2) {
  if (value === null || value === undefined) {
    return "—";
  }

  return new Intl.NumberFormat(
      "ru-RU",
      {
        maximumFractionDigits: digits,
      },
  ).format(Number(value));
}

function formatPercent(value) {
  if (value === null || value === undefined) {
    return "—";
  }

  return `${Number(value).toFixed(1)}%`;
}

function formatBytes(value) {
  if (value === null || value === undefined) {
    return "—";
  }

  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let size = Number(value);
  let unit = 0;

  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }

  const digits = unit === 0 || size >= 100 ? 0 : 1;

  return `${size.toFixed(digits)} ${units[unit]}`;
}

function formatDuration(seconds) {
  if (seconds === null || seconds === undefined) {
    return "—";
  }

  let rest = Math.max(0, Number(seconds));

  const days = Math.floor(rest / 86400);
  rest %= 86400;

  const hours = Math.floor(rest / 3600);
  rest %= 3600;

  const minutes = Math.floor(rest / 60);
  rest %= 60;

  const parts = [];

  if (days) {
    parts.push(`${days}д`);
  }

  if (hours) {
    parts.push(`${hours}ч`);
  }

  if (minutes) {
    parts.push(`${minutes}м`);
  }

  if (parts.length === 0) {
    parts.push(`${Math.floor(rest)}с`);
  }

  return parts.join(" ");
}

function formatTimestamp(value) {
  if (!value) {
    return "—";
  }

  const date = new Date(value);

  if (Number.isNaN(date.getTime())) {
    return String(value);
  }

  return date.toLocaleString("ru-RU");
}

function yesNo(value) {
  return value ? "да" : "нет";
}

function tableEmpty(body, columns, text = "Нет данных") {
  body.replaceChildren();

  const row = document.createElement("tr");
  const cell = document.createElement("td");

  cell.colSpan = columns;
  cell.className = "table-empty";
  cell.textContent = text;

  row.append(cell);
  body.append(row);
}

function appendCells(row, values) {
  for (const value of values) {
    const cell = document.createElement("td");

    cell.textContent = value;
    row.append(cell);
  }
}

function renderSectionError(id, section) {
  const element = byId(id);
  const error = section?.error;

  element.hidden = !error;
  element.textContent = error
      ? `Раздел недоступен: ${error}`
      : "";
}

function formatSignedInteger(value) {
  if (value === null || value === undefined) {
    return "—";
  }

  const number = Number(value);
  const formatted = formatInteger(
      Math.abs(number),
  );

  if (number > 0) {
    return `+${formatted}`;
  }

  if (number < 0) {
    return `-${formatted}`;
  }

  return "0";
}

function formatSnapshotTime(milliseconds) {
  if (milliseconds === null || milliseconds === undefined) {
    return "—";
  }

  const date =
      new Date(Number(milliseconds));

  if (Number.isNaN(date.getTime())) {
    return "—";
  }

  return date.toLocaleString("ru-RU");
}

function relationSampleStateLabel(state) {
  if (state === "present_in_both") {
    return "в обоих TOP";
  }

  if (state === "entered_new_sample") {
    return "вошла в новый TOP";
  }

  if (state === "not_in_new_sample") {
    return "не в новом TOP";
  }

  return state;
}

const HTML_FRAGMENT_GROUPS = [
  {
    targetId: "auth-views",
    paths: [
      "views/auth.html",
    ],
  },
  {
    targetId: "workspace-views",
    paths: [
      "views/connections.html",
      "views/diagnostics.html",
      "views/activity.html",
      "views/history.html",
      "views/changes.html",
      "views/tools.html",
      "views/sessions.html",
      "views/settings.html",
    ],
  },
];

async function loadHtmlFragments() {
  for (const group of HTML_FRAGMENT_GROUPS) {
    const target = byId(group.targetId);

    if (!target) {
      throw new Error(
          `Не найден контейнер UI fragment: #${group.targetId}`,
      );
    }

    const fragments = await Promise.all(
        group.paths.map(async (path) => {
          const response = await fetch(path);

          if (!response.ok) {
            throw new Error(
                `Не удалось загрузить UI fragment ${path}: HTTP ${response.status}`,
            );
          }

          return response.text();
        }),
    );

    target.replaceChildren();

    for (const fragment of fragments) {
      target.insertAdjacentHTML(
          "beforeend",
          fragment,
      );
    }
  }
}
