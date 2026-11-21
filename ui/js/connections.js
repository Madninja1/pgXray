"use strict";

function activeSessionsForConnection(connectionId) {
  return Array.from(sessionsById.values()).filter(
      (session) =>
          session.connection_id === connectionId
          && session.running,
  );
}

function connectionActionButton(
    label,
    actionName,
    id,
    className = "secondary",
) {
  const button = document.createElement("button");

  button.type = "button";
  button.textContent = label;
  button.className = className;
  button.dataset.connectionAction = actionName;
  button.dataset.connectionId = String(id);

  return button;
}

function renderConnectionStatus(statusElement, status) {
  statusElement.hidden = !status;

  if (!status) {
    statusElement.textContent = "";
    statusElement.className = "inline-status";
    return;
  }

  statusElement.textContent = status.text;
  statusElement.className = `inline-status ${status.kind}`;
}

function setConnectionStatus(id, kind, text) {
  const previousTimer = connectionStatusTimers.get(id);

  if (previousTimer) {
    clearTimeout(previousTimer);
    connectionStatusTimers.delete(id);
  }

  const status = { kind, text };

  connectionChecks.set(id, status);

  const element = document.querySelector(
      `[data-connection-status="${id}"]`,
  );

  if (element) {
    renderConnectionStatus(element, status);
  }

  if (kind === "success") {
    const timer = setTimeout(() => {
      connectionChecks.delete(id);
      connectionStatusTimers.delete(id);

      const currentElement = document.querySelector(
          `[data-connection-status="${id}"]`,
      );

      if (currentElement) {
        renderConnectionStatus(currentElement, null);
      }
    }, 10_000);

    connectionStatusTimers.set(id, timer);
  }
}

function renderConnections(connections) {
  const list = byId("connection-list");

  list.replaceChildren();

  if (connections.length === 0) {
    const empty = document.createElement("p");

    empty.className = "empty-state";
    empty.textContent = "Сохранённых подключений пока нет.";

    list.append(empty);
    return;
  }

  for (const connection of connections) {
    const card = document.createElement("article");
    const heading = document.createElement("div");
    const name = document.createElement("strong");
    const address = document.createElement("span");
    const details = document.createElement("div");
    const sessionState = document.createElement("div");
    const actions = document.createElement("div");
    const status = document.createElement("div");

    const activeSessions = activeSessionsForConnection(connection.id);
    const activeSession = activeSessions[0] ?? null;

    card.className = "connection-card";
    heading.className = "connection-card-heading";
    details.className = "connection-details";
    sessionState.className = activeSession
        ? "connection-session active"
        : "connection-session";
    actions.className = "row-actions";

    name.textContent = connection.name;

    address.textContent =
        `${connection.username}@${connection.host}:${connection.port}/${connection.database}`;

    details.textContent =
        `TLS: ${connection.sslmode} · пароль: ${
            connection.has_password ? "сохранён" : "не сохранён"
        }`;

    sessionState.textContent = activeSession
        ? `● Сессии ${activeSessions.length}/3 · ${activeSessions
            .map((session) => `#${session.id}`)
            .join(", ")} · стартуют read-only`
        : "Сессия не открыта";

    heading.append(name, address);

    const connectionButtons = [
      connectionActionButton(
          activeSession ? "Открыть сессию" : "Подключиться",
          "connect",
          connection.id,
          activeSession ? "secondary" : "",
      ),
    ];

    if (activeSession) {
      connectionButtons.push(
          connectionActionButton(
              "Открыть новое подключение",
              "connect-new",
              connection.id,
          ),
      );
    }

    actions.append(
        ...connectionButtons,
        connectionActionButton(
            "Проверить",
            "probe",
            connection.id,
        ),
        connectionActionButton(
            "Изменить",
            "edit",
            connection.id,
        ),
        connectionActionButton(
            "Удалить",
            "delete",
            connection.id,
            "danger",
        ),
    );

    status.dataset.connectionStatus = String(connection.id);
    renderConnectionStatus(
        status,
        connectionChecks.get(connection.id) ?? null,
    );

    card.append(
        heading,
        details,
        sessionState,
        actions,
        status,
    );

    list.append(card);
  }
}

function renderConnectionDependentControls(connections) {
  renderXidConnectionOptions(connections);
  renderPgssConnectionOptions(connections);
  renderAutovacuumConnectionOptions(connections);
  renderIoWalConnectionOptions(connections);
  renderIndexHealthConnectionOptions(connections);
  renderActivityConnectionOptions();

  if (typeof renderLocksConnectionOptions === "function") {
    renderLocksConnectionOptions();
  }

  if (typeof renderPgSettingsConnectionOptions === "function") {
    renderPgSettingsConnectionOptions();
  }
}

async function refreshConnections() {
  const connections = await invoke("list_connections");

  connectionsById = new Map(
      connections.map((connection) => [
        connection.id,
        connection,
      ]),
  );

  renderConnections(connections);
  renderConnectionDependentControls(connections);
  renderWorkspaceConnectionOptions(connections);

  // action() temporarily disables every button. During initial login or after
  // creating the first connection, that busy-state restoration can otherwise
  // overwrite the freshly calculated enabled state of monitoring/diagnostics.
  // Re-render availability on the next frame, after action() has left busy mode.
  requestAnimationFrame(() => {
    renderConnectionDependentControls(
        Array.from(connectionsById.values()),
    );
  });
}

function editingConnectionId() {
  const value = byId("connection-form").dataset.connectionId;

  return value ? Number(value) : null;
}

function clearConnectionFieldError(name) {
  const form = byId("connection-form");
  const field = form.elements[name];
  const error = form.querySelector(
      `[data-error-for="${name}"]`,
  );

  field?.classList.remove("field-invalid");

  if (error) {
    error.hidden = true;
    error.textContent = "";
  }
}

function setConnectionFieldError(name, text) {
  const form = byId("connection-form");
  const field = form.elements[name];
  const error = form.querySelector(
      `[data-error-for="${name}"]`,
  );

  field?.classList.add("field-invalid");

  if (error) {
    error.hidden = false;
    error.textContent = text;
  }
}

function clearConnectionFieldErrors() {
  for (const name of [
    "name",
    "host",
    "port",
    "database",
    "username",
    "password",
    "sslmode",
  ]) {
    clearConnectionFieldError(name);
  }
}

function validateConnectionForm() {
  const form = byId("connection-form");

  clearConnectionFieldErrors();

  const messages = {
    name: "Укажите название подключения.",
    host: "Укажите адрес PostgreSQL-сервера.",
    port: "Укажите порт от 1 до 65535.",
    database: "Укажите имя базы данных.",
    username: "Укажите пользователя PostgreSQL.",
  };

  let valid = true;

  for (const name of [
    "name",
    "host",
    "port",
    "database",
    "username",
  ]) {
    const field = form.elements[name];

    if (!field.checkValidity()) {
      setConnectionFieldError(
          name,
          messages[name] ?? field.validationMessage,
      );

      valid = false;
    }
  }

  if (!valid) {
    showConnectionEditorStatus(
        "error",
        "Исправьте выделенные поля.",
    );
  }

  return valid;
}

function markConnectionFailureFields(errorText) {
  const text = errorText.toLowerCase();

  if (
      text.includes("password authentication failed")
      || text.includes("аутентификац")
      || text.includes("парол")
  ) {
    setConnectionFieldError(
        "username",
        "Проверьте пользователя PostgreSQL.",
    );

    setConnectionFieldError(
        "password",
        "Проверьте пароль PostgreSQL.",
    );

    return;
  }

  if (
      text.includes("database")
      && (
          text.includes("does not exist")
          || text.includes("не существует")
      )
  ) {
    setConnectionFieldError(
        "database",
        "Проверьте имя базы данных.",
    );

    return;
  }

  if (
      text.includes("certificate")
      || text.includes("ssl")
      || text.includes("tls")
      || text.includes("verify-full")
  ) {
    setConnectionFieldError(
        "sslmode",
        "Проверьте режим TLS и сертификат.",
    );

    setConnectionFieldError(
        "host",
        "Для verify-full имя сервера должно совпадать с сертификатом.",
    );

    return;
  }

  if (
      text.includes("connection refused")
      || text.includes("timeout")
      || text.includes("timed out")
      || text.includes("could not connect")
      || text.includes("не удалось подключ")
      || text.includes("адрес")
      || text.includes("порт")
  ) {
    setConnectionFieldError(
        "host",
        "Проверьте адрес сервера.",
    );

    setConnectionFieldError(
        "port",
        "Проверьте порт PostgreSQL.",
    );
  }
}

function showConnectionEditorStatus(kind, text) {
  if (connectionEditorStatusTimer) {
    clearTimeout(connectionEditorStatusTimer);
    connectionEditorStatusTimer = null;
  }

  const element = byId("connection-editor-status");

  element.hidden = false;
  element.textContent = text;
  element.className = `inline-status ${kind}`;

  if (kind === "success") {
    connectionEditorStatusTimer = setTimeout(() => {
      connectionEditorStatusTimer = null;
      hideConnectionEditorStatus();
    }, 10_000);
  }
}

function hideConnectionEditorStatus() {
  if (connectionEditorStatusTimer) {
    clearTimeout(connectionEditorStatusTimer);
    connectionEditorStatusTimer = null;
  }

  const element = byId("connection-editor-status");

  element.hidden = true;
  element.textContent = "";
  element.className = "inline-status";
}

function resetConnectionEditor() {
  const form = byId("connection-form");

  form.reset();
  delete form.dataset.connectionId;

  byId("connection-editor").hidden = true;
  byId("connection-editor-title").textContent = "Новое подключение";
  byId("save-connection").textContent = "Сохранить подключение";
  byId("remove-password-row").hidden = true;

  form.elements.remove_password.checked = false;
  form.elements.password.placeholder = "";

  hideConnectionEditorStatus();
  clearConnectionFieldErrors();
}

function openNewConnectionEditor() {
  resetConnectionEditor();

  const form = byId("connection-form");

  byId("connection-editor").hidden = false;

  form.elements.host.value = "127.0.0.1";
  form.elements.port.value = "5432";
  form.elements.database.value = "postgres";
  form.elements.username.value = "postgres";
  form.elements.sslmode.value = "verify-full";

  form.elements.name.focus();
}

function openConnectionEditor(id) {
  const connection = connectionsById.get(id);

  if (!connection) {
    message("Подключение не найдено.", true);
    return;
  }

  resetConnectionEditor();

  const form = byId("connection-form");

  byId("connection-editor").hidden = false;
  form.dataset.connectionId = String(id);

  form.elements.name.value = connection.name;
  form.elements.host.value = connection.host;
  form.elements.port.value = connection.port;
  form.elements.database.value = connection.database;
  form.elements.username.value = connection.username;
  form.elements.sslmode.value = connection.sslmode;
  form.elements.password.value = "";
  form.elements.remove_password.checked = false;

  form.elements.password.placeholder = connection.has_password
      ? "Оставьте пустым, чтобы сохранить текущий пароль"
      : "Пароль не сохранён";

  byId("remove-password-row").hidden = !connection.has_password;
  byId("connection-editor-title").textContent =
      `Изменить: ${connection.name}`;
  byId("save-connection").textContent = "Сохранить изменения";

  form.scrollIntoView({
    behavior: "smooth",
    block: "start",
  });

  form.elements.name.focus();
}

function connectionFormValue() {
  const data = new FormData(byId("connection-form"));

  return {
    name: String(data.get("name") ?? "").trim(),
    input: {
      host: String(data.get("host") ?? "").trim(),
      port: Number(data.get("port")),
      database: String(data.get("database") ?? "").trim(),
      username: String(data.get("username") ?? "").trim(),
      password: String(data.get("password") ?? ""),
      sslmode: String(data.get("sslmode") ?? ""),
    },
  };
}

function initConnectionsUi() {
  for (const name of [
    "name",
    "host",
    "port",
    "database",
    "username",
    "password",
    "sslmode",
  ]) {
    byId("connection-form")
        .elements[name]
        .addEventListener("input", () => {
          clearConnectionFieldError(name);
          hideConnectionEditorStatus();
        });

    byId("connection-form")
        .elements[name]
        .addEventListener("change", () => {
          clearConnectionFieldError(name);
          hideConnectionEditorStatus();
        });
  }
  byId("open-connection-create").addEventListener(
      "click",
      openNewConnectionEditor,
  );

  byId("cancel-connection-edit").addEventListener(
      "click",
      resetConnectionEditor,
  );

  byId("connection-form").addEventListener("submit", (event) => {
    event.preventDefault();

    if (!validateConnectionForm()) {
      return;
    }

    clearConnectionFieldErrors();

    const { input } = connectionFormValue();
    const connectionId = editingConnectionId();

    const existing = connectionId !== null
        ? connectionsById.get(connectionId)
        : null;

    const removePassword =
        event.currentTarget.elements.remove_password.checked;

    const useSavedPassword = Boolean(
        existing
        && existing.has_password
        && input.password === ""
        && !removePassword,
    );

    showConnectionEditorStatus(
        "pending",
        "Проверяем подключение…",
    );

    void action(
        async () => {
          const result = await invoke(
              "probe_connection_draft",
              {
                connectionId,
                useSavedPassword,
                input,
              },
          );

          showConnectionEditorStatus(
              "success",
              `Подключение успешно.\n${result}`,
          );
        },
        {
          startMessage: null,
          onError: (error) => {
            markConnectionFailureFields(error);

            showConnectionEditorStatus(
                "error",
                `Не удалось подключиться.\n${error}`,
            );
          },
        },
    );
  });

  byId("save-connection").addEventListener("click", () => {
    const form = byId("connection-form");

    if (!validateConnectionForm()) {
      return;
    }

    const { name, input } = connectionFormValue();
    const id = editingConnectionId();

    const existing = id !== null
        ? connectionsById.get(id)
        : null;

    const removePassword =
        form.elements.remove_password.checked;

    const keepExistingPassword = Boolean(
        existing
        && existing.has_password
        && input.password === ""
        && !removePassword,
    );

    void action(async () => {
      if (id !== null) {
        await invoke(
            "update_saved_connection",
            {
              id,
              name,
              input,
              keepExistingPassword,
            },
        );
      } else {
        await invoke(
            "save_connection",
            {
              name,
              input,
            },
        );
      }

      resetConnectionEditor();
      await refreshConnections();

      if (typeof runPgSettingsScheduleCheck === "function") {
        void runPgSettingsScheduleCheck();
      }

      message(
          id !== null
              ? "Подключение обновлено."
              : `Подключение ${name} сохранено.`,
      );
    });
  });

  byId("connection-list").addEventListener("click", (event) => {
    const button = event.target.closest(
        "button[data-connection-action]",
    );

    if (!button) {
      return;
    }

    const id = Number(
        button.dataset.connectionId,
    );

    if (
        button.dataset.connectionAction === "connect"
        || button.dataset.connectionAction === "connect-new"
    ) {
      const forceNew =
          button.dataset.connectionAction === "connect-new";

      button.disabled = true;

      void openConnectionSession(
          id,
          null,
          forceNew,
      ).then(() => {
        const connection = connectionsById.get(id);

        message(
            `Сессия ${connection?.name ?? id} открыта.`,
        );
      }).catch((error) => {
        message(String(error), true);
      }).finally(() => {
        if (button.isConnected) {
          button.disabled = false;
        }
      });

      return;
    }

    if (button.dataset.connectionAction === "probe") {
      setConnectionStatus(
          id,
          "pending",
          "Проверяем подключение…",
      );

      void action(
          async () => {
            const result = await invoke(
                "probe_saved_connection",
                {
                  id,
                },
            );

            setConnectionStatus(
                id,
                "success",
                `Подключение успешно.\n${result}`,
            );
          },
          {
            startMessage: null,
            onError: (error) => {
              setConnectionStatus(
                  id,
                  "error",
                  `Не удалось подключиться.\n${error}`,
              );
            },
          },
      );

      return;
    }

    if (button.dataset.connectionAction === "edit") {
      openConnectionEditor(id);
      return;
    }

    if (button.dataset.connectionAction === "delete") {
      const connection = connectionsById.get(id);
      const label = connection?.name ?? `#${id}`;

      if (
          !window.confirm(
              `Удалить подключение "${label}"?\n\n`
              + "Будет безвозвратно удалено всё локальное состояние этого подключения:\n"
              + "• сохранённый пароль;\n"
              + "• история XID / pg_stat_statements / Autovacuum;\n"
              + "• Cases и все их artifacts;\n"
              + "• connection privacy scopes и mapping.json.\n\n"
              + "Project privacy scopes, не привязанные к подключению, не удаляются.",
          )
      ) {
        return;
      }

      void action(async () => {
        await invoke(
            "delete_saved_connection",
            {
              id,
            },
        );

        connectionChecks.delete(id);

        if (editingConnectionId() === id) {
          resetConnectionEditor();
        }

        await refreshConnections();

        message(
            `Подключение "${label}" и все его локальные данные удалены.`,
        );
      });
    }
  });
}
