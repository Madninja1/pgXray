"use strict";

const pendingTerminalOutput = new Map();

const TERMINAL_ERROR_TIMEOUT_MS = 5000;
let terminalErrorTimer = null;

function clearTerminalError() {
  const element = byId("terminal-error");

  if (terminalErrorTimer !== null) {
    window.clearTimeout(terminalErrorTimer);
    terminalErrorTimer = null;
  }

  element.hidden = true;
  element.textContent = "";
}

function showTerminalError(text) {
  clearTerminalError();

  const element = byId("terminal-error");

  element.textContent = text;
  element.hidden = false;

  terminalErrorTimer = window.setTimeout(() => {
    element.hidden = true;
    element.textContent = "";
    terminalErrorTimer = null;
  }, TERMINAL_ERROR_TIMEOUT_MS);
}

function activeSessionForConnectionCase(
    connectionId,
    caseId = null,
) {
  return Array.from(sessionsById.values()).find(
      (session) =>
          session.connection_id === connectionId
          && (session.case_id ?? null) === caseId
          && session.running,
  ) ?? null;
}

function nextAnimationFrame() {
  return new Promise((resolve) => {
    requestAnimationFrame(() => {
      resolve();
    });
  });
}

async function fitTerminalStable() {
  await nextAnimationFrame();
  fitTerminal();

  // A second frame matters after changing main view / un-hiding terminal-shell:
  // WebKit can report the previous layout size during the first frame.
  await nextAnimationFrame();
  fitTerminal();

  return {
    cols: terminal?.cols || 120,
    rows: terminal?.rows || 30,
  };
}

async function copyTerminalSelection(target = terminal) {
  if (!target?.hasSelection()) {
    return;
  }

  const text = target.getSelection();

  if (!text) {
    return;
  }

  const clipboard =
      window.__TAURI__?.clipboardManager;

  if (!clipboard?.writeText) {
    showTerminalError(
        "Tauri Clipboard API недоступен.",
    );

    return;
  }

  try {
    await clipboard.writeText(text);
  } catch (error) {
    showTerminalError(
        `Не удалось скопировать текст терминала: ${String(error)}`,
    );
  }
}

function ensureTerminalInfrastructure() {
  if (!window.Terminal || !window.FitAddon?.FitAddon) {
    throw new Error(
        "xterm.js не загружен. Проверьте файлы ui/vendor/xterm/.",
    );
  }

  if (terminalResizeObserver) {
    return;
  }

  terminalResizeObserver = new ResizeObserver(() => {
    if (byId("sessions-view").hidden) {
      return;
    }

    fitTerminal();
  });

  terminalResizeObserver.observe(
      byId("terminal-shell"),
  );
}

