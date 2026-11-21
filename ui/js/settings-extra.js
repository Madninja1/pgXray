"use strict";

let activeSettingsTab = "users";
let historyConnectionRows = [];
let currentPathSettings = null;
let currentApplicationLogState = null;
let applicationLogEntries = [];

function setSettingsTab(name) {
    activeSettingsTab = name;

    document.querySelectorAll("[data-settings-tab]").forEach((button) => {
        const active = button.dataset.settingsTab === name;

        button.classList.toggle("active", active);
    });

    byId("settings-users-panel").hidden = name !== "users";
    byId("settings-interface-panel").hidden = name !== "interface";
    byId("settings-history-panel").hidden = name !== "history";
    byId("settings-logs-panel").hidden = name !== "logs";
    byId("settings-paths-panel").hidden = name !== "paths";
    byId("settings-readiness-panel").hidden = name !== "readiness";

    if (name === "readiness") {
        renderReadinessConnectionOptions();
    }
}

function themeDisplayName(theme) {
    return {
        "graphite-teal": "Graphite + Teal",
        "forest-mint": "Forest + Mint",
        "indigo-cyan": "Indigo + Cyan",
        light: "Light",
        custom: "Custom",
    }[theme] ?? theme;
}

function renderThemeSelection() {
    const current = readStoredThemeName();

    document.querySelectorAll("[data-theme-choice]").forEach((button) => {
        button.classList.toggle(
            "active",
            button.dataset.themeChoice === current,
        );
    });

    byId("theme-current").textContent =
        `Текущая тема: ${themeDisplayName(current)}`;

    byId("custom-theme-panel").hidden = current !== "custom";
}

function renderCustomThemeControls() {
    const root = byId("custom-theme-controls");

    root.replaceChildren();

    const saved = readCustomTheme();

    for (const [property, label, fallback] of CUSTOM_THEME_FIELDS) {
        const wrapper = document.createElement("label");
        const caption = document.createElement("span");
        const input = document.createElement("input");

        wrapper.className = "theme-color-field";
        caption.textContent = label;

        input.type = "color";
        input.value = saved[property] ?? fallback;
        input.dataset.themeProperty = property;

        wrapper.append(caption, input);
        root.append(wrapper);
    }
}

function collectCustomThemeControls() {
    const values = {};

    byId("custom-theme-controls")
        .querySelectorAll("input[data-theme-property]")
        .forEach((input) => {
            values[input.dataset.themeProperty] = input.value;
        });

    return values;
}

function renderHistoryConnections() {
    const body = byId("history-settings-connections");

    body.replaceChildren();

    if (historyConnectionRows.length === 0) {
        tableEmpty(
            body,
            6,
            "История диагностики пока не накоплена.",
        );

        return;
    }

    for (const row of historyConnectionRows) {
        const tr = document.createElement("tr");
        const live = connectionsById.has(row.connection_id);

        appendCells(
            tr,
            [
                `#${row.connection_id}`,
                row.connection_name,
                live ? "Сохранено" : "Подключение удалено",
                formatInteger(row.snapshot_count),
                formatSnapshotTime(row.oldest_at_ms),
                formatSnapshotTime(row.newest_at_ms),
            ],
        );

        const actions = document.createElement("td");
        const button = document.createElement("button");

        button.type = "button";
        button.className = "danger";
        button.textContent = "Удалить историю";
        button.dataset.deleteHistoryConnection = String(row.connection_id);
        button.dataset.deleteHistoryConnectionName = row.connection_name;

        actions.append(button);
        tr.append(actions);
        body.append(tr);
    }
}

async function refreshHistorySettings() {
    const [
        retention,
        rows,
    ] = await Promise.all([
        invoke("history_retention_settings"),
        invoke("list_history_connections"),
    ]);

    const select = byId("history-retention");
    const custom = byId("history-retention-custom");
    const days = retention.retention_days;

    if (days === null || days === undefined) {
        select.value = "forever";
        custom.hidden = true;
        custom.value = "";
    } else if (["7", "30", "90", "180", "365"].includes(String(days))) {
        select.value = String(days);
        custom.hidden = true;
        custom.value = "";
    } else {
        select.value = "custom";
        custom.hidden = false;
        custom.value = String(days);
    }

    historyConnectionRows = rows;
    renderHistoryConnections();
}

async function refreshPathSettings() {
    currentPathSettings = await invoke("path_settings");

    byId("export-directory").value = currentPathSettings.export_dir;
    byId("workspace-root").value = currentPathSettings.workspace_root;
    byId("application-log-directory").value =
        currentPathSettings.application_log_dir;
}

function applicationLogPhaseLabel(phase) {
    return {
        started: "запуск",
        succeeded: "успешно",
        failed: "ошибка",
        updated: "изменено",
        cleared: "очищено",
        exit_requested: "закрытие",
        exited: "завершено",
        slot_acquired: "slot получен",
        slot_acquire_timeout: "slot timeout",
        client_created: "client создан",
        client_reused: "client переиспользован",
        client_connect_failed: "ошибка подключения",
        client_retired: "client выведен",
        client_shutdown: "client закрыт",
        client_shutdown_deferred: "shutdown отложен",
        query_started: "query запущен",
        query_finished: "query завершён",
        query_timeout: "query timeout",
        query_failed: "query ошибка",
        invalidate_started: "invalidate запуск",
        invalidate_finished: "invalidate завершён",
        invalidate_deferred: "invalidate отложен",
    }[phase] ?? phase;
}

function renderApplicationLogEntries() {
    const body = byId("application-log-entries");

    body.replaceChildren();

    if (applicationLogEntries.length === 0) {
        tableEmpty(body, 7, "Журнал пока пуст.");
        return;
    }

    for (const entry of applicationLogEntries) {
        const row = document.createElement("tr");
        const details = [
            entry.request_id ? `request=${entry.request_id}` : null,
            entry.context,
            entry.error,
        ]
            .filter(Boolean)
            .join(" · ");

        appendCells(
            row,
            [
                formatSnapshotTime(entry.timestamp_ms),
                entry.actor || "—",
                entry.component,
                entry.operation,
                applicationLogPhaseLabel(entry.phase),
                entry.duration_ms === null || entry.duration_ms === undefined
                    ? "—"
                    : `${formatInteger(entry.duration_ms)} ms`,
                details || "—",
            ],
        );

        body.append(row);
    }
}

function renderApplicationLogState() {
    if (!currentApplicationLogState) {
        return;
    }

    const { settings, status } = currentApplicationLogState;
    const sizeSelect = byId("application-log-size-limit");
    const retentionSelect = byId("application-log-retention-days");

    sizeSelect.value = String(settings.max_total_bytes);
    retentionSelect.value = String(settings.retention_days);

    byId("application-log-health").textContent = status.available
        ? "Состояние: журнал доступен для записи."
        : `Состояние: журнал временно недоступен для записи${status.last_error ? ` · ${status.last_error}` : ""}`;
    byId("application-log-current-directory").textContent =
        `Каталог: ${status.directory}`;
    byId("application-log-usage").textContent =
        `Размер: ${formatBytes(status.total_bytes)} из ${formatBytes(settings.max_total_bytes)} · файлов: ${formatInteger(status.file_count)} · rotation: ${formatBytes(status.max_file_bytes)}`;

    const oldest = formatSnapshotTime(status.oldest_at_ms);
    const newest = formatSnapshotTime(status.newest_at_ms);
    byId("application-log-range").textContent =
        `Период: ${oldest} — ${newest} · retention: ${settings.retention_days} дн.`;

    byId("application-log-error").textContent = status.last_error
        ? `Последняя ошибка logger: ${status.last_error}`
        : "";
}

async function refreshApplicationLog() {
    currentApplicationLogState = await invoke("application_log_state");

    try {
        applicationLogEntries = await invoke(
            "list_application_log_entries",
            { limit: 200 },
        );
    } catch (_) {
        applicationLogEntries = [];
    }

    renderApplicationLogState();
    renderApplicationLogEntries();
}

function readinessStatusLabel(status) {
    return {
        ready: "Готово",
        supported: "Поддерживается",
        limited: "Ограничено",
        warning: "Предупреждение",
        unavailable: "Недоступно",
        unsupported: "Не поддерживается",
        unverified: "Не проверено",
        error: "Ошибка",
    }[status] ?? status;
}

function renderReadinessConnectionOptions() {
    const select = byId("readiness-connection");
    const selected = select.value;

    select.replaceChildren();

    const localOnly = document.createElement("option");
    localOnly.value = "";
    localOnly.textContent = "Только локальное состояние pgXray";
    select.append(localOnly);

    for (const connection of connectionsById.values()) {
        const option = document.createElement("option");
        option.value = String(connection.id);
        option.textContent = `${connection.name} · ${connection.host}:${connection.port}/${connection.database}`;
        select.append(option);
    }

    if ([...select.options].some((option) => option.value === selected)) {
        select.value = selected;
    }
}

function readinessCard(check) {
    const card = document.createElement("article");
    const heading = document.createElement("div");
    const title = document.createElement("strong");
    const status = document.createElement("span");
    const detail = document.createElement("p");

    card.className = `readiness-card readiness-${check.status}`;
    heading.className = "readiness-card-heading";
    title.textContent = check.label;
    status.className = "readiness-status";
    status.textContent = readinessStatusLabel(check.status);
    detail.textContent = check.detail;

    heading.append(title, status);
    card.append(heading, detail);
    return card;
}

function renderRuntimeReadiness(result) {
    byId("readiness-version").textContent =
        `pgXray ${result.app_version} · supported PostgreSQL: ${result.supported_postgres}`;

    const localRoot = byId("readiness-local-checks");
    localRoot.replaceChildren();
    for (const check of result.local_checks ?? []) {
        localRoot.append(readinessCard(check));
    }
    byId("readiness-local-section").hidden = false;

    const localErrors = (result.local_checks ?? [])
        .filter((check) => check.status === "error").length;
    const localWarnings = (result.local_checks ?? [])
        .filter((check) => check.status === "warning").length;

    const summary = byId("readiness-summary");
    summary.className = localErrors > 0
        ? "inline-status error"
        : "inline-status success";
    summary.textContent = localErrors > 0
        ? `Локальная проверка: ошибок ${localErrors}, предупреждений ${localWarnings}.`
        : `Локальная проверка пройдена${localWarnings > 0 ? `; предупреждений: ${localWarnings}` : ""}.`;

    const postgresSection = byId("readiness-postgres-section");
    if (!result.postgres) {
        postgresSection.hidden = true;
        return;
    }

    const postgres = result.postgres;
    postgresSection.hidden = false;
    byId("readiness-postgres-meta").textContent =
        `PostgreSQL ${postgres.server_version} · ${postgres.database} · ${postgres.username} · ${readinessStatusLabel(postgres.support_status)}. ${postgres.support_detail}`;

    const featuresRoot = byId("readiness-postgres-features");
    featuresRoot.replaceChildren();
    for (const feature of postgres.features ?? []) {
        featuresRoot.append(readinessCard(feature));
    }

    const warnings = byId("readiness-warning-list");
    warnings.replaceChildren();
    for (const warningText of postgres.warnings ?? []) {
        const item = document.createElement("li");
        item.textContent = warningText;
        warnings.append(item);
    }
    byId("readiness-warnings").hidden = warnings.children.length === 0;

    const unavailable = (postgres.features ?? [])
        .filter((feature) => ["unavailable", "unsupported", "error"].includes(feature.status))
        .length;
    const limited = (postgres.features ?? [])
        .filter((feature) => ["limited", "warning", "unverified"].includes(feature.status))
        .length;

    if (postgres.support_status !== "supported" || unavailable > 0) {
        summary.className = "inline-status error";
    } else if (limited > 0 || (postgres.warnings ?? []).length > 0 || localWarnings > 0) {
        summary.className = "inline-status pending";
    }

    summary.textContent += ` PostgreSQL: ${readinessStatusLabel(postgres.support_status)}; ограничено: ${limited}; недоступно: ${unavailable}.`;
}

async function runRuntimeReadiness() {
    const raw = byId("readiness-connection").value;
    const connectionId = raw ? Number(raw) : null;
    const result = await invoke("runtime_readiness", { connectionId });
    renderRuntimeReadiness(result);
}

async function refreshSettingsExtra() {
    renderThemeSelection();
    renderCustomThemeControls();

    await refreshConnections();
    renderReadinessConnectionOptions();
    await Promise.all([
        refreshHistorySettings(),
        refreshPathSettings(),
        refreshApplicationLog(),
    ]);
}

function selectedRetentionDays() {
    const value = byId("history-retention").value;

    if (value === "forever") {
        return null;
    }

    if (value === "custom") {
        const custom = Number(
            byId("history-retention-custom").value,
        );

        if (
            !Number.isInteger(custom)
            || custom < 1
            || custom > 36500
        ) {
            throw new Error(
                "Укажите retention от 1 до 36500 дней.",
            );
        }

        return custom;
    }

    return Number(value);
}

function initSettingsExtraUi() {
    document.querySelectorAll("[data-settings-tab]").forEach((button) => {
        button.addEventListener("click", () => {
            setSettingsTab(
                button.dataset.settingsTab,
            );
        });
    });

    byId("run-readiness").addEventListener("click", () => {
        void action(
            async () => {
                await runRuntimeReadiness();
                message("Проверка готовности завершена.");
            },
            {
                startMessage: "Проверяю local storage и PostgreSQL compatibility…",
            },
        );
    });

    document.querySelectorAll("[data-theme-choice]").forEach((button) => {
        button.addEventListener("click", () => {
            applyPgXrayTheme(
                button.dataset.themeChoice,
            );

            renderThemeSelection();

            if (
                button.dataset.themeChoice === "custom"
            ) {
                renderCustomThemeControls();
            }
        });
    });

    byId("custom-theme-controls").addEventListener("input", () => {
        const values = collectCustomThemeControls();

        writeCustomTheme(values);

        if (
            readStoredThemeName() === "custom"
        ) {
            applyCustomThemeVariables(values);
            syncTerminalTheme();
        }
    });

    byId("custom-theme-save").addEventListener("click", () => {
        const values = collectCustomThemeControls();

        saveCustomTheme(values);
        renderThemeSelection();

        message("Custom theme сохранена.");
    });

    byId("custom-theme-reset").addEventListener("click", () => {
        try {
            localStorage.removeItem(
                PGXRAY_CUSTOM_THEME_KEY,
            );
        } catch (_) {
            // Ignore.
        }

        renderCustomThemeControls();

        if (
            readStoredThemeName() === "custom"
        ) {
            applyCustomThemeVariables(
                readCustomTheme(),
            );
            syncTerminalTheme();
        }

        message("Custom theme сброшена к базовым значениям.");
    });

    byId("save-export-directory").addEventListener("click", () => {
        void action(
            async () => {
                const directory = byId("export-directory").value.trim();

                if (!directory) {
                    throw new Error("Укажите export directory.");
                }

                currentPathSettings = await invoke(
                    "set_export_directory",
                    { directory },
                );

                byId("export-directory").value = currentPathSettings.export_dir;
                message(`Export directory сохранён: ${currentPathSettings.export_dir}`);
            },
            {
                startMessage: null,
            },
        );
    });

    byId("migrate-workspace-root").addEventListener("click", () => {
        const directory = byId("workspace-root").value.trim();

        if (!directory) {
            message("Укажите workspace root.");
            return;
        }

        if (directory === currentPathSettings?.workspace_root) {
            message("Этот workspace root уже используется.");
            return;
        }

        if (
            !window.confirm(
                `Перенести workspace в:
${directory}

Все psql-сессии должны быть закрыты. Существующие cases, artifacts и privacy mappings будут скопированы в новый каталог. Старый workspace будет удалён после успешного переключения.`,
            )
        ) {
            return;
        }

        void action(
            async () => {
                const result = await invoke(
                    "migrate_workspace_root",
                    { directory },
                );

                currentPathSettings = result.settings;
                byId("workspace-root").value = currentPathSettings.workspace_root;

                message(
                    result.warning
                        ? `Workspace перенесён в ${currentPathSettings.workspace_root}. ${result.warning}`
                        : `Workspace перенесён в ${currentPathSettings.workspace_root}.`,
                );
            },
            {
                startMessage: null,
            },
        );
    });

    byId("migrate-application-log-directory").addEventListener("click", () => {
        const directory = byId("application-log-directory").value.trim();

        if (!directory) {
            message("Укажите application log directory.", true);
            return;
        }

        if (directory === currentPathSettings?.application_log_dir) {
            message("Этот application log directory уже используется.");
            return;
        }

        if (
            !window.confirm(
                `Перенести application log в:
${directory}

Существующие JSONL-файлы будут скопированы в новый каталог. Новый target должен отсутствовать или быть пустым.`,
            )
        ) {
            return;
        }

        void action(
            async () => {
                currentPathSettings = await invoke(
                    "set_application_log_directory",
                    { directory },
                );
                byId("application-log-directory").value =
                    currentPathSettings.application_log_dir;
                await refreshApplicationLog();
                message(
                    `Application log перенесён в ${currentPathSettings.application_log_dir}.`,
                );
            },
            {
                startMessage: null,
            },
        );
    });

    byId("save-application-log-settings").addEventListener("click", () => {
        void action(
            async () => {
                const maxTotalBytes = Number(
                    byId("application-log-size-limit").value,
                );
                const retentionDays = Number(
                    byId("application-log-retention-days").value,
                );

                currentApplicationLogState = await invoke(
                    "set_application_log_settings",
                    {
                        maxTotalBytes,
                        retentionDays,
                    },
                );

                await refreshApplicationLog();
                message(
                    `Настройки журнала сохранены: ${formatBytes(maxTotalBytes)}, ${retentionDays} дн.`,
                );
            },
            {
                startMessage: null,
            },
        );
    });

    byId("refresh-application-log").addEventListener("click", () => {
        void action(
            async () => {
                await refreshApplicationLog();
                message("Журнал приложения обновлён.");
            },
            {
                startMessage: null,
            },
        );
    });

    byId("reveal-application-log").addEventListener("click", () => {
        void action(
            async () => {
                const directory = await invoke(
                    "reveal_application_log_directory",
                );
                message(`Открыт каталог журнала: ${directory}`);
            },
            {
                startMessage: null,
            },
        );
    });

    byId("clear-application-log").addEventListener("click", () => {
        if (
            !window.confirm(
                "Очистить весь application log?\n\nБудут удалены накопленные JSONL-файлы журнала. Диагностическая история, cases и artifacts не изменятся.",
            )
        ) {
            return;
        }

        void action(
            async () => {
                currentApplicationLogState = await invoke(
                    "clear_application_log",
                );
                await refreshApplicationLog();
                message("Журнал приложения очищен.");
            },
            {
                startMessage: null,
            },
        );
    });

    byId("history-retention").addEventListener("change", () => {
        byId("history-retention-custom").hidden =
            byId("history-retention").value !== "custom";
    });

    byId("save-history-retention").addEventListener("click", () => {
        void action(
            async () => {
                const retentionDays = selectedRetentionDays();

                const result = await invoke(
                    "set_history_retention",
                    {
                        retentionDays,
                    },
                );

                await refreshHistorySettings();

                message(
                    retentionDays === null
                        ? "Автоматическое удаление старой истории отключено."
                        : `Retention сохранён: ${retentionDays} дн. Удалено snapshots: ${result.total_deleted}.`,
                );
            },
            {
                startMessage: null,
            },
        );
    });

    byId("history-settings-connections").addEventListener("click", (event) => {
        const button = event.target.closest(
            "button[data-delete-history-connection]",
        );

        if (!button) {
            return;
        }

        const connectionId = Number(
            button.dataset.deleteHistoryConnection,
        );

        const connectionName =
            button.dataset.deleteHistoryConnectionName;

        if (
            !window.confirm(
                `Удалить всю диагностическую историю для "${connectionName}" (#${connectionId})?\n\nБудут удалены XID, pg_stat_statements и Autovacuum snapshots. Сам профиль подключения, cases и artifacts не изменятся.`,
            )
        ) {
            return;
        }

        void action(
            async () => {
                const result = await invoke(
                    "delete_connection_history",
                    {
                        connectionId,
                    },
                );

                await refreshHistorySettings();

                xidHistorySnapshots = [];
                pgssHistorySnapshots = [];
                autovacuumHistorySnapshots = [];

                message(
                    `История подключения удалена. Snapshots: ${result.total_deleted}.`,
                );
            },
            {
                startMessage: null,
            },
        );
    });

    setSettingsTab(
        activeSettingsTab,
    );
}