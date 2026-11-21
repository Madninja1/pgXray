"use strict";

function setDiagnosticModule(name) {
  activeDiagnosticModule = name;

  document.querySelectorAll("[data-diagnostic]").forEach((button) => {
    const active = button.dataset.diagnostic === name;

    button.classList.toggle("active", active);
  });

  byId("xid-diagnostic").hidden = name !== "xid";
  byId("pgss-diagnostic").hidden = name !== "pgss";
  byId("autovacuum-diagnostic").hidden = name !== "autovacuum";
  byId("io-wal-diagnostic").hidden = name !== "io-wal";
  byId("index-health-diagnostic").hidden = name !== "index-health";
}

function setHistoryModule(name) {
  activeHistoryModule = name;

  document.querySelectorAll("[data-history-module]").forEach((button) => {
    const active = button.dataset.historyModule === name;

    button.classList.toggle("active", active);
  });

  byId("xid-history-panel").hidden = name !== "xid";
  byId("pgss-history-panel").hidden = name !== "pgss";
  byId("autovacuum-history-panel").hidden = name !== "autovacuum";
}

function initAppUi() {
  byId("registration-form").addEventListener("submit", (event) => {
    event.preventDefault();

    const form = event.currentTarget;
    const data = new FormData(form);

    void action(async () => {
      if (data.get("password") !== data.get("confirm")) {
        throw new Error("Пароли не совпадают");
      }

      const session = await invoke(
          "register_initial_user",
          {
            username: data.get("username"),
            password: data.get("password"),
          },
      );

      form.reset();
      await enterWorkspace(session);

      message(
          "Первый пользователь создан.",
      );
    }).then(() => {
      clearPasswordInputs(form);
    });
  });

  byId("login-form").addEventListener("submit", (event) => {
    event.preventDefault();

    const form = event.currentTarget;
    const data = new FormData(form);

    void action(async () => {
      const session = await invoke(
          "login",
          {
            username: data.get("username"),
            password: data.get("password"),
          },
      );

      form.reset();
      await enterWorkspace(session);

      message("Вы вошли в pgXray.");
    }).then(() => {
      clearPasswordInputs(form);
    });
  });
  document.querySelectorAll("[data-diagnostic]").forEach((button) => {
    button.addEventListener("click", () => {
      const name = button.dataset.diagnostic;

      setDiagnosticModule(name);

      if (name === "xid") {
        void loadXidReference();
      } else if (name === "pgss") {
        void loadPgssReference();
      } else if (name === "autovacuum") {
        void loadAutovacuumReference();
      } else if (name === "io-wal") {
        void loadIoWalReference();
      } else {
        void loadIndexHealthReference();
      }
    });
  });
  document.querySelectorAll("[data-history-module]").forEach((button) => {
    button.addEventListener("click", () => {
      const name = button.dataset.historyModule;

      setHistoryModule(name);

      void action(
          async () => {
            await refreshConnections();

            if (name === "xid") {
              await refreshXidHistory();
            } else if (name === "pgss") {
              await refreshPgssHistory();
            } else {
              await refreshAutovacuumHistory();
            }
          },
          {
            startMessage: null,
          },
      );
    });
  });
  document.querySelectorAll(".main-nav-item").forEach((button) => {
    button.addEventListener("click", () => {
      const view = button.dataset.view;

      if (view === "connections") {
        void action(async () => {
          setMainView("connections");

          await Promise.all([
            refreshConnections(),
            refreshSessions(),
          ]);

          message("Сохранённые подключения PostgreSQL.");
        });

        return;
      }

      if (view === "diagnostics") {
        void action(async () => {
          setMainView("diagnostics");

          await refreshConnections();

          if (activeDiagnosticModule === "xid") {
            await loadXidReference();
          } else if (activeDiagnosticModule === "pgss") {
            await loadPgssReference();
          } else if (activeDiagnosticModule === "autovacuum") {
            await loadAutovacuumReference();
          } else if (activeDiagnosticModule === "io-wal") {
            await loadIoWalReference();
          } else {
            await loadIndexHealthReference();
          }

          message("Диагностические модули pgXray.");
        });

        return;
      }

      if (view === "activity") {
        void action(async () => {
          setMainView("activity");
          await refreshConnections();
          renderActivityConnectionOptions();
          renderLocksConnectionOptions();
          renderPgSettingsConnectionOptions();
          setMonitoringTab(activeMonitoringTab);

          if (activeMonitoringTab === "settings") {
            await refreshPgSettingsMonitoring();
          } else if (activeMonitoringTab === "locks") {
            await refreshLocksCases();
          }

          message("Мониторинг activity, Locks / Waits и конфигурации PostgreSQL.");
        });

        return;
      }

      if (view === "history") {
        void action(async () => {
          setMainView("history");

          await refreshConnections();

          if (activeHistoryModule === "xid") {
            await refreshXidHistory();
          } else if (activeHistoryModule === "pgss") {
            await refreshPgssHistory();
          } else {
            await refreshAutovacuumHistory();
          }

          message("История диагностических snapshots.");
        });

        return;
      }

      if (view === "changes") {
        void action(async () => {
          setMainView("changes");
          await refreshConnections();
          await refreshWhatChangedView();
          message("What Changed? — baseline/incident period comparison.");
        });

        return;
      }

      if (view === "tools") {
        void action(async () => {
          setMainView("tools");

          await refreshTools();

          setToolsTab(
              activeToolsTab,
          );

          message("Cases, privacy и локальный PEV2.");
        });

        return;
      }

      if (view === "sessions") {
        void action(async () => {
          setMainView("sessions");

          await refreshSessions();

          message("Активные PostgreSQL-сессии.");
        });

        return;
      }

      if (view === "settings") {
        void action(async () => {
          setMainView("settings");

          hideUserEditors();
          await refreshUsers();
          await refreshSettingsExtra();

          message("Настройки pgXray.");
        });
      }
    });
  });

  byId("logout").addEventListener("click", () => {
    void action(async () => {
      await invoke("logout");

      currentUsername = null;

      connectionsById.clear();
      connectionChecks.clear();
      sessionsById.clear();
      disposeAllTerminalViews();
      sessionChannels.clear();
      xidHistorySnapshots = [];
      pgssHistorySnapshots = [];
      pgssPerformanceSamples = null;
      pgssServerVersionNum = null;
      whatChangedComparison = null;
      whatChangedMarkers = [];
      whatChangedHistoryConnections = [];
      investigationCases = [];
      casePrivacyScopes = [];
      privacyScopes = [];
      caseArtifacts = [];
      pev2Artifacts = [];
      activeCaseId = null;
      pendingPev2Payload = null;

      activeSessionId = null;
      resetActivityView();
      stopPgSettingsSchedule();
      resetPgSettingsView();
      setMonitoringTab("activity");

      byId("identity").textContent = "";

      resetConnectionEditor();
      hideUserEditors();
      resetXidSnapshot();
      setXidTab("snapshot");
      resetPgssSnapshot();
      setPgssTab("health");
      resetIoWalView();
      resetIndexHealthView();
      setDiagnosticModule("xid");
      setHistoryModule("xid");
      setToolsTab("cases");
      renderPrivacyMappingInfo(null);

      renderConnections([]);
      renderSessions([]);

      setMainView("connections");
      screen("login");

      message("Вы вышли из приложения.");
    });
  });
}

async function initialize() {
  try {
    await loadHtmlFragments();

    initConnectionsUi();
    initActivityUi();
    initLocksUi();
    initPgSettingsUi();
    initChangesUi();
    initSessionsUi();
    initXidUi();
    initPgssUi();
    initAutovacuumUi();
    initIoWalUi();
    initIndexHealthUi();
    initToolsUi();
    initSettingsUi();
    initSettingsExtraUi();
    initAppUi();
  } catch (error) {
    message(
        `Не удалось загрузить UI: ${String(error)}`,
        true,
    );

    return;
  }

  if (!invoke) {
      screen("login");

      message(
          "Для работы нужен запуск через cargo tauri dev.",
          true,
      );

      return;
    }

    await action(async () => {
      const state = await invoke("auth_state");

      if (state.session) {
        await enterWorkspace(
            state.session,
        );

        message(
            "Сессия pgXray активна.",
        );

        return;
      }

      if (state.needs_registration) {
        screen("registration");

        message(
            "Создайте первого пользователя pgXray.",
        );

        return;
      }

      screen("login");
      message("Войдите в pgXray.");
    });
}

void initialize();