function createTerminalView(sessionId = null) {
  ensureTerminalInfrastructure();

  const host = document.createElement("div");
  host.className = "terminal-instance";
  host.hidden = true;

  byId("terminal").append(host);

  const instance = new window.Terminal({
    cursorBlink: true,
    cursorStyle: "block",
    convertEol: false,
    scrollback: 20000,
    fontSize: 14,
    lineHeight: 1.2,
    fontFamily:
        "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
    theme: terminalThemeFromCss(),
  });

  const addon = new window.FitAddon.FitAddon();
  const view = {
    sessionId,
    terminal: instance,
    fitAddon: addon,
    host,
    ready: false,
  };

  instance.loadAddon(addon);
  instance.open(host);

  instance.attachCustomKeyEventHandler((event) => {
    if (event.type !== "keydown") {
      return true;
    }

    // event.key зависит от текущей раскладки (например, "с" / "м"),
    // event.code привязан к физической клавише и подходит для shortcuts.
    if (event.code === "KeyC") {
      const ctrlCopy =
          event.ctrlKey
          && instance.hasSelection();

      const metaCopy =
          event.metaKey
          && instance.hasSelection();

      if (ctrlCopy || metaCopy) {
        void copyTerminalSelection(instance);

        return false;
      }

      // Ctrl+Shift+C — terminal-style copy shortcut.
      // Без выделения он не должен отправлять Ctrl+C в psql.
      if (
          event.ctrlKey
          && event.shiftKey
      ) {
        return false;
      }

      // Обычный Ctrl+C без выделения остаётся сигналом ^C для psql.
      return true;
    }

    // Paste намеренно не перехватываем здесь. xterm сам обрабатывает
    // native `paste` event WebView. Ручной instance.paste() на keydown
    // приводил к двойной вставке для Ctrl+Shift+V / Cmd+V.
    return true;
  });

  instance.onData((data) => {
    const currentSessionId = view.sessionId;

    if (
        currentSessionId === null
        || activeSessionId !== currentSessionId
    ) {
      return;
    }

    const session = sessionsById.get(currentSessionId);

    if (!session?.running) {
      return;
    }

    void invoke(
        "send_psql_session_input",
        {
          sessionId: session.id,
          connectionId: session.connection_id,
          caseId: session.case_id ?? null,
          data,
        },
    ).catch((error) => {
      showTerminalError(
          `Не удалось передать ввод в PTY: ${String(error)}`,
      );
    });
  });

  instance.onResize(({ cols, rows }) => {
    if (view.sessionId === null) {
      return;
    }

    void resizeSession(
        view.sessionId,
        cols,
        rows,
    );
  });

  return view;
}

function fitTerminal() {
  if (!terminal || !fitAddon || byId("terminal-shell").hidden) {
    return;
  }

  try {
    fitAddon.fit();
  } catch (error) {
    console.error("Terminal fit failed:", error);
  }
}

function appendPendingTerminalOutput(sessionId, data) {
  if (!data) {
    return;
  }

  const current = pendingTerminalOutput.get(sessionId) ?? "";
  let next = current + data;

  const maxLength = 5_000_000;

  if (next.length > maxLength) {
    next = next.slice(-maxLength);
  }

  pendingTerminalOutput.set(sessionId, next);
}

function flushPendingTerminalOutput(view) {
  if (view.sessionId === null) {
    return;
  }

  const data = pendingTerminalOutput.get(view.sessionId);

  if (!data) {
    return;
  }

  pendingTerminalOutput.delete(view.sessionId);
  view.terminal.write(data);
}

function registerTerminalView(sessionId, view) {
  view.sessionId = sessionId;
  terminalViews.set(sessionId, view);
}

function disposeTerminalView(sessionId) {
  const view = terminalViews.get(sessionId);

  if (!view) {
    pendingTerminalOutput.delete(sessionId);
    return;
  }

  if (terminal === view.terminal) {
    terminal = null;
    fitAddon = null;
  }

  view.terminal.dispose();
  view.host.remove();
  terminalViews.delete(sessionId);
  pendingTerminalOutput.delete(sessionId);
}

function disposeAllTerminalViews() {
  for (const sessionId of Array.from(terminalViews.keys())) {
    disposeTerminalView(sessionId);
  }

  pendingTerminalOutput.clear();
  terminal = null;
  fitAddon = null;
}

function cleanupTerminalViews(validSessionIds) {
  for (const sessionId of Array.from(terminalViews.keys())) {
    if (!validSessionIds.has(sessionId)) {
      disposeTerminalView(sessionId);
    }
  }
}

async function activateTerminalView(sessionId) {
  let view = terminalViews.get(sessionId);

  if (!view) {
    // This can happen after a frontend reload while the backend psql is still
    // alive. We cannot reconstruct its previous screen, but future output and
    // input remain isolated in a dedicated terminal instance.
    view = createTerminalView(sessionId);
    terminalViews.set(sessionId, view);
  }

  for (const [candidateId, candidate] of terminalViews) {
    candidate.host.hidden = candidateId !== sessionId;
  }

  terminal = view.terminal;
  fitAddon = view.fitAddon;

  await fitTerminalStable();

  if (activeSessionId !== sessionId) {
    return;
  }

  view.ready = true;
  flushPendingTerminalOutput(view);
  view.terminal.focus();
}

function renderTerminalSession() {
  const session = activeSessionId !== null
      ? sessionsById.get(activeSessionId)
      : null;

  byId("session-empty").hidden = Boolean(session);
  byId("terminal-shell").hidden = !session;

  if (!session) {
    terminal = null;
    fitAddon = null;
    return;
  }

  byId("terminal-session-title").textContent =
      `${session.connection_name} · ${session.username}@${session.host}:${session.port}/${session.database}`;

  byId("terminal-session-state").textContent = session.running
      ? `Права роли PostgreSQL · editor: ${session.editor ?? "—"}`
      : "psql завершён";

  byId("terminal-config-path").textContent =
      [
        session.workspace_path
            ? `workspace: ${session.workspace_path}`
            : "",
        session.psqlrc_path
            ? `.psqlrc: ${session.psqlrc_path}`
            : "",
      ]
          .filter(Boolean)
          .join(" · ");

  void activateTerminalView(session.id).catch((error) => {
    showTerminalError(
        `Не удалось подготовить terminal session: ${String(error)}`,
    );
  });
}

function renderSessions(sessions) {
  sessionsById = new Map(
      sessions.map((session) => [
        session.id,
        session,
      ]),
  );

  cleanupTerminalViews(
      new Set(sessions.map((session) => session.id)),
  );

  byId("session-count").textContent = String(
      sessions.filter((session) => session.running).length,
  );

  if (
      activeSessionId !== null
      && !sessionsById.has(activeSessionId)
  ) {
    activeSessionId = null;
  }

  if (
      activeSessionId === null
      && sessions.length > 0
  ) {
    activeSessionId = sessions[0].id;
  }

  const list = byId("session-list");

  list.replaceChildren();

  for (const session of sessions) {
    const button = document.createElement("button");

    button.type = "button";
    button.className = "session-selector secondary";

    button.classList.toggle(
        "active",
        session.id === activeSessionId,
    );

    button.dataset.sessionId = String(session.id);

    const sessionLabel = session.case_id
        ? `#${session.id} · ${session.connection_name} · case #${session.case_id}`
        : `#${session.id} · ${session.connection_name}`;

    button.textContent = session.running
        ? sessionLabel
        : `${sessionLabel} · завершена`;

    list.append(button);
  }

  renderTerminalSession();

  if (connectionsById.size > 0) {
    renderConnections(
        Array.from(connectionsById.values()),
    );
  }
}

async function refreshSessions() {
  const sessions = await invoke("list_psql_sessions");
  for (const session of sessions) {
    if (!session.running || sessionChannels.has(session.id)) continue;
    const onEvent = createPsqlChannel();
    try {
      await invoke("subscribe_psql_session", {
        sessionId: session.id,
        connectionId: session.connection_id,
        caseId: session.case_id,
        onEvent,
      });
      sessionChannels.set(session.id, onEvent);
    } catch (error) {
      onEvent.onmessage = null;
      showTerminalError(`Не удалось восстановить psql-вывод: ${String(error)}`);
    }
  }
  renderSessions(sessions);
}

async function resizeSession(sessionId, cols, rows) {
  if (
      !Number.isInteger(cols)
      || !Number.isInteger(rows)
      || cols < 1
      || rows < 1
  ) {
    return;
  }

  try {
    await invoke(
        "resize_psql_session",
        {
          sessionId,
          cols,
          rows,
        },
    );
  } catch (error) {
    console.error(
        "PTY resize failed:",
        error,
    );
  }
}

function handlePsqlEvent(event) {
  const sessionId = event.session_id;

  if (event.kind === "output") {
    const view = terminalViews.get(sessionId);

    if (view?.ready) {
      view.terminal.write(event.data);
    } else {
      appendPendingTerminalOutput(
          sessionId,
          event.data,
      );
    }

    return;
  }

  if (event.kind === "output_gap") {
    showTerminalError("Часть вывода psql пропущена из-за перегрузки интерфейса.");
    return;
  }

  if (event.kind === "stream_ended") {
    // PTY EOF is not proof that the psql process has exited.
    return;
  }

  if (event.kind === "exit") {
    const session =
        sessionsById.get(sessionId);

    if (session) {
      session.running = false;
    }

    sessionChannels.delete(
        sessionId,
    );

    renderSessions(
        Array.from(
            sessionsById.values(),
        ),
    );

    void refreshSessions().catch((error) => {
      console.error(
          "Не удалось очистить завершённую psql-сессию:",
          error,
      );
    });
  }
}

function createPsqlChannel() {
  if (!Channel) {
    throw new Error(
        "Tauri Channel API недоступен. Терминальный transport не может быть запущен.",
    );
  }

  const channel =
      new Channel();

  channel.onmessage =
      handlePsqlEvent;

  return channel;
}

async function openConnectionSession(
    connectionId,
    caseId = null,
    forceNew = false,
) {
  const existing = forceNew
      ? null
      : activeSessionForConnectionCase(
          connectionId,
          caseId,
      );

  setMainView("sessions");

  if (existing) {
    if (!sessionChannels.has(existing.id)) {
      const onEvent = createPsqlChannel();
      await invoke("subscribe_psql_session", {
        sessionId: existing.id,
        connectionId: existing.connection_id,
        caseId: existing.case_id,
        onEvent,
      });
      sessionChannels.set(existing.id, onEvent);
    }
    activeSessionId =
        existing.id;

    renderSessions(
        Array.from(
            sessionsById.values(),
        ),
    );

    return;
  }

  byId("session-empty").hidden = true;
  byId("terminal-shell").hidden = false;
  clearTerminalError();

  ensureTerminalInfrastructure();

  let bootstrapView = null;

  if (!terminal || !fitAddon) {
    bootstrapView = createTerminalView();
    bootstrapView.host.hidden = false;
    terminal = bootstrapView.terminal;
    fitAddon = bootstrapView.fitAddon;
  }

  // A new PTY must start with the same geometry as the visible xterm.
  const {
    cols,
    rows,
  } = await fitTerminalStable();

  const onEvent =
      createPsqlChannel();

  const started =
      await invoke(
          "start_psql_session",
          {
            connectionId,
            caseId,
            cols,
            rows,
            forceNew,
            onEvent,
          },
      );

  const session = started.session;

  if (started.created) {
    const view = bootstrapView ?? createTerminalView();

    registerTerminalView(
        session.id,
        view,
    );

  } else if (bootstrapView) {
    bootstrapView.terminal.dispose();
    bootstrapView.host.remove();
    terminal = null;
    fitAddon = null;
  }

  // Rust now attaches this channel even when it reuses an existing PTY.
  sessionChannels.set(session.id, onEvent);
  sessionsById.set(
      session.id,
      session,
  );

  activeSessionId =
      session.id;

  renderSessions(
      Array.from(
          sessionsById.values(),
      ),
  );
}

function initSessionsUi() {
  byId("session-list").addEventListener("click", (event) => {
    const button = event.target.closest(
        "button[data-session-id]",
    );

    if (!button) {
      return;
    }

    activeSessionId = Number(
        button.dataset.sessionId,
    );

    renderSessions(
        Array.from(sessionsById.values()),
    );
  });

  byId("close-session").addEventListener("click", () => {
    if (activeSessionId === null) {
      return;
    }

    const id = activeSessionId;

    void action(async () => {
      await invoke(
          "close_psql_session",
          {
            sessionId: id,
          },
      );

      disposeTerminalView(id);
      sessionChannels.delete(id);

      activeSessionId = null;

      await refreshSessions();

      message("PostgreSQL-сессия закрыта.");
    });
  });
}