"use strict";

let caseArtifactContexts = [];
let caseComparisons = [];
let caseInvestigationContext = null;
let privacyMappingEntries = [];
const privacyMappingSelection = new Set();
let privacyMappingSearchTimer = null;
let privacyMappingRequestId = 0;
let caseFileEditorArtifactId = null;
let selectedCaseArtifactId = null;
const inlineStatusTimers = new Map();

function setToolsTab(name) {
  activeToolsTab = name;

  document.querySelectorAll("[data-tools-tab]").forEach((button) => {
    const active = button.dataset.toolsTab === name;

    button.classList.toggle("active", active);
  });

  byId("tools-cases-panel").hidden = name !== "cases";
  byId("tools-privacy-panel").hidden = name !== "privacy";
  byId("tools-pev2-panel").hidden = name !== "pev2";

}

function setInlineStatus(id, kind, text) {
  const element = byId(id);
  const currentTimer = inlineStatusTimers.get(id);

  if (currentTimer) {
    window.clearTimeout(currentTimer);
    inlineStatusTimers.delete(id);
  }

  element.hidden = !text;
  element.textContent = text ?? "";
  element.className = `inline-status ${kind ?? ""}`.trim();

  if (text && kind === "success") {
    const timer = window.setTimeout(() => {
      element.hidden = true;
      element.textContent = "";
      element.className = "inline-status";
      inlineStatusTimers.delete(id);
    }, 5000);

    inlineStatusTimers.set(
        id,
        timer,
    );
  }
}

function clearInlineStatus(id) {
  setInlineStatus(
      id,
      "",
      "",
  );
}

function setFieldError(inputId, errorId, text) {
  const input = byId(inputId);
  const error = byId(errorId);

  input.classList.toggle(
      "field-invalid",
      Boolean(text),
  );

  input.setAttribute(
      "aria-invalid",
      String(Boolean(text)),
  );

  error.hidden = !text;
  error.textContent = text ?? "";
}

function clearFieldError(inputId, errorId) {
  setFieldError(
      inputId,
      errorId,
      "",
  );
}

function selectedPrivacyScope() {
  const scopeId = selectedPositiveId("privacy-scope");

  return privacyScopes.find(
      (scope) => scope.id === scopeId,
  ) ?? null;
}

const resultPrivacyActionOptions = [
  ["auto", "Auto"],
  ["preserve", "Preserve"],
  ["value", "Sensitive value"],
  ["table", "Table identifier"],
  ["column", "Column identifier"],
  ["columns", "Column list"],
  ["schema", "Schema identifier"],
  ["index", "Index identifier"],
  ["database", "Database identifier"],
  ["role", "Role identifier"],
  ["application", "Application identifier"],
  ["address", "Address identifier"],
  ["tablespace", "Tablespace identifier"],
  ["sql", "SQL"],
  ["sql_fragment", "SQL fragment"],
];

const resultPrivacyKeyActionOptions = [
  ["", "Preserve keys"],
  ["value", "Sensitive value"],
  ["table", "Table identifier"],
  ["column", "Column identifier"],
  ["schema", "Schema identifier"],
  ["index", "Index identifier"],
  ["database", "Database identifier"],
  ["role", "Role identifier"],
  ["application", "Application identifier"],
  ["address", "Address identifier"],
  ["tablespace", "Tablespace identifier"],
];

function normalizeResultPrivacyFieldName(value) {
  return value.trim().toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_+|_+$/g, "");
}

function resultPrivacyConfig(scope = selectedPrivacyScope()) {
  return scope?.result_privacy ?? {default: "auto", fields: {}, patterns: []};
}

function createResultPrivacySelect(options, value, datasetName, ariaLabel) {
  const select = document.createElement("select");
  select.dataset[datasetName] = "1";
  select.setAttribute("aria-label", ariaLabel);
  for (const [optionValue, label] of options) {
    const option = document.createElement("option");
    option.value = optionValue;
    option.textContent = label;
    select.append(option);
  }
  select.value = value ?? options[0]?.[0] ?? "";
  return select;
}

function createResultPrivacyRuleRow(kind = "field", selector = "", config = "value", depth = 0) {
  const row = document.createElement("div");
  row.className = "privacy-result-rule";
  row.dataset.resultPrivacyRule = "1";
  row.dataset.resultPrivacyDepth = String(depth);

  const header = document.createElement("div");
  header.className = "privacy-result-rule-header";

  const kindSelect = createResultPrivacySelect(
      [["field", "Поле"], ["regex", "Regex"], ["values", "Все значения"]],
      kind,
      "resultPrivacySelectorKind",
      "Как выбирать поле diagnostic result",
  );
  if (depth === 0) {
    kindSelect.querySelector('option[value="values"]')?.remove();
  }

  const selectorInput = document.createElement("input");
  selectorInput.type = "text";
  selectorInput.maxLength = kind === "regex" ? 512 : 128;
  selectorInput.placeholder = kind === "regex" ? "^(source|target)_table$" : "row_identity";
  selectorInput.value = selector;
  selectorInput.dataset.resultPrivacySelector = "1";
  selectorInput.setAttribute("aria-label", "Имя поля или regex diagnostic result");
  selectorInput.hidden = kind === "values";

  const structured = typeof config === "object" && config !== null;
  const modeSelect = createResultPrivacySelect(
      [["action", "Обычное поле"], ["json", "Вложенная структура / JSON"]],
      structured ? "json" : "action",
      "resultPrivacyMode",
      "Способ обработки result field",
  );
  const actionSelect = createResultPrivacySelect(
      resultPrivacyActionOptions,
      structured ? "preserve" : config,
      "resultPrivacyAction",
      "Действие для result field",
  );

  const removeButton = document.createElement("button");
  removeButton.type = "button";
  removeButton.className = "secondary";
  removeButton.textContent = "Удалить";
  removeButton.dataset.resultPrivacyRemove = "1";
  header.append(kindSelect, selectorInput, modeSelect, actionSelect, removeButton);
  row.append(header);

  const nested = document.createElement("div");
  nested.className = "privacy-result-nested";
  nested.dataset.resultPrivacyNested = "1";
  nested.hidden = !structured;

  const nestedConfig = structured ? config : {};
  const options = document.createElement("div");
  options.className = "privacy-result-nested-options";

  const jsonLabel = document.createElement("label");
  const jsonCheckbox = document.createElement("input");
  jsonCheckbox.type = "checkbox";
  jsonCheckbox.checked = Boolean(nestedConfig.json);
  jsonCheckbox.dataset.resultPrivacyJsonString = "1";
  jsonLabel.append(jsonCheckbox, document.createTextNode(" Значение — JSON строка (например JSONB cell из psql)"));

  const defaultLabel = document.createElement("label");
  defaultLabel.append(document.createTextNode("Внутри по умолчанию "));
  defaultLabel.append(createResultPrivacySelect(
      [["auto", "Auto"], ["preserve", "Preserve"]],
      nestedConfig.default ?? "preserve",
      "resultPrivacyNestedDefault",
      "Политика для ненастроенных вложенных полей",
  ));

  const keysLabel = document.createElement("label");
  keysLabel.append(document.createTextNode("Ключи объекта "));
  keysLabel.append(createResultPrivacySelect(
      resultPrivacyKeyActionOptions,
      nestedConfig.keys ?? "",
      "resultPrivacyKeys",
      "Анонимизация JSON object keys",
  ));
  options.append(jsonLabel, defaultLabel, keysLabel);

  const hint = document.createElement("p");
  hint.className = "hint privacy-result-rule-hint";
  hint.textContent = "Добавляйте точные поля, regex по имени ключа или правило «Все значения» для динамических map/array. Ключи сохраняются, пока явно не выбран другой режим.";

  const children = document.createElement("div");
  children.className = "privacy-result-child-rules";
  children.dataset.resultPrivacyChildren = "1";

  for (const [field, action] of Object.entries(nestedConfig.fields ?? {})) {
    children.append(createResultPrivacyRuleRow("field", field, action, depth + 1));
  }
  for (const rule of nestedConfig.patterns ?? []) {
    children.append(createResultPrivacyRuleRow("regex", rule.pattern ?? "", rule.action ?? "value", depth + 1));
  }
  if (nestedConfig.values !== undefined && nestedConfig.values !== null) {
    children.append(createResultPrivacyRuleRow("values", "", nestedConfig.values, depth + 1));
  }

  const add = document.createElement("button");
  add.type = "button";
  add.className = "secondary privacy-result-add-nested";
  add.textContent = "Добавить вложенное правило";
  add.dataset.resultPrivacyAddNested = "1";
  nested.append(options, hint, children, add);
  row.append(nested);

  actionSelect.hidden = structured;
  return row;
}

function setResultPrivacyRuleMode(row, mode) {
  const action = row.querySelector(":scope > .privacy-result-rule-header [data-result-privacy-action]");
  const nested = row.querySelector(":scope > [data-result-privacy-nested]");
  const structured = mode === "json";
  action.hidden = structured;
  nested.hidden = !structured;
}

function renderResultPrivacyPolicy() {
  const panel = byId("privacy-result-policy");
  const scope = selectedPrivacyScope();
  panel.hidden = false;

  const config = resultPrivacyConfig(scope);
  const list = byId("privacy-result-rule-list");
  byId("privacy-result-default").value = config.default ?? "auto";
  byId("privacy-result-policy-save").disabled = !scope;
  byId("privacy-result-add-rule").disabled = !scope;
  list.replaceChildren();

  for (const [field, action] of Object.entries(config.fields ?? {})) {
    list.append(createResultPrivacyRuleRow("field", field, action));
  }
  for (const rule of config.patterns ?? []) {
    list.append(createResultPrivacyRuleRow("regex", rule.pattern ?? "", rule.action ?? "value"));
  }

  if (list.children.length === 0) {
    const empty = document.createElement("p");
    empty.className = "hint privacy-result-rules-empty";
    empty.textContent = "Overrides не заданы.";
    list.append(empty);
  }
}

function collectResultPrivacyRule(row) {
  const header = row.querySelector(":scope > .privacy-result-rule-header");
  const kind = header.querySelector("[data-result-privacy-selector-kind]").value;
  const rawSelector = header.querySelector("[data-result-privacy-selector]").value.trim();
  const mode = header.querySelector("[data-result-privacy-mode]").value;

  let selector = rawSelector;
  if (kind === "field") {
    selector = normalizeResultPrivacyFieldName(rawSelector);
    if (!selector) throw new Error("Укажите имя поля для каждого exact override.");
  } else if (kind === "regex") {
    if (!selector) throw new Error("Укажите regex для каждого regex override.");
    try { new RegExp(selector); } catch (error) {
      throw new Error(`Некорректный regex ${selector}: ${error.message}`);
    }
  }

  if (mode === "action") {
    return {kind, selector, action: header.querySelector("[data-result-privacy-action]").value};
  }

  const nested = row.querySelector(":scope > [data-result-privacy-nested]");
  const action = {
    json: nested.querySelector("[data-result-privacy-json-string]").checked,
    default: nested.querySelector("[data-result-privacy-nested-default]").value,
    fields: {},
    patterns: [],
  };
  const keys = nested.querySelector("[data-result-privacy-keys]").value;
  if (keys) action.keys = keys;

  let valuesSeen = false;
  const exactSeen = new Set();
  for (const child of nested.querySelector(":scope > [data-result-privacy-children]").children) {
    if (!child.matches(".privacy-result-rule")) continue;
    const collected = collectResultPrivacyRule(child);
    if (collected.kind === "field") {
      if (exactSeen.has(collected.selector)) throw new Error(`Вложенное поле ${collected.selector} добавлено больше одного раза.`);
      exactSeen.add(collected.selector);
      action.fields[collected.selector] = collected.action;
    } else if (collected.kind === "regex") {
      action.patterns.push({pattern: collected.selector, action: collected.action});
    } else {
      if (valuesSeen) throw new Error("Правило «Все значения» можно задать только один раз в одном блоке.");
      valuesSeen = true;
      action.values = collected.action;
    }
  }
  return {kind, selector, action};
}

function collectResultPrivacyPolicy() {
  const fields = {};
  const patterns = [];
  const seen = new Set();

  for (const row of byId("privacy-result-rule-list").querySelectorAll(":scope > .privacy-result-rule")) {
    const rule = collectResultPrivacyRule(row);
    if (rule.kind === "field") {
      if (seen.has(rule.selector)) throw new Error(`Поле ${rule.selector} добавлено больше одного раза.`);
      seen.add(rule.selector);
      fields[rule.selector] = rule.action;
    } else if (rule.kind === "regex") {
      patterns.push({pattern: rule.selector, action: rule.action});
    }
  }

  return {default: byId("privacy-result-default").value, fields, patterns};
}

function addResultPrivacyRule() {
  const list = byId("privacy-result-rule-list");
  list.querySelector(".privacy-result-rules-empty")?.remove();
  const row = createResultPrivacyRuleRow();
  list.append(row);
  row.querySelector("[data-result-privacy-selector]").focus();
}

function renderPrivacyScopeActions() {
  const scope = selectedPrivacyScope();
  const resetButton = byId("privacy-reset-mapping");
  const deleteButton = byId("privacy-delete-scope");

  resetButton.disabled = !scope;
  deleteButton.disabled = !scope || scope.is_default;

  for (const id of [
    "privacy-mapping-search",
    "privacy-mapping-category",
    "privacy-mapping-refresh",
  ]) {
    const control = byId(id);

    if (control) {
      control.disabled = !scope;
    }
  }

  deleteButton.title = scope?.is_default
      ? "Default scope подключения удалить нельзя. Очистите mapping."
      : "";

  renderResultPrivacyPolicy();
}

function renderWorkspaceConnectionOptions(connections) {
  for (const id of [
    "case-connection",
    "privacy-connection",
  ]) {
    const select = byId(id);

    if (!select) {
      continue;
    }

    const previous = select.value;

    select.replaceChildren();

    if (connections.length === 0) {
      const option = document.createElement("option");

      option.value = "";
      option.textContent = "Нет сохранённых подключений";

      select.append(option);
      select.disabled = true;
      continue;
    }

    select.disabled = false;

    for (const connection of connections) {
      const option = document.createElement("option");

      option.value = String(connection.id);
      option.textContent =
          `${connection.name} — ${connection.username}@${connection.host}:${connection.port}/${connection.database}`;

      select.append(option);
    }

    if (
        previous
        && connections.some(
            (connection) => String(connection.id) === previous,
        )
    ) {
      select.value = previous;
    } else if (id === "case-connection") {
      const remembered = rememberedLastCase();

      if (
          remembered
          && connections.some(
              (connection) => connection.id === remembered.connectionId,
          )
      ) {
        select.value = String(remembered.connectionId);
      }
    }
  }
}

function selectedPositiveId(id) {
  const value = Number(
      byId(id)?.value,
  );

  return Number.isInteger(value) && value > 0
      ? value
      : null;
}

function scopeLabel(scope) {
  const kind = scope.is_default
      ? "default connection"
      : scope.connection_id === null
          ? "project"
          : "connection";

  return `${scope.name} · ${kind}`;
}

function renderScopeOptions(
    select,
    scopes,
    preferredId = null,
) {
  const previous = preferredId ?? Number(select.value);

  select.replaceChildren();

  if (scopes.length === 0) {
    const option = document.createElement("option");

    option.value = "";
    option.textContent = "Scopes не найдены";

    select.append(option);
    select.disabled = true;
    return;
  }

  select.disabled = false;

  for (const scope of scopes) {
    const option = document.createElement("option");

    option.value = String(scope.id);
    option.textContent = scopeLabel(scope);

    select.append(option);
  }

  const preferred = scopes.find(
      (scope) => scope.id === previous,
  ) ?? scopes.find(
      (scope) => scope.is_default,
  ) ?? scopes[0];

  select.value = String(preferred.id);
}

const CASE_ACTIVE_STORAGE_PREFIX = "pgxray.tools.activeCase";
const CASE_LAST_ACTIVE_STORAGE_KEY = "pgxray.tools.lastActiveCase";

function activeCaseStorageKey(connectionId) {
  return `${CASE_ACTIVE_STORAGE_PREFIX}.${connectionId}`;
}

function rememberedLastCase() {
  try {
    const parsed = JSON.parse(
        window.localStorage.getItem(CASE_LAST_ACTIVE_STORAGE_KEY) ?? "null",
    );
    const connectionId = Number(parsed?.connectionId);
    const caseId = Number(parsed?.caseId);

    return Number.isInteger(connectionId)
        && connectionId > 0
        && Number.isInteger(caseId)
        && caseId > 0
        ? {connectionId, caseId}
        : null;
  } catch {
    return null;
  }
}

function rememberedCaseId(connectionId) {
  try {
    const value = Number(
        window.localStorage.getItem(
            activeCaseStorageKey(connectionId),
        ),
    );

    return Number.isInteger(value) && value > 0
        ? value
        : null;
  } catch {
    return null;
  }
}

function rememberActiveCase(connectionId, caseId) {
  if (!connectionId || !caseId) {
    return;
  }

  try {
    window.localStorage.setItem(
        activeCaseStorageKey(connectionId),
        String(caseId),
    );
    window.localStorage.setItem(
        CASE_LAST_ACTIVE_STORAGE_KEY,
        JSON.stringify({connectionId, caseId}),
    );
  } catch {
    // Case selection remains usable without persistent WebView storage.
  }
}

function forgetActiveCase(connectionId, caseId) {
  if (!connectionId) {
    return;
  }

  try {
    const key = activeCaseStorageKey(connectionId);

    if (Number(window.localStorage.getItem(key)) === caseId) {
      window.localStorage.removeItem(key);
    }

    const last = rememberedLastCase();
    if (
        last?.connectionId === connectionId
        && last.caseId === caseId
    ) {
      window.localStorage.removeItem(CASE_LAST_ACTIVE_STORAGE_KEY);
    }
  } catch {
    // Nothing else to clean up.
  }
}

function resetCaseFileEditor() {
  caseFileEditorArtifactId = null;

  const editor = byId("case-file-editor");
  if (!editor) {
    return;
  }

  editor.hidden = true;
  byId("case-file-editor-title").textContent = "Новый файл";
  byId("case-file-name").value = "";
  byId("case-file-name").readOnly = false;
  byId("case-file-kind").value = "sql";
  byId("case-file-kind").disabled = false;
  byId("case-file-content").value = "";
  byId("case-file-save").textContent = "Создать файл";
}

function caseArtifactIsEditable(context) {
  const artifact = context?.artifact;

  return Boolean(
      artifact
      && artifact.visibility === "sensitive"
      && artifact.relative_path.includes("/source/")
      && ["sql", "text", "source", "import", "explain"].includes(artifact.kind)
      && !context.derivation,
  );
}

function openNewCaseFileEditor() {
  if (!activeCase()) {
    return;
  }

  resetCaseFileEditor();
  byId("case-file-editor").hidden = false;
  byId("case-file-content").value = "";
  byId("case-file-editor").scrollIntoView({behavior: "smooth", block: "center"});
  byId("case-file-name").focus();
}

async function openCaseFileEditor(artifactId) {
  const context = caseArtifactContexts.find(
      (entry) => entry.artifact.id === artifactId,
  );

  if (!caseArtifactIsEditable(context)) {
    throw new Error(
        "Встроенный редактор доступен только для локальных source artifacts.",
    );
  }

  const content = await invoke(
      "read_case_artifact",
      { artifactId },
  );

  caseFileEditorArtifactId = artifactId;
  byId("case-file-editor").hidden = false;
  byId("case-file-editor-title").textContent =
      `Редактирование #${artifactId} · ${context.artifact.original_name}`;
  byId("case-file-name").value = context.artifact.original_name;
  byId("case-file-name").readOnly = true;
  byId("case-file-kind").value = context.artifact.kind;
  byId("case-file-kind").disabled = true;
  byId("case-file-content").value = content;
  byId("case-file-save").textContent = "Сохранить изменения";

  byId("case-file-editor").scrollIntoView({behavior: "smooth", block: "center"});
  byId("case-file-content").focus();
}

function renderCaseList() {
  const list = byId("case-list");

  list.replaceChildren();

  if (investigationCases.length === 0) {
    const empty = document.createElement("p");

    empty.className = "empty-state";
    empty.textContent = "Для этого подключения cases пока нет.";

    list.append(empty);
    return;
  }

  for (const item of investigationCases) {
    const button = document.createElement("button");

    button.type = "button";
    button.className = "case-selector secondary";
    button.dataset.caseId = String(item.id);
    button.classList.toggle(
        "active",
        item.id === activeCaseId,
    );

    const title = document.createElement("strong");
    const meta = document.createElement("span");

    title.textContent = item.title;

    const createdAt = item.created_at_ms
        ? formatSnapshotTime(item.created_at_ms)
        : "дата неизвестна";

    meta.textContent =
        `#${item.id} · ${item.kind ?? "manual"} · ${createdAt} · scope ${item.scope_id} · ${item.status}`;

    button.append(title, meta);
    list.append(button);
  }
}

function activeCase() {
  return investigationCases.find(
      (item) => item.id === activeCaseId,
  ) ?? null;
}

function renderActiveCase() {
  const item = activeCase();

  byId("case-empty").hidden = Boolean(item);
  byId("case-content").hidden = !item;

  if (!item) {
    resetCaseFileEditor();
    clearInlineStatus("case-detail-status");
    return;
  }

  byId("case-title").textContent =
      item.title;

  const createdAt = item.created_at_ms
      ? formatSnapshotTime(item.created_at_ms)
      : "дата неизвестна";

  const detectedAt = item.detected_at_ms
      ? formatSnapshotTime(item.detected_at_ms)
      : null;
  const incidentMeta = item.kind === "incident"
      ? ` · incident${item.trigger ? ` · trigger ${item.trigger}` : ""}${detectedAt ? ` · detected ${detectedAt}` : ""}`
      : " · manual";

  byId("case-meta").textContent =
      `Case #${item.id}${incidentMeta} · создан ${createdAt} · connection #${item.connection_id} · privacy scope #${item.scope_id}`;

  const description = (item.description ?? "").trim();
  const descriptionDetails = byId("case-description-details");
  const descriptionEditor = byId("case-description-editor");
  const descriptionEdit = byId("case-description-edit");

  byId("case-description-preview").textContent =
      description || "Описание пока не добавлено.";

  byId("case-description").value =
      description;

  descriptionDetails.open = !description;
  descriptionEditor.hidden = Boolean(description);
  descriptionEdit.hidden = !description;

  byId("case-inbox-path").textContent =
      item.inbox_path;
}


function datetimeLocalValue(timestampMs) {
  if (!timestampMs) return "";
  const date = new Date(Number(timestampMs));
  if (Number.isNaN(date.getTime())) return "";
  const pad = (value) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function datetimeInputMs(id) {
  const value = byId(id)?.value;
  if (!value) return null;
  const timestamp = new Date(value).getTime();
  return Number.isFinite(timestamp) && timestamp > 0 ? timestamp : null;
}

function caseNavigationLabel(target) {
  switch (target?.kind) {
    case "query": return `Открыть queryid ${target.queryid ?? ""}`.trim();
    case "relation": return `Открыть relation OID ${target.relation_oid ?? ""}`.trim();
    case "pid": return `Открыть Locks / PID ${target.pid ?? ""}`.trim();
    case "settings": return "Открыть PostgreSQL settings";
    case "changes": return "Открыть What Changed?";
    case "locks": return "Открыть Locks / Waits";
    case "artifact": return "Открыть Plan Workspace";
    default: return "Открыть evidence";
  }
}

function renderCaseTimeline() {
  const target = byId("case-timeline");
  if (!target) return;
  target.replaceChildren();
  const events = caseInvestigationContext?.timeline ?? [];
  if (events.length === 0) {
    const empty = document.createElement("p");
    empty.className = "empty-state";
    empty.textContent = "Timeline пока пуст.";
    target.append(empty);
    return;
  }

  for (const item of events) {
    const card = document.createElement("article");
    card.className = "case-timeline-event";
    const header = document.createElement("div");
    header.className = "case-timeline-event-header";
    const title = document.createElement("strong");
    title.textContent = item.title;
    const meta = document.createElement("span");
    meta.className = "case-timeline-event-meta";
    meta.textContent = `${formatSnapshotTime(item.occurred_at_ms)} · ${item.source} · ${item.evidence_level}`;
    header.append(title, meta);
    card.append(header);

    if (item.detail) {
      const detail = document.createElement("p");
      detail.textContent = item.detail;
      card.append(detail);
    }
    if (item.navigation || item.kind === "intervention") {
      const actions = document.createElement("div");
      actions.className = "case-timeline-event-actions";
      if (item.navigation) {
        const button = document.createElement("button");
        button.type = "button";
        button.className = "secondary";
        button.dataset.caseNavigation = JSON.stringify(item.navigation);
        button.textContent = caseNavigationLabel(item.navigation);
        actions.append(button);

        if (item.navigation.artifact_id) {
          const evidence = document.createElement("button");
          evidence.type = "button";
          evidence.className = "secondary";
          evidence.dataset.caseEvidenceArtifact = String(item.navigation.artifact_id);
          evidence.textContent = `Показать evidence #${item.navigation.artifact_id}`;
          actions.append(evidence);
        }
      }
      if (item.kind === "intervention") {
        const interventionId = Number(String(item.id).split(":").pop());
        if (Number.isInteger(interventionId) && interventionId > 0) {
          const remove = document.createElement("button");
          remove.type = "button";
          remove.className = "secondary";
          remove.dataset.deleteIntervention = String(interventionId);
          remove.textContent = "Удалить intervention";
          actions.append(remove);
        }
      }
      card.append(actions);
    }
    target.append(card);
  }
}

function renderCaseInterventions() {
  const select = byId("case-impact-intervention");
  if (!select) return;
  const previous = select.value;
  const interventions = caseInvestigationContext?.interventions ?? [];
  select.replaceChildren();
  if (interventions.length === 0) {
    const option = document.createElement("option");
    option.value = "";
    option.textContent = "Сначала добавьте intervention";
    select.append(option);
    select.disabled = true;
    byId("case-impact-save").disabled = true;
  } else {
    select.disabled = false;
    byId("case-impact-save").disabled = false;
    for (const item of interventions) {
      const option = document.createElement("option");
      option.value = String(item.id);
      option.textContent = `#${item.id} · ${formatSnapshotTime(item.occurred_at_ms)} · ${item.title}`;
      select.append(option);
    }
    if (interventions.some((item) => String(item.id) === previous)) {
      select.value = previous;
    }
    seedCaseImpactPeriods(Number(select.value));
  }
}

function seedCaseImpactPeriods(interventionId, force = false) {
  const intervention = (caseInvestigationContext?.interventions ?? [])
      .find((item) => item.id === Number(interventionId));
  if (!intervention) return;
  const fields = [
    "case-impact-before-from",
    "case-impact-before-to",
    "case-impact-after-from",
    "case-impact-after-to",
  ];
  if (!force && fields.some((id) => byId(id)?.value)) return;
  const at = Number(intervention.occurred_at_ms);
  const windowMs = 30 * 60 * 1000;
  byId("case-impact-before-from").value = datetimeLocalValue(at - windowMs);
  byId("case-impact-before-to").value = datetimeLocalValue(at);
  byId("case-impact-after-from").value = datetimeLocalValue(at);
  byId("case-impact-after-to").value = datetimeLocalValue(at + windowMs);
}

function changeImpactHighlights(impact) {
  const rows = [];
  const changes = impact.result?.comparison?.pgss?.statement_changes ?? [];
  const interesting = new Set([
    "exec_ms_per_call",
    "calls_per_hour",
    "temp_writes_per_call",
    "temp_reads_per_call",
    "wal_bytes_per_call",
  ]);
  for (const statement of changes) {
    for (const metric of statement.metrics ?? []) {
      if (
        !interesting.has(metric.id)
        || metric.baseline_value === null || metric.baseline_value === undefined
        || metric.incident_value === null || metric.incident_value === undefined
      ) {
        continue;
      }
      rows.push(
          `queryid ${statement.queryid} · ${metric.label}: ${formatNumber(metric.baseline_value)} → ${formatNumber(metric.incident_value)} ${metric.unit ?? ""}`.trim(),
      );
      if (rows.length >= 6) return rows;
    }
  }
  return rows;
}

function renderCaseChangeImpacts() {
  const target = byId("case-impact-list");
  if (!target) return;
  target.replaceChildren();
  const impacts = caseInvestigationContext?.change_impacts ?? [];
  if (impacts.length === 0) {
    const empty = document.createElement("p");
    empty.className = "empty-state";
    empty.textContent = "Before/after verification ещё не сохранена.";
    target.append(empty);
    return;
  }

  for (const impact of impacts) {
    const card = document.createElement("article");
    card.className = "case-impact-card";
    const header = document.createElement("div");
    header.className = "case-impact-card-header";
    const title = document.createElement("strong");
    title.textContent = impact.result?.assessment?.label ?? "Change Impact";
    const meta = document.createElement("span");
    meta.className = "case-impact-meta";
    meta.textContent = `#${impact.id} · intervention #${impact.intervention_id}`;
    header.append(title, meta);
    const detail = document.createElement("p");
    const workload = impact.result?.assessment?.workload_shift_observed
        ? " · workload shift observed"
        : "";
    detail.textContent = `Before ${formatSnapshotTime(impact.before_from_ms)}–${formatSnapshotTime(impact.before_to_ms)} → After ${formatSnapshotTime(impact.after_from_ms)}–${formatSnapshotTime(impact.after_to_ms)}${workload}. Causal attribution is not established.`;
    const highlights = changeImpactHighlights(impact);
    const simultaneous = impact.result?.other_simultaneous_changes ?? {};
    if (highlights.length > 0) {
      const list = document.createElement("ul");
      list.className = "case-impact-highlights";
      for (const value of highlights) {
        const item = document.createElement("li");
        item.textContent = value;
        list.append(item);
      }
      card.append(list);
    }
    const simultaneousCount = [
      simultaneous.settings,
      simultaneous.autovacuum_relations,
      simultaneous.storage_relations,
      simultaneous.index_changes,
    ].reduce((sum, value) => sum + (Number(value) || 0), 0);
    if (simultaneousCount > 0) {
      const note = document.createElement("p");
      note.className = "hint";
      note.textContent = `Параллельные изменения: settings ${simultaneous.settings ?? 0}, autovacuum ${simultaneous.autovacuum_relations ?? 0}, storage ${simultaneous.storage_relations ?? 0}, indexes ${simultaneous.index_changes ?? 0}.`;
      card.append(note);
    }
    const actions = document.createElement("div");
    actions.className = "case-impact-actions";
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "secondary";
    remove.dataset.deleteCaseImpact = String(impact.id);
    remove.textContent = "Удалить verification";
    actions.append(remove);
    card.append(header, detail, actions);
    target.append(card);
  }
}

function renderCaseFingerprint() {
  const target = byId("case-fingerprint");
  const similar = byId("case-similar-incidents");
  if (!target || !similar) return;
  target.replaceChildren();
  similar.replaceChildren();
  const fingerprint = caseInvestigationContext?.fingerprint ?? {};
  const values = [];
  if (fingerprint.dominant_wait_event) values.push(`wait=${fingerprint.dominant_wait_event}`);
  for (const value of fingerprint.root_blocker_queryids ?? []) values.push(`root queryid=${value}`);
  for (const value of fingerprint.root_blocker_applications ?? []) values.push(`app=${value}`);
  for (const value of fingerprint.relation_oids ?? []) values.push(`relation=${value}`);
  for (const value of fingerprint.regressed_queryids ?? []) values.push(`regressed queryid=${value}`);
  for (const value of fingerprint.changed_settings ?? []) values.push(`setting=${value}`);

  const card = document.createElement("article");
  card.className = "case-fingerprint-card";
  if (values.length === 0) {
    const empty = document.createElement("p");
    empty.className = "hint";
    empty.textContent = "Fingerprint появится после lock / What Changed evidence.";
    card.append(empty);
  } else {
    const heading = document.createElement("strong");
    heading.textContent = "Current fingerprint";
    const valuesElement = document.createElement("div");
    valuesElement.className = "case-fingerprint-values";
    for (const value of values) {
      const code = document.createElement("code");
      code.textContent = value;
      valuesElement.append(code);
    }
    card.append(heading, valuesElement);
  }
  target.append(card);

  const matches = caseInvestigationContext?.similar_incidents ?? [];
  if (matches.length === 0) {
    const empty = document.createElement("p");
    empty.className = "empty-state";
    empty.textContent = "Похожих incident cases по текущим признакам не найдено.";
    similar.append(empty);
    return;
  }
  for (const match of matches) {
    const matchCard = document.createElement("article");
    matchCard.className = "case-similar-card";
    const header = document.createElement("div");
    header.className = "case-similar-card-header";
    const title = document.createElement("strong");
    title.textContent = `#${match.case_id} · ${match.title}`;
    const meta = document.createElement("span");
    meta.className = "case-similar-meta";
    meta.textContent = match.detected_at_ms ? formatSnapshotTime(match.detected_at_ms) : "time unknown";
    header.append(title, meta);
    const reasons = document.createElement("p");
    reasons.textContent = (match.matches ?? []).join("; ");
    const actions = document.createElement("div");
    actions.className = "case-similar-actions";
    const open = document.createElement("button");
    open.type = "button";
    open.className = "secondary";
    open.dataset.openSimilarCase = String(match.case_id);
    open.textContent = "Открыть case";
    actions.append(open);
    matchCard.append(header, reasons, actions);
    similar.append(matchCard);
  }
}

function renderCaseInvestigation() {
  renderCaseTimeline();
  renderCaseInterventions();
  renderCaseChangeImpacts();
  renderCaseFingerprint();

  const counts = {
    "case-timeline-count": caseInvestigationContext?.timeline?.length ?? 0,
    "case-intervention-count": caseInvestigationContext?.interventions?.length ?? 0,
    "case-impact-count": caseInvestigationContext?.change_impacts?.length ?? 0,
    "case-similar-count": caseInvestigationContext?.similar_incidents?.length ?? 0,
  };
  for (const [id, count] of Object.entries(counts)) {
    const target = byId(id);
    if (target) target.textContent = String(count);
  }

  const timeInput = byId("case-intervention-time");
  if (timeInput && !timeInput.value) {
    timeInput.value = datetimeLocalValue(Date.now());
  }
}

async function openCaseNavigation(target) {
  const item = activeCase();
  if (!item || !target) return;
  const connectionId = item.connection_id;
  await refreshConnections();
  const connections = Array.from(connectionsById.values());

  if (target.kind === "artifact" && target.artifact_id) {
    await openPlanWorkspaceForArtifact(Number(target.artifact_id), item.id);
    return;
  }
  if (target.kind === "changes") {
    setMainView("changes");
    await refreshWhatChangedView();
    byId("changes-connection").value = String(connectionId);
    await Promise.all([refreshWhatChangedMarkers(), refreshWhatChangedCases()]);
    return;
  }
  if (target.kind === "settings") {
    setMainView("activity");
    setMonitoringTab("settings");
    renderPgSettingsConnectionOptions();
    byId("pg-settings-connection").value = String(connectionId);
    await refreshPgSettingsMonitoring();
    return;
  }
  if (target.kind === "locks" || target.kind === "pid") {
    setMainView("activity");
    setMonitoringTab("locks");
    renderLocksConnectionOptions();
    byId("locks-connection").value = String(connectionId);
    await refreshLocksCases();
    await refreshLocksSnapshot();
    if (target.pid) {
      message(`Открыт live Locks / Waits для evidence PID ${target.pid}. Исторический PID уже мог исчезнуть.`);
    }
    return;
  }
  if (target.kind === "relation" && target.relation_oid) {
    setMainView("diagnostics");
    setDiagnosticModule("autovacuum");
    renderAutovacuumConnectionOptions(connections);
    byId("autovacuum-connection").value = String(connectionId);
    await loadAutovacuumReference();
    await loadAutovacuumTableDetail(Number(target.relation_oid));
    return;
  }
  if (target.kind === "query" && target.queryid) {
    setMainView("diagnostics");
    setDiagnosticModule("pgss");
    renderPgssConnectionOptions(connections);
    byId("pgss-connection").value = String(connectionId);
    await loadPgssReference();
    if (Number.isInteger(Number(target.snapshot_id)) && Number(target.snapshot_id) > 0) {
      pgssCurrentSnapshotId = Number(target.snapshot_id);
    } else {
      const snapshots = await invoke("list_pgss_snapshots", {connectionId, limit: 1});
      pgssCurrentSnapshotId = snapshots[0]?.id ?? null;
    }
    if (
      target.userid === null || target.userid === undefined
      || target.dbid === null || target.dbid === undefined
      || target.toplevel === null || target.toplevel === undefined
    ) {
      throw new Error(`Для queryid ${target.queryid} в evidence не хватает identity fields.`);
    }
    await loadPgssStatementInvestigation({
      userid: Number(target.userid),
      dbid: Number(target.dbid),
      queryid: String(target.queryid),
      toplevel: Boolean(target.toplevel),
    });
  }
}

const CASE_LIST_STORAGE_KEY =
    "pgxray.tools.caseListCollapsed";

function setCaseListCollapsed(collapsed) {
  const layout = byId("case-layout");
  const caseListPanel = byId("case-list-panel");
  const button = byId("toggle-case-sidebar");

  if (!layout || !caseListPanel || !button) {
    return;
  }

  caseListPanel.hidden = collapsed;

  layout.classList.toggle(
      "case-list-collapsed",
      collapsed,
  );

  button.textContent = collapsed
      ? "Показать cases"
      : "Скрыть cases";

  button.setAttribute(
      "aria-expanded",
      String(!collapsed),
  );

  try {
    window.localStorage.setItem(
        CASE_LIST_STORAGE_KEY,
        collapsed ? "1" : "0",
    );
  } catch {
    // UI продолжает работать и без persistent state.
  }
}

function restoreCaseListState() {
  let collapsed = false;

  try {
    collapsed =
        window.localStorage.getItem(
            CASE_LIST_STORAGE_KEY,
        ) === "1";
  } catch {
    // Используем открытое состояние по умолчанию.
  }

  setCaseListCollapsed(collapsed);
}

const ARTIFACT_ROLES = [
  "",
  "query",
  "ddl",
  "plan",
  "reference",
  "note",
  "other",
];

let artifactRoleMenuState = null;

function closeArtifactRoleMenu(restoreFocus = false) {
  if (!artifactRoleMenuState) {
    return;
  }

  const { menu, trigger } = artifactRoleMenuState;

  artifactRoleMenuState = null;
  menu.remove();
  trigger.setAttribute("aria-expanded", "false");

  if (restoreFocus && document.contains(trigger)) {
    trigger.focus();
  }
}

function positionArtifactRoleMenu(trigger, menu) {
  const viewportMargin = 8;
  const gap = 6;
  const triggerRect = trigger.getBoundingClientRect();

  const width = Math.min(
      Math.max(triggerRect.width, 220),
      window.innerWidth - viewportMargin * 2,
  );

  menu.style.width = `${width}px`;
  menu.style.maxHeight =
      `${Math.max(120, window.innerHeight - viewportMargin * 2)}px`;

  const menuHeight = menu.getBoundingClientRect().height;
  const spaceBelow =
      window.innerHeight - triggerRect.bottom - viewportMargin;
  const spaceAbove =
      triggerRect.top - viewportMargin;

  const openAbove =
      spaceBelow < menuHeight
      && spaceAbove > spaceBelow;

  let top = openAbove
      ? triggerRect.top - menuHeight - gap
      : triggerRect.bottom + gap;

  top = Math.max(
      viewportMargin,
      Math.min(
          top,
          window.innerHeight - menuHeight - viewportMargin,
      ),
  );

  const left = Math.max(
      viewportMargin,
      Math.min(
          triggerRect.left,
          window.innerWidth - width - viewportMargin,
      ),
  );

  menu.style.top = `${top}px`;
  menu.style.left = `${left}px`;
}

function openArtifactRoleMenu(trigger) {
  if (
      artifactRoleMenuState?.trigger === trigger
  ) {
    closeArtifactRoleMenu();
    return;
  }

  closeArtifactRoleMenu();

  const artifactId = Number(
      trigger.dataset.artifactRoleId,
  );

  const currentRole =
      trigger.dataset.artifactRole ?? "";

  const menu = document.createElement("div");

  menu.className = "artifact-role-menu";
  menu.setAttribute("role", "menu");
  menu.setAttribute(
      "aria-label",
      "Роль artifact",
  );

  for (const role of ARTIFACT_ROLES) {
    const option = document.createElement("button");

    option.type = "button";
    option.className = "artifact-role-menu-item";
    option.dataset.artifactRoleOption = role;
    option.setAttribute("role", "menuitemradio");
    option.setAttribute(
        "aria-checked",
        String(role === currentRole),
    );
    option.classList.toggle(
        "active",
        role === currentRole,
    );
    option.textContent = roleLabel(role);

    menu.append(option);
  }

  document.body.append(menu);

  artifactRoleMenuState = {
    artifactId,
    menu,
    trigger,
  };

  trigger.setAttribute("aria-expanded", "true");

  positionArtifactRoleMenu(
      trigger,
      menu,
  );
}

function artifactPrivacyMode(artifact) {
  const name = artifact.original_name.toLowerCase();

  if (
      artifact.kind === "explain"
  ) {
    return "explain";
  }

  if (
      artifact.kind === "sql"
      || name.endsWith(".sql")
  ) {
    return "sql";
  }

  return "result";
}

function artifactMode(artifact) {
  const name = artifact.original_name.toLowerCase();

  if (
      artifact.kind === "explain"
      || name.endsWith(".json")
  ) {
    return "explain";
  }

  if (
      artifact.kind === "sql"
      || name.endsWith(".sql")
  ) {
    return "sql";
  }

  return "text";
}

function artifactContextForId(artifactId) {
  return caseArtifactContexts.find(
      (item) => item.artifact.id === artifactId,
  ) ?? null;
}

function comparisonUsageForArtifact(artifactId) {
  return caseComparisons.filter(
      (comparison) => comparison.members.some(
          (member) => member.artifact_id === artifactId,
      ),
  );
}

function staleComparisonUsageForArtifact(artifactId) {
  return comparisonUsageForArtifact(artifactId).filter(
      (comparison) => comparison.members.some(
          (member) => member.artifact_id === artifactId && member.stale,
      ),
  );
}

function roleLabel(role) {
  return {
    query: "SQL query",
    ddl: "DDL / schema",
    plan: "Execution plan",
    reference: "Reference",
    note: "Note",
    other: "Other",
  }[role] ?? "Без роли";
}

function roleDescription(role) {
  return {
    query: "SQL query — запрос или его фрагмент. Baseline/candidate задаются конкретным comparison.",
    ddl: "DDL / schema — VIEW, функция, индекс или другая структура, нужная для понимания case.",
    plan: "Execution plan — сохранённый EXPLAIN / EXPLAIN ANALYZE. Сторона задаётся comparison.",
    reference: "Reference — дополнительный технический контекст: настройки, статистика, описание объекта.",
    note: "Note — локальные заметки по расследованию.",
    other: "Other — artifact, которому не подходит более точная роль.",
  }[role] ?? "Без роли — семантика artifact пока не назначена.";
}

function artifactCanOpenPlan(artifact) {
  return artifactMode(artifact) === "explain"
      && artifact.visibility === "sensitive";
}

function artifactActionButton(artifact, label, actionName, className = "secondary") {
  const button = document.createElement("button");
  button.type = "button";
  button.className = className;
  button.textContent = label;
  button.dataset.caseArtifactAction = actionName;
  button.dataset.artifactId = String(artifact.id);
  button.dataset.caseId = String(artifact.case_id);
  return button;
}

function closeCaseArtifactViewer() {
  selectedCaseArtifactId = null;
  const viewer = byId("case-artifact-viewer");
  if (viewer) viewer.hidden = true;
  document.querySelectorAll(".case-artifact-item.active").forEach(
      (item) => item.classList.remove("active"),
  );
}

async function openCaseArtifactViewer(artifactId) {
  const context = artifactContextForId(artifactId);
  if (!context) return;

  const artifact = context.artifact;
  const content = await invoke(
      "read_case_artifact",
      { artifactId },
  );

  selectedCaseArtifactId = artifactId;
  byId("case-artifact-viewer").hidden = false;
  byId("case-artifact-viewer-title").textContent = artifact.original_name;
  byId("case-artifact-viewer-meta").textContent =
      `#${artifact.id} · ${artifact.kind} · ${roleLabel(context.role)} · ${artifact.visibility} · ${formatBytes(artifact.size_bytes)}`;
  byId("case-artifact-preview").textContent = content;

  const actions = byId("case-artifact-viewer-actions");
  actions.replaceChildren();

  if (caseArtifactIsEditable(context)) {
    actions.append(artifactActionButton(artifact, "Редактировать", "edit"));
  }
  actions.append(artifactActionButton(artifact, "Копировать", "copy"));
  actions.append(artifactActionButton(artifact, "В privacy", "privacy"));
  if (artifactCanOpenPlan(artifact)) {
    actions.append(artifactActionButton(artifact, "Открыть план", "plan"));
  }
  actions.append(artifactActionButton(artifact, "Экспорт", "export"));
  actions.append(artifactActionButton(artifact, "Папка", "reveal"));
  actions.append(artifactActionButton(artifact, "Удалить", "delete", "danger"));

  document.querySelectorAll(".case-artifact-item").forEach((item) => {
    item.classList.toggle(
        "active",
        Number(item.dataset.caseArtifactRow) === artifactId,
    );
  });
}

function renderCaseArtifacts() {
  const list = byId("case-artifact-list");

  closeArtifactRoleMenu();
  list.replaceChildren();

  if (caseArtifactContexts.length === 0) {
    const empty = document.createElement("p");
    empty.className = "empty-state";
    empty.textContent =
        "Artifacts пока нет. Используйте import или \\o в psql текущего case.";
    list.append(empty);
    closeCaseArtifactViewer();
    return;
  }

  for (const context of caseArtifactContexts) {
    const artifact = context.artifact;
    const item = document.createElement("article");
    item.className = "case-artifact-item";
    item.dataset.caseArtifactRow = String(artifact.id);
    item.tabIndex = 0;
    item.setAttribute("role", "button");

    const main = document.createElement("div");
    main.className = "case-artifact-item-main";

    const fileName = document.createElement("strong");
    fileName.className = "case-artifact-name";
    fileName.textContent = artifact.original_name;

    const meta = document.createElement("span");
    meta.className = "case-artifact-meta";
    const fileTimestamp = artifact.modified_at_ms ?? artifact.created_at_ms;
    meta.textContent =
        `#${artifact.id} · ${artifact.kind} · ${artifact.visibility} · ${formatBytes(artifact.size_bytes)} · ${formatTimestamp(fileTimestamp)}`;

    main.append(fileName, meta);

    if (context.derivation) {
      const lineage = document.createElement("small");
      lineage.className = context.stale
          ? "artifact-lineage stale"
          : "artifact-lineage";
      lineage.textContent = context.stale
          ? `STALE · derived from #${context.derivation.source_artifact_id}`
          : `derived from #${context.derivation.source_artifact_id}`;
      main.append(lineage);
    }

    const comparisonUsage = comparisonUsageForArtifact(artifact.id);
    if (comparisonUsage.length > 0) {
      const usage = document.createElement("small");
      const staleUsage = staleComparisonUsageForArtifact(artifact.id).length;
      usage.className = staleUsage > 0
          ? "artifact-comparison-usage stale"
          : "artifact-comparison-usage";
      usage.textContent = staleUsage > 0
          ? `comparisons: ${comparisonUsage.length} · stale snapshots: ${staleUsage}`
          : `comparisons: ${comparisonUsage.length}`;
      main.append(usage);
    }

    const roleButton = document.createElement("button");
    const currentRole = context.role ?? "";
    roleButton.type = "button";
    roleButton.className = "artifact-role-picker secondary";
    roleButton.dataset.artifactRoleId = String(artifact.id);
    roleButton.dataset.artifactRole = currentRole;
    roleButton.setAttribute("aria-haspopup", "menu");
    roleButton.setAttribute("aria-expanded", "false");
    roleButton.textContent = roleLabel(currentRole);

    item.append(main, roleButton);
    item.classList.toggle("active", artifact.id === selectedCaseArtifactId);
    list.append(item);
  }

  if (selectedCaseArtifactId && artifactContextForId(selectedCaseArtifactId)) {
    void openCaseArtifactViewer(selectedCaseArtifactId);
  } else {
    closeCaseArtifactViewer();
  }
}

function renderPev2CaseOptions() {
  const select = byId("pev2-case");
  const previous = Number(select.value);

  select.replaceChildren();

  if (investigationCases.length === 0) {
    const option = document.createElement("option");

    option.value = "";
    option.textContent = "Cases не найдены";

    select.append(option);
    select.disabled = true;
    return;
  }

  select.disabled = false;

  for (const item of investigationCases) {
    const option = document.createElement("option");

    option.value = String(item.id);
    option.textContent = `#${item.id} · ${item.title}`;

    select.append(option);
  }

  const selected = investigationCases.find(
      (item) => item.id === previous,
  ) ?? activeCase() ?? investigationCases[0];

  select.value = String(selected.id);
}

function renderPev2Artifacts() {
  const select = byId("pev2-artifact");
  const previous = Number(select.value);

  const candidates = pev2Artifacts.filter(
      (artifact) =>
          artifactMode(artifact) === "explain"
          && artifact.visibility === "sensitive",
  );

  select.replaceChildren();

  if (candidates.length === 0) {
    const option = document.createElement("option");

    option.value = "";
    option.textContent = "EXPLAIN JSON artifacts не найдены";

    select.append(option);
    select.disabled = true;
    byId("pev2-open").disabled = true;
    return;
  }

  select.disabled = false;
  byId("pev2-open").disabled = false;

  for (const artifact of candidates) {
    const option = document.createElement("option");

    option.value = String(artifact.id);
    option.textContent =
        `#${artifact.id} · ${artifact.original_name} · ${artifact.kind}`;

    select.append(option);
  }

  const selected = candidates.find(
      (artifact) => artifact.id === previous,
  ) ?? candidates[0];

  select.value = String(selected.id);
}

async function loadPev2Artifacts(caseId) {
  if (!caseId) {
    pev2Artifacts = [];
    renderPev2Artifacts();
    return;
  }

  const context = await invoke(
      "case_context",
      {
        caseId,
      },
  );

  pev2Artifacts = context.artifacts
      .map((item) => item.artifact);

  renderPev2Artifacts();
}

async function refreshCaseArtifacts(scanInbox = true) {
  const item = activeCase();

  if (!item) {
    caseArtifacts = [];
    caseArtifactContexts = [];
    caseComparisons = [];
    caseInvestigationContext = null;
    renderCaseArtifacts();
    renderCaseInvestigation();
    return;
  }

  if (scanInbox) {
    await invoke(
        "list_case_artifacts",
        {
          caseId: item.id,
          scanInbox: true,
        },
    );
  }

  const context = await invoke(
      "case_context",
      {
        caseId: item.id,
      },
  );

  caseInvestigationContext = context;
  caseArtifactContexts = context.artifacts;
  caseComparisons = context.comparisons ?? [];
  caseArtifacts = context.artifacts
      .map((entry) => entry.artifact);

  renderCaseArtifacts();
  renderCaseInvestigation();

  if (selectedPositiveId("pev2-case") === item.id) {
    pev2Artifacts = [...caseArtifacts];
    renderPev2Artifacts();
  }
}

async function refreshCases() {
  const connectionId = selectedPositiveId("case-connection");

  if (!connectionId) {
    investigationCases = [];
    casePrivacyScopes = [];
    activeCaseId = null;
    caseArtifacts = [];
    caseArtifactContexts = [];
    caseComparisons = [];
    caseInvestigationContext = null;
    renderCaseList();
    renderActiveCase();
    renderCaseInvestigation();
    renderPev2CaseOptions();
    pev2Artifacts = [];
    renderPev2Artifacts();
    return;
  }

  await invoke(
      "ensure_connection_workspace",
      {
        connectionId,
      },
  );

  const [scopes, cases] = await Promise.all([
    invoke(
        "list_privacy_scopes",
        {
          connectionId,
        },
    ),
    invoke(
        "list_investigation_cases",
        {
          connectionId,
        },
    ),
  ]);

  casePrivacyScopes = scopes;
  investigationCases = [...cases].sort(
      (left, right) =>
          Number(right.created_at_ms ?? right.id)
          - Number(left.created_at_ms ?? left.id),
  );

  renderScopeOptions(
      byId("case-scope"),
      casePrivacyScopes,
  );

  if (
      activeCaseId === null
      || !investigationCases.some(
          (item) => item.id === activeCaseId,
      )
  ) {
    const remembered = rememberedCaseId(connectionId);

    activeCaseId = investigationCases.some(
        (item) => item.id === remembered,
    )
        ? remembered
        : null;
  }

  renderCaseList();
  renderActiveCase();
  renderPev2CaseOptions();

  if (activeCaseId !== null) {
    await refreshCaseArtifacts(true);
    await loadPev2Artifacts(
        selectedPositiveId("pev2-case"),
    );
  } else {
    caseArtifacts = [];
    caseArtifactContexts = [];
    caseComparisons = [];
    caseInvestigationContext = null;
    renderCaseArtifacts();
    renderCaseInvestigation();
    pev2Artifacts = [];
    renderPev2Artifacts();
  }
}

function renderPrivacyMappingInfo(info) {
  byId("privacy-mapping-path").textContent =
      info
          ? `${info.path} · SENSITIVE / LOCAL`
          : "Mapping не загружен.";

  byId("privacy-count-identifiers").textContent =
      formatInteger(info?.identifiers ?? 0);

  byId("privacy-count-strings").textContent =
      formatInteger(info?.strings ?? 0);

  byId("privacy-count-integers").textContent =
      formatInteger(info?.integers ?? 0);

  byId("privacy-count-floats").textContent =
      formatInteger(info?.floats ?? 0);

  byId("privacy-count-bitstrings").textContent =
      formatInteger(info?.bitstrings ?? 0);

  byId("privacy-count-redactions").textContent =
      formatInteger(info?.redactions ?? 0);

  renderPrivacyScopeActions();
}

async function refreshPrivacyMappingInfo() {
  const scopeId = selectedPositiveId("privacy-scope");

  if (!scopeId) {
    renderPrivacyMappingInfo(null);
    renderPrivacyScopeActions();
    return;
  }

  const info = await invoke(
      "privacy_mapping_info",
      {
        scopeId,
      },
  );

  renderPrivacyMappingInfo(info);
}

function privacyMappingEntryKey(entry) {
  return JSON.stringify([
    entry.category,
    entry.original,
  ]);
}

function renderPrivacyMappingEntries() {
  const body = byId("privacy-mapping-entry-list");
  const selectAll = byId("privacy-mapping-select-all");
  const selected = privacyMappingEntries.filter((entry) =>
    privacyMappingSelection.has(privacyMappingEntryKey(entry)),
  );

  byId("privacy-mapping-result-count").textContent =
      `Показано: ${formatInteger(privacyMappingEntries.length)}`;
  byId("privacy-mapping-selected-count").textContent =
      `Выбрано: ${formatInteger(selected.length)}`;
  byId("privacy-mapping-delete-selected").disabled = selected.length === 0;

  const allSelected = privacyMappingEntries.length > 0
      && selected.length === privacyMappingEntries.length;

  selectAll.checked = allSelected;
  selectAll.indeterminate = selected.length > 0 && !allSelected;
  selectAll.disabled = privacyMappingEntries.length === 0;

  body.replaceChildren();

  if (privacyMappingEntries.length === 0) {
    const row = document.createElement("tr");
    const cell = document.createElement("td");

    cell.colSpan = 6;
    cell.className = "privacy-mapping-empty";
    cell.textContent = "Mapping entries не найдены.";
    row.append(cell);
    body.append(row);
    return;
  }

  for (const entry of privacyMappingEntries) {
    const row = document.createElement("tr");
    const key = privacyMappingEntryKey(entry);
    const selectionCell = document.createElement("td");
    const checkbox = document.createElement("input");

    checkbox.type = "checkbox";
    checkbox.checked = privacyMappingSelection.has(key);
    checkbox.dataset.mappingKey = key;
    selectionCell.append(checkbox);

    const categoryCell = document.createElement("td");
    categoryCell.textContent = entry.category;

    const kindCell = document.createElement("td");
    kindCell.textContent = entry.kind ?? "—";

    const originalCell = document.createElement("td");
    const original = document.createElement("code");
    original.textContent = entry.original;
    originalCell.append(original);

    const aliasCell = document.createElement("td");
    const alias = document.createElement("code");
    alias.textContent = entry.alias;
    aliasCell.append(alias);

    const actionCell = document.createElement("td");
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "danger";
    remove.textContent = "Удалить";
    remove.dataset.mappingDeleteKey = key;
    actionCell.append(remove);

    row.append(
        selectionCell,
        categoryCell,
        kindCell,
        originalCell,
        aliasCell,
        actionCell,
    );
    body.append(row);
  }
}

async function refreshPrivacyMappingEntries() {
  const scopeId = selectedPositiveId("privacy-scope");
  const requestId = ++privacyMappingRequestId;

  privacyMappingSelection.clear();

  if (!scopeId) {
    privacyMappingEntries = [];
    renderPrivacyMappingEntries();
    renderResultPrivacyPolicy();
    return;
  }

  const query = byId("privacy-mapping-search").value.trim();
  const selectedCategory = byId("privacy-mapping-category").value;
  const entries = await invoke(
      "privacy_mapping_entries",
      {
        scopeId,
        query: query || null,
        category: selectedCategory === "all" ? null : selectedCategory,
        limit: 500,
      },
  );

  if (requestId !== privacyMappingRequestId) {
    return;
  }

  privacyMappingEntries = entries;
  renderPrivacyMappingEntries();
}

async function refreshPrivacyMapping() {
  await Promise.all([
    refreshPrivacyMappingInfo(),
    refreshPrivacyMappingEntries(),
  ]);
}

function mappingEntriesByKeys(keys) {
  const wanted = new Set(keys);

  return privacyMappingEntries.filter((entry) =>
    wanted.has(privacyMappingEntryKey(entry)),
  );
}

async function deletePrivacyMappingEntries(entries) {
  const scope = selectedPrivacyScope();

  if (!scope || entries.length === 0) {
    return;
  }

  const examples = entries
      .slice(0, 3)
      .map((entry) => `${entry.original} → ${entry.alias}`)
      .join("\n");
  const more = entries.length > 3
      ? `\n… и ещё ${entries.length - 3}`
      : "";

  if (
      !window.confirm(
          `Удалить mapping entries: ${entries.length}?\n\n${examples}${more}\n\nСуществующие anonymized artifacts могут потерять корректную деанонимизацию. Удалённые alias не переиспользуются автоматически.`,
      )
  ) {
    return;
  }

  const result = await invoke(
      "delete_privacy_mapping_entries",
      {
        scopeId: scope.id,
        entries: entries.map((entry) => ({
          category: entry.category,
          original: entry.original,
        })),
        force: true,
      },
  );

  renderPrivacyMappingInfo(result.mapping);
  byId("privacy-output").value = "";
  await refreshPrivacyMappingEntries();

  const missing = result.missing > 0
      ? ` Не найдено при удалении: ${result.missing}.`
      : "";

  setInlineStatus(
      "privacy-scope-status",
      "success",
      `Удалено mapping entries: ${result.deleted}.${missing}`,
  );
}

async function refreshPrivacyScopes(preferredScopeId = null) {
  const connectionId = selectedPositiveId("privacy-connection");

  if (!connectionId) {
    privacyScopes = [];
    renderScopeOptions(
        byId("privacy-scope"),
        [],
    );
    renderPrivacyMappingInfo(null);
    privacyMappingRequestId += 1;
    privacyMappingEntries = [];
    privacyMappingSelection.clear();
    renderPrivacyMappingEntries();
    return;
  }

  privacyScopes = await invoke(
      "list_privacy_scopes",
      {
        connectionId,
      },
  );

  renderScopeOptions(
      byId("privacy-scope"),
      privacyScopes,
      preferredScopeId,
  );

  await refreshPrivacyMapping();
  renderResultPrivacyPolicy();
}

async function refreshTools() {
  await refreshConnections();

  const caseConnection = selectedPositiveId("case-connection");
  const privacyConnection = selectedPositiveId("privacy-connection");

  if (
      caseConnection
      && privacyConnection !== caseConnection
      && !byId("privacy-connection").dataset.userSelected
  ) {
    byId("privacy-connection").value = String(caseConnection);
  }

  await Promise.all([
    refreshCases(),
    refreshPrivacyScopes(),
  ]);
}

async function runPrivacyTransform(operation) {
  const scopeId = selectedPositiveId("privacy-scope");

  if (!scopeId) {
    throw new Error(
        "Выберите privacy scope.",
    );
  }

  const input = byId("privacy-input").value;
  const mode = operation === "anonymize"
      ? byId("privacy-anonymize-mode").value
      : byId("privacy-deanonymize-mode").value;

  if (mode === "result") {
    const resultPrivacy = collectResultPrivacyPolicy();
    const updated = await invoke("update_privacy_scope_result_policy", {
      scopeId,
      resultPrivacy,
    });
    const index = privacyScopes.findIndex((item) => item.id === updated.id);
    if (index >= 0) privacyScopes[index] = updated;
  }

  const result = await invoke(
      "privacy_transform",
      {
        scopeId,
        mode,
        operation,
        input,
      },
  );

  byId("privacy-output").value =
      result.output;

  renderPrivacyMappingInfo(
      result.mapping,
  );
  await refreshPrivacyMappingEntries();

  const warning = byId("privacy-warning");
  const warnings = result.warnings ?? [];

  warning.hidden = warnings.length === 0;
  warning.textContent = warnings.join("\n");

  return result;
}


function showPev2Status(text, error = false) {
  setInlineStatus(
      "pev2-status",
      error ? "error" : "success",
      text,
  );
}

async function openArtifactInPrivacy(artifactId) {
  const artifact = caseArtifacts.find(
      (item) => item.id === artifactId,
  );

  const item = activeCase();

  if (!artifact || !item) {
    throw new Error(
        "Artifact или case не найден.",
    );
  }

  const content = await invoke(
      "read_case_artifact",
      {
        artifactId,
      },
  );

  const mode = artifactPrivacyMode(artifact);

  byId("privacy-anonymize-mode").value = mode;
  byId("privacy-deanonymize-mode").value = mode;
  renderResultPrivacyPolicy();

  byId("privacy-input").value =
      content;

  byId("privacy-output").value = "";

  setToolsTab("privacy");

  window.requestAnimationFrame(() => {
    byId("privacy-anonymize-mode")
        .closest(".privacy-actions")
        ?.scrollIntoView({block: "start"});
  });
}

async function openPlanWorkspaceForArtifact(
    artifactId,
    caseId = null,
) {
  const resolvedCaseId =
      caseId
      ?? activeCase()?.id
      ?? null;

  if (!resolvedCaseId) {
    throw new Error(
        "Не удалось определить case для artifact.",
    );
  }

  await invoke(
      "open_plan_workspace",
      {
        caseId: resolvedCaseId,
        artifactId,
      },
  );
}

function initToolsUi() {
  restoreCaseListState();
  document.querySelectorAll("[data-tools-tab]").forEach((button) => {
    button.addEventListener("click", () => {
      setToolsTab(
          button.dataset.toolsTab,
      );
    });
  });

  byId("case-connection").addEventListener("change", () => {
    void action(
        async () => {
          activeCaseId = null;
          await refreshCases();
        },
        {
          startMessage: null,
        },
    );
  });

  byId("privacy-connection").addEventListener("change", () => {
    byId("privacy-connection").dataset.userSelected = "1";

    void action(
        async () => {
          await refreshPrivacyScopes();
        },
        {
          startMessage: null,
        },
    );
  });

  byId("privacy-scope").addEventListener("change", () => {
    clearInlineStatus(
        "privacy-scope-status",
    );
    renderPrivacyScopeActions();

    void action(
        refreshPrivacyMapping,
        {
          startMessage: null,
        },
    );
  });

  byId("refresh-cases").addEventListener("click", () => {
    void action(async () => {
      await refreshCases();
      message("Cases обновлены.");
    });
  });

  byId("case-intervention-form").addEventListener("submit", (event) => {
    event.preventDefault();
    const item = activeCase();
    const occurredAtMs = datetimeInputMs("case-intervention-time");
    const title = byId("case-intervention-title").value.trim();
    if (!item || !occurredAtMs || !title) {
      setInlineStatus(
          "case-intervention-status",
          "error",
          "Укажите время и описание intervention.",
      );
      return;
    }

    void action(
        async () => {
          await invoke("create_case_intervention", {
            caseId: item.id,
            occurredAtMs,
            kind: byId("case-intervention-kind").value,
            title,
            note: byId("case-intervention-note").value.trim(),
          });
          byId("case-intervention-title").value = "";
          byId("case-intervention-note").value = "";
          byId("case-intervention-time").value = datetimeLocalValue(Date.now());
          for (const id of [
            "case-impact-before-from",
            "case-impact-before-to",
            "case-impact-after-from",
            "case-impact-after-to",
          ]) {
            byId(id).value = "";
          }
          await refreshCaseArtifacts(false);
          setInlineStatus(
              "case-intervention-status",
              "success",
              "Intervention добавлен в Case timeline.",
          );
        },
        {
          startMessage: null,
          onError: (errorText) => setInlineStatus(
              "case-intervention-status",
              "error",
              errorText,
          ),
        },
    );
  });

  byId("case-impact-intervention").addEventListener("change", (event) => {
    seedCaseImpactPeriods(Number(event.currentTarget.value), true);
    clearInlineStatus("case-impact-status");
  });

  byId("case-impact-save").addEventListener("click", () => {
    const item = activeCase();
    const interventionId = selectedPositiveId("case-impact-intervention");
    const beforeFromMs = datetimeInputMs("case-impact-before-from");
    const beforeToMs = datetimeInputMs("case-impact-before-to");
    const afterFromMs = datetimeInputMs("case-impact-after-from");
    const afterToMs = datetimeInputMs("case-impact-after-to");
    if (
      !item || !interventionId || !beforeFromMs || !beforeToMs
      || !afterFromMs || !afterToMs
    ) {
      setInlineStatus(
          "case-impact-status",
          "error",
          "Выберите intervention и заполните before/after periods.",
      );
      return;
    }

    void action(
        async () => {
          const impact = await invoke("record_case_change_impact", {
            connectionId: item.connection_id,
            caseId: item.id,
            interventionId,
            beforeFromMs,
            beforeToMs,
            afterFromMs,
            afterToMs,
          });
          await refreshCaseArtifacts(false);
          setInlineStatus(
              "case-impact-status",
              "success",
              `Change Impact #${impact.id} сохранён: ${impact.result?.assessment?.label ?? "verification complete"}.`,
          );
        },
        {
          startMessage: null,
          onError: (errorText) => setInlineStatus(
              "case-impact-status",
              "error",
              errorText,
          ),
        },
    );
  });

  byId("case-timeline").addEventListener("click", (event) => {
    const navigationButton = event.target.closest("button[data-case-navigation]");
    if (navigationButton) {
      let target;
      try {
        target = JSON.parse(navigationButton.dataset.caseNavigation);
      } catch (_) {
        return;
      }
      void action(
          () => openCaseNavigation(target),
          {startMessage: null},
      );
      return;
    }

    const evidenceButton = event.target.closest("button[data-case-evidence-artifact]");
    if (evidenceButton) {
      const artifactId = Number(evidenceButton.dataset.caseEvidenceArtifact);
      const row = document.querySelector(`[data-case-artifact-row="${artifactId}"]`);
      if (row) {
        row.scrollIntoView({behavior: "smooth", block: "center"});
        row.classList.add("case-artifact-highlight");
        window.setTimeout(() => row.classList.remove("case-artifact-highlight"), 2200);
      }
      return;
    }

    const deleteButton = event.target.closest("button[data-delete-intervention]");
    if (!deleteButton) return;
    const interventionId = Number(deleteButton.dataset.deleteIntervention);
    if (!Number.isInteger(interventionId) || interventionId <= 0) return;
    if (!window.confirm(`Удалить intervention #${interventionId}?`)) return;
    void action(
        async () => {
          await invoke("delete_case_intervention", {interventionId});
          await refreshCaseArtifacts(false);
        },
        {
          startMessage: null,
          onError: (errorText) => setInlineStatus(
              "case-intervention-status",
              "error",
              errorText,
          ),
        },
    );
  });

  byId("case-impact-list").addEventListener("click", (event) => {
    const button = event.target.closest("button[data-delete-case-impact]");
    if (!button) return;
    const impactId = Number(button.dataset.deleteCaseImpact);
    if (!Number.isInteger(impactId) || impactId <= 0) return;
    if (!window.confirm(`Удалить Change Impact verification #${impactId}?`)) return;
    void action(
        async () => {
          await invoke("delete_case_change_impact", {impactId});
          await refreshCaseArtifacts(false);
        },
        {startMessage: null},
    );
  });

  byId("case-similar-incidents").addEventListener("click", (event) => {
    const button = event.target.closest("button[data-open-similar-case]");
    if (!button) return;
    const caseId = Number(button.dataset.openSimilarCase);
    if (!Number.isInteger(caseId) || caseId <= 0) return;
    if (!investigationCases.some((item) => item.id === caseId)) return;
    activeCaseId = caseId;
    const item = activeCase();
    if (item) {
      rememberActiveCase(item.connection_id, item.id);
    }
    resetCaseFileEditor();
    renderCaseList();
    renderActiveCase();
    void action(
        () => refreshCaseArtifacts(false),
        {startMessage: null},
    );
  });

  byId("toggle-case-sidebar").addEventListener("click", () => {
    const caseListPanel = byId("case-list-panel");

    setCaseListCollapsed(
        !caseListPanel.hidden,
    );
  });

  byId("case-title-input").addEventListener("input", () => {
    clearFieldError(
        "case-title-input",
        "case-title-error",
    );
    clearInlineStatus(
        "case-create-status",
    );
  });

  byId("case-description-input").addEventListener("input", () => {
    clearInlineStatus(
        "case-create-status",
    );
  });

  byId("case-create-form").addEventListener("submit", (event) => {
    event.preventDefault();

    const form = event.currentTarget;

    clearFieldError(
        "case-title-input",
        "case-title-error",
    );
    clearInlineStatus(
        "case-create-status",
    );

    const connectionId = selectedPositiveId("case-connection");
    const scopeId = selectedPositiveId("case-scope");
    const title = byId("case-title-input").value.trim();
    const description =
        byId("case-description-input").value.trim();

    let valid = true;

    if (!title) {
      setFieldError(
          "case-title-input",
          "case-title-error",
          "Укажите название case.",
      );
      valid = false;
    }

    if (!connectionId) {
      setInlineStatus(
          "case-create-status",
          "error",
          "Выберите подключение.",
      );
      valid = false;
    }

    if (!scopeId) {
      setInlineStatus(
          "case-create-status",
          "error",
          "Выберите privacy scope.",
      );
      valid = false;
    }

    if (!valid) {
      byId("case-title-input").focus();
      return;
    }

    void action(
        async () => {
          const created = await invoke(
              "create_investigation_case",
              {
                connectionId,
                title,
                description,
                scopeId,
              },
          );

          activeCaseId = created.id;
          rememberActiveCase(created.connection_id, created.id);
          form.reset();

          await refreshCases();

          setInlineStatus(
              "case-create-status",
              "success",
              `Case #${created.id} создан.`,
          );
        },
        {
          startMessage: null,
          onError: (errorText) => {
            setInlineStatus(
                "case-create-status",
                "error",
                errorText,
            );
          },
        },
    );
  });

  byId("case-list").addEventListener("click", (event) => {
    const button = event.target.closest(
        "button[data-case-id]",
    );

    if (!button) {
      return;
    }

    activeCaseId = Number(
        button.dataset.caseId,
    );

    const selectedCase = activeCase();
    if (selectedCase) {
      rememberActiveCase(
          selectedCase.connection_id,
          selectedCase.id,
      );
    }
    resetCaseFileEditor();

    clearInlineStatus(
        "case-detail-status",
    );

    renderCaseList();
    renderActiveCase();

    void action(
        async () => {
          await refreshCaseArtifacts(true);

          byId("pev2-case").value =
              String(activeCaseId);

          await loadPev2Artifacts(
              activeCaseId,
          );
        },
        {
          startMessage: null,
        },
    );
  });

  byId("case-description-edit").addEventListener("click", () => {
    const item = activeCase();

    if (!item) {
      return;
    }

    byId("case-description").value =
        item.description ?? "";

    byId("case-description-editor").hidden = false;
    byId("case-description-edit").hidden = true;
    byId("case-description").focus();
  });

  byId("case-description-cancel").addEventListener("click", () => {
    const item = activeCase();

    if (!item) {
      return;
    }

    const description = (item.description ?? "").trim();

    byId("case-description").value = description;
    byId("case-description-editor").hidden = true;
    byId("case-description-edit").hidden = false;

    if (!description) {
      byId("case-description-details").open = false;
    }

    clearInlineStatus(
        "case-detail-status",
    );
  });

  byId("case-description").addEventListener("input", () => {
    clearInlineStatus(
        "case-detail-status",
    );
  });

  byId("case-description-save").addEventListener("click", () => {
    const item = activeCase();

    if (!item) {
      return;
    }

    const description =
        byId("case-description").value.trim();

    clearInlineStatus(
        "case-detail-status",
    );

    void action(
        async () => {
          const updated = await invoke(
              "update_investigation_case_description",
              {
                caseId: item.id,
                description,
              },
          );

          const index = investigationCases.findIndex(
              (candidate) => candidate.id === updated.id,
          );

          if (index !== -1) {
            investigationCases[index] = updated;
          }

          renderCaseList();
          renderActiveCase();

          setInlineStatus(
              "case-detail-status",
              "success",
              "Описание case сохранено.",
          );
        },
        {
          startMessage: null,
          onError: (errorText) => {
            setInlineStatus(
                "case-detail-status",
                "error",
                errorText,
            );
          },
        },
    );
  });

  byId("case-open-terminal").addEventListener("click", () => {
    const item = activeCase();

    if (!item) {
      return;
    }

    void openConnectionSession(
        item.connection_id,
        item.id,
    ).then(() => {
      message(
          `psql открыт в inbox case #${item.id}.`,
      );
    }).catch((error) => {
      message(String(error), true);
    });
  });

  byId("case-delete").addEventListener("click", () => {
    const item = activeCase();

    if (!item) {
      return;
    }

    if (
        !window.confirm(
            `Удалить case #${item.id} "${item.title}"?\n\nБудут удалены inbox/source/share/analyses и все зарегистрированные artifacts. Privacy scope и mapping останутся.`,
        )
    ) {
      return;
    }

    clearInlineStatus(
        "case-detail-status",
    );

    void action(
        async () => {
          await invoke(
              "delete_investigation_case",
              {
                caseId: item.id,
              },
          );

          forgetActiveCase(item.connection_id, item.id);
          activeCaseId = null;
          caseArtifacts = [];
          pev2Artifacts = [];
          resetCaseFileEditor();

          await refreshCases();

          setInlineStatus(
              "case-create-status",
              "success",
              `Case #${item.id} удалён.`,
          );
        },
        {
          startMessage: null,
          onError: (errorText) => {
            setInlineStatus(
                "case-detail-status",
                "error",
                errorText,
            );
          },
        },
    );
  });

  byId("case-new-file").addEventListener("click", () => {
    clearInlineStatus("case-detail-status");
    openNewCaseFileEditor();
  });

  byId("case-file-cancel").addEventListener("click", () => {
    resetCaseFileEditor();
    clearInlineStatus("case-detail-status");
  });

  byId("case-file-save").addEventListener("click", () => {
    const item = activeCase();

    if (!item) {
      return;
    }

    const content = byId("case-file-content").value;
    const editingArtifactId = caseFileEditorArtifactId;

    if (!editingArtifactId && !byId("case-file-name").value.trim()) {
      setInlineStatus(
          "case-detail-status",
          "error",
          "Укажите имя файла.",
      );
      byId("case-file-name").focus();
      return;
    }

    clearInlineStatus("case-detail-status");

    void action(
        async () => {
          const artifact = editingArtifactId
              ? await invoke(
                  "update_case_file",
                  { artifactId: editingArtifactId, content },
              )
              : await invoke(
                  "create_case_file",
                  {
                    caseId: item.id,
                    fileName: byId("case-file-name").value.trim(),
                    content,
                    kind: byId("case-file-kind").value,
                  },
              );

          resetCaseFileEditor();
          await refreshCaseArtifacts(false);
          await loadPev2Artifacts(item.id);

          setInlineStatus(
              "case-detail-status",
              "success",
              editingArtifactId
                  ? `Artifact #${artifact.id} сохранён.`
                  : `Artifact #${artifact.id} создан.`,
          );
        },
        {
          startMessage: null,
          onError: (errorText) => {
            setInlineStatus(
                "case-detail-status",
                "error",
                errorText,
            );
          },
        },
    );
  });

  byId("case-import-file").addEventListener("click", () => {
    const item = activeCase();
    const sourcePath = byId("case-import-path").value.trim();

    if (!item || !sourcePath) {
      setInlineStatus(
          "case-detail-status",
          "error",
          "Укажите локальный путь к файлу.",
      );
      return;
    }

    clearInlineStatus(
        "case-detail-status",
    );

    void action(async () => {
      const artifact = await invoke(
          "import_case_file",
          {
            caseId: item.id,
            sourcePath,
            kind: byId("case-import-kind").value,
          },
      );

      byId("case-import-path").value = "";

      await refreshCaseArtifacts(false);
      await loadPev2Artifacts(item.id);

      setInlineStatus(
          "case-detail-status",
          "success",
          `Artifact #${artifact.id} импортирован.`,
      );
    }, {
      startMessage: null,
      onError: (errorText) => {
        setInlineStatus(
            "case-detail-status",
            "error",
            errorText,
        );
      },
    });
  });

  byId("case-scan-inbox").addEventListener("click", () => {
    clearInlineStatus(
        "case-detail-status",
    );

    void action(async () => {
      await refreshCaseArtifacts(true);

      const item = activeCase();

      if (item) {
        await loadPev2Artifacts(item.id);
      }

      setInlineStatus(
          "case-detail-status",
          "success",
          "Case inbox перечитан.",
      );
    }, {
      startMessage: null,
      onError: (errorText) => {
        setInlineStatus(
            "case-detail-status",
            "error",
            errorText,
        );
      },
    });
  });

  byId("case-artifact-browser").addEventListener("click", (event) => {
    const trigger = event.target.closest(
        "button[data-artifact-role-id]",
    );

    if (!trigger) {
      return;
    }

    event.stopPropagation();
    openArtifactRoleMenu(trigger);
  });

  byId("case-artifact-list").addEventListener("click", (event) => {
    if (event.target.closest("button")) return;
    const item = event.target.closest("[data-case-artifact-row]");
    if (!item) return;
    void action(
        () => openCaseArtifactViewer(Number(item.dataset.caseArtifactRow)),
        { startMessage: null },
    );
  });

  byId("case-artifact-list").addEventListener("keydown", (event) => {
    if (event.key !== "Enter" && event.key !== " ") return;
    if (event.target.closest("button")) return;
    const item = event.target.closest("[data-case-artifact-row]");
    if (!item) return;
    event.preventDefault();
    void action(
        () => openCaseArtifactViewer(Number(item.dataset.caseArtifactRow)),
        { startMessage: null },
    );
  });

  byId("case-artifact-viewer-close").addEventListener(
      "click",
      closeCaseArtifactViewer,
  );

  document.addEventListener("pointerdown", (event) => {
    if (!artifactRoleMenuState) {
      return;
    }

    const { menu, trigger } = artifactRoleMenuState;

    if (
        menu.contains(event.target)
        || trigger.contains(event.target)
    ) {
      return;
    }

    closeArtifactRoleMenu();
  }, true);

  document.addEventListener("keydown", (event) => {
    if (
        event.key === "Escape"
        && artifactRoleMenuState
    ) {
      event.preventDefault();
      closeArtifactRoleMenu(true);
    }
  });

  window.addEventListener(
      "resize",
      () => closeArtifactRoleMenu(),
  );

  window.addEventListener(
      "scroll",
      (event) => {
        if (
            artifactRoleMenuState?.menu === event.target
        ) {
          return;
        }

        closeArtifactRoleMenu();
      },
      true,
  );

  document.addEventListener("click", (event) => {
    const option = event.target.closest(
        "button[data-artifact-role-option]",
    );

    if (!option || !artifactRoleMenuState) {
      return;
    }

    const artifactId =
        artifactRoleMenuState.artifactId;

    const role =
        option.dataset.artifactRoleOption || null;

    closeArtifactRoleMenu();

    void action(
        async () => {
          await invoke(
              "set_case_artifact_role",
              { artifactId, role },
          );

          await refreshCaseArtifacts(false);
        },
        { startMessage: null },
    );
  });

  byId("case-artifact-browser").addEventListener("click", (event) => {
    const button = event.target.closest(
        "button[data-case-artifact-action]",
    );

    if (!button) {
      return;
    }

    const artifactId = Number(
        button.dataset.artifactId,
    );

    const artifactCaseId = Number(
        button.dataset.caseId,
    );

    if (button.dataset.caseArtifactAction === "edit") {
      void action(
          () => openCaseFileEditor(artifactId),
          {
            startMessage: null,
            onError: (errorText) => {
              setInlineStatus(
                  "case-detail-status",
                  "error",
                  errorText,
              );
            },
          },
      );

      return;
    }

    if (button.dataset.caseArtifactAction === "copy") {
      void action(
          async () => {
            const content = await invoke(
                "read_case_artifact",
                { artifactId },
            );

            if (!navigator.clipboard?.writeText) {
              throw new Error(
                  "Clipboard API недоступен в текущем WebView.",
              );
            }

            await navigator.clipboard.writeText(
                content,
            );

            setInlineStatus(
                "case-detail-status",
                "success",
                "Содержимое artifact скопировано.",
            );
          },
          {
            startMessage: null,
          },
      );

      return;
    }

    if (button.dataset.caseArtifactAction === "privacy") {
      void action(
          async () => {
            await openArtifactInPrivacy(
                artifactId,
            );

            message("Artifact открыт в privacy editor.");
          },
      );

      return;
    }

    if (button.dataset.caseArtifactAction === "plan") {
      void action(
          async () => {
            await openPlanWorkspaceForArtifact(
                artifactId,
                artifactCaseId,
            );

            message("Plan Workspace открыт в отдельном окне.");
          },
      );

      return;
    }

    if (button.dataset.caseArtifactAction === "export") {
      void action(
          async () => {
            const path = await invoke(
                "export_case_artifact",
                { artifactId },
            );

            setInlineStatus(
                "case-detail-status",
                "success",
                `Artifact экспортирован: ${path}`,
            );
          },
          { startMessage: null },
      );

      return;
    }

    if (button.dataset.caseArtifactAction === "reveal") {
      void action(
          async () => {
            const path = await invoke(
                "reveal_case_artifact",
                { artifactId },
            );

            setInlineStatus(
                "case-detail-status",
                "success",
                `Открыт каталог: ${path}`,
            );
          },
          { startMessage: null },
      );

      return;
    }

    if (button.dataset.caseArtifactAction === "delete") {
      const artifact = caseArtifacts.find(
          (item) => item.id === artifactId,
      );

      if (!artifact) {
        return;
      }

      const comparisons = comparisonUsageForArtifact(artifact.id);
      const comparisonIds = comparisons
          .map((comparison) => `#${comparison.id}`)
          .join(", ");
      const warning = comparisons.length > 0
          ? `\n\nArtifact участвует в ${comparisons.length} comparison(s): ${comparisonIds}. Удаление также удалит эти записи истории. Остальные artifacts сохранятся.`
          : "";

      if (
          !window.confirm(
              `Удалить artifact #${artifact.id} "${artifact.original_name}"?\n\nФайл будет удалён с диска, запись — из case.${warning}`,
          )
      ) {
        return;
      }

      void action(
          async () => {
            await invoke(
                "delete_case_artifact",
                {
                  artifactId,
                  deleteComparisons: comparisons.length > 0,
                },
            );

            if (selectedCaseArtifactId === artifactId) {
              closeCaseArtifactViewer();
            }

            await refreshCaseArtifacts(false);

            const item = activeCase();

            if (item) {
              await loadPev2Artifacts(item.id);
            }

            setInlineStatus(
                "case-detail-status",
                "success",
                comparisons.length > 0
                    ? `Artifact #${artifact.id} и ${comparisons.length} comparison(s) удалены.`
                    : `Artifact #${artifact.id} удалён.`,
            );
          },
          {
            startMessage: null,
            onError: (errorText) => {
              setInlineStatus(
                  "case-detail-status",
                  "error",
                  errorText,
              );
            },
          },
      );

      return;
    }
  });

  byId("privacy-new-scope-name").addEventListener("input", () => {
    clearFieldError(
        "privacy-new-scope-name",
        "privacy-new-scope-error",
    );
    clearInlineStatus(
        "privacy-scope-status",
    );
  });

  byId("privacy-scope-create-form").addEventListener("submit", (event) => {
    event.preventDefault();

    const form = event.currentTarget;
    const name = byId("privacy-new-scope-name").value.trim();
    const connectionId = selectedPositiveId("privacy-connection");

    clearFieldError(
        "privacy-new-scope-name",
        "privacy-new-scope-error",
    );
    clearInlineStatus(
        "privacy-scope-status",
    );

    if (!name) {
      setFieldError(
          "privacy-new-scope-name",
          "privacy-new-scope-error",
          "Укажите название privacy scope.",
      );
      byId("privacy-new-scope-name").focus();
      return;
    }

    if (
        byId("privacy-new-scope-type").value === "connection"
        && !connectionId
    ) {
      setInlineStatus(
          "privacy-scope-status",
          "error",
          "Для connection scope выберите подключение.",
      );
      return;
    }

    void action(
        async () => {
          const created = await invoke(
              "create_privacy_scope",
              {
                name,
                connectionId:
                    byId("privacy-new-scope-type").value === "connection"
                        ? connectionId
                        : null,
              },
          );

          form.reset();

          await refreshPrivacyScopes(
              created.id,
          );

          setInlineStatus(
              "privacy-scope-status",
              "success",
              `Privacy scope "${created.name}" создан.`,
          );
        },
        {
          startMessage: null,
          onError: (errorText) => {
            setInlineStatus(
                "privacy-scope-status",
                "error",
                errorText,
            );
          },
        },
    );
  });

  byId("privacy-reset-mapping").addEventListener("click", () => {
    const scope = selectedPrivacyScope();

    if (!scope) {
      return;
    }

    if (
        !window.confirm(
            `Очистить mapping scope "${scope.name}"?\n\nВсе существующие anonymized artifacts этого scope потеряют возможность корректной деанонимизации. Сам scope и cases останутся.`,
        )
    ) {
      return;
    }

    clearInlineStatus(
        "privacy-scope-status",
    );

    void action(
        async () => {
          const info = await invoke(
              "reset_privacy_mapping",
              {
                scopeId: scope.id,
                force: true,
              },
          );

          renderPrivacyMappingInfo(
              info,
          );
          await refreshPrivacyMappingEntries();

          byId("privacy-output").value = "";

          setInlineStatus(
              "privacy-scope-status",
              "success",
              `Mapping scope "${scope.name}" очищен.`,
          );
        },
        {
          startMessage: null,
          onError: (errorText) => {
            setInlineStatus(
                "privacy-scope-status",
                "error",
                errorText,
            );
          },
        },
    ).then(() => {
      renderPrivacyScopeActions();
    });
  });

  byId("privacy-delete-scope").addEventListener("click", () => {
    const scope = selectedPrivacyScope();

    if (!scope) {
      return;
    }

    if (scope.is_default) {
      setInlineStatus(
          "privacy-scope-status",
          "error",
          "Default privacy scope подключения удалить нельзя. Очистите mapping.",
      );
      return;
    }

    if (
        !window.confirm(
            `Удалить privacy scope "${scope.name}"?\n\nУдаление разрешено только если ни один case его не использует.`,
        )
    ) {
      return;
    }

    clearInlineStatus(
        "privacy-scope-status",
    );

    void action(
        async () => {
          await invoke(
              "delete_privacy_scope",
              {
                scopeId: scope.id,
              },
          );

          await refreshPrivacyScopes();

          // Cases selector may include project scopes, so refresh it too.
          await refreshCases();

          setInlineStatus(
              "privacy-scope-status",
              "success",
              `Privacy scope "${scope.name}" удалён.`,
          );
        },
        {
          startMessage: null,
          onError: (errorText) => {
            setInlineStatus(
                "privacy-scope-status",
                "error",
                errorText,
            );
          },
        },
    ).then(() => {
      renderPrivacyScopeActions();
    });
  });

  byId("privacy-mapping-search").addEventListener("input", () => {
    if (privacyMappingSearchTimer) {
      window.clearTimeout(privacyMappingSearchTimer);
    }

    privacyMappingSearchTimer = window.setTimeout(() => {
      privacyMappingSearchTimer = null;
      void action(
          refreshPrivacyMappingEntries,
          {
            startMessage: null,
            onError: (errorText) => {
              setInlineStatus(
                  "privacy-scope-status",
                  "error",
                  errorText,
              );
            },
          },
      );
    }, 250);
  });

  byId("privacy-mapping-category").addEventListener("change", () => {
    void action(
        refreshPrivacyMappingEntries,
        {
          startMessage: null,
          onError: (errorText) => {
            setInlineStatus(
                "privacy-scope-status",
                "error",
                errorText,
            );
          },
        },
    );
  });

  byId("privacy-mapping-refresh").addEventListener("click", () => {
    void action(
        refreshPrivacyMapping,
        {
          startMessage: null,
          onError: (errorText) => {
            setInlineStatus(
                "privacy-scope-status",
                "error",
                errorText,
            );
          },
        },
    );
  });

  byId("privacy-mapping-select-all").addEventListener("change", (event) => {
    privacyMappingSelection.clear();

    if (event.currentTarget.checked) {
      for (const entry of privacyMappingEntries) {
        privacyMappingSelection.add(privacyMappingEntryKey(entry));
      }
    }

    renderPrivacyMappingEntries();
  });

  byId("privacy-mapping-entry-list").addEventListener("change", (event) => {
    const checkbox = event.target.closest("input[data-mapping-key]");

    if (!checkbox) {
      return;
    }

    if (checkbox.checked) {
      privacyMappingSelection.add(checkbox.dataset.mappingKey);
    } else {
      privacyMappingSelection.delete(checkbox.dataset.mappingKey);
    }

    renderPrivacyMappingEntries();
  });

  byId("privacy-mapping-entry-list").addEventListener("click", (event) => {
    const button = event.target.closest("button[data-mapping-delete-key]");

    if (!button) {
      return;
    }

    const entries = mappingEntriesByKeys([button.dataset.mappingDeleteKey]);

    void action(
        async () => {
          await deletePrivacyMappingEntries(entries);
        },
        {
          startMessage: null,
          onError: (errorText) => {
            setInlineStatus(
                "privacy-scope-status",
                "error",
                errorText,
            );
          },
        },
    );
  });

  byId("privacy-mapping-delete-selected").addEventListener("click", () => {
    const entries = mappingEntriesByKeys(privacyMappingSelection);

    void action(
        async () => {
          await deletePrivacyMappingEntries(entries);
        },
        {
          startMessage: null,
          onError: (errorText) => {
            setInlineStatus(
                "privacy-scope-status",
                "error",
                errorText,
            );
          },
        },
    );
  });

  byId("privacy-result-add-rule").addEventListener("click", () => {
    addResultPrivacyRule();
    clearInlineStatus("privacy-result-policy-status");
  });

  byId("privacy-result-rule-list").addEventListener("click", (event) => {
    const addNested = event.target.closest("[data-result-privacy-add-nested]");
    if (addNested) {
      const parent = addNested.closest(".privacy-result-rule");
      const children = parent.querySelector(":scope > [data-result-privacy-nested] > [data-result-privacy-children]");
      const depth = Number(parent.dataset.resultPrivacyDepth ?? 0) + 1;
      children.append(createResultPrivacyRuleRow("field", "", "value", depth));
      children.lastElementChild.querySelector("[data-result-privacy-selector]").focus();
      clearInlineStatus("privacy-result-policy-status");
      return;
    }

    const button = event.target.closest("[data-result-privacy-remove]");
    if (!button) return;

    button.closest(".privacy-result-rule")?.remove();
    const list = byId("privacy-result-rule-list");
    if (!list.querySelector(":scope > .privacy-result-rule")) {
      const empty = document.createElement("p");
      empty.className = "hint privacy-result-rules-empty";
      empty.textContent = "Overrides не заданы.";
      list.append(empty);
    }
    clearInlineStatus("privacy-result-policy-status");
  });

  byId("privacy-result-rule-list").addEventListener("change", (event) => {
    const row = event.target.closest(".privacy-result-rule");
    if (!row) return;

    if (event.target.matches("[data-result-privacy-mode]")) {
      setResultPrivacyRuleMode(row, event.target.value);
    }

    if (event.target.matches("[data-result-privacy-selector-kind]")) {
      const input = row.querySelector(":scope > .privacy-result-rule-header [data-result-privacy-selector]");
      const kind = event.target.value;
      input.hidden = kind === "values";
      input.maxLength = kind === "regex" ? 512 : 128;
      input.placeholder = kind === "regex" ? "^(source|target)_table$" : "row_identity";
      if (kind === "values") input.value = "";
    }

    clearInlineStatus("privacy-result-policy-status");
  });

  byId("privacy-result-policy").addEventListener("input", () => {
    clearInlineStatus("privacy-result-policy-status");
  });
  byId("privacy-result-policy").addEventListener("change", () => {
    clearInlineStatus("privacy-result-policy-status");
  });

  byId("privacy-result-policy-save").addEventListener("click", () => {
    const scope = selectedPrivacyScope();
    if (!scope) return;

    void action(
        async () => {
          const resultPrivacy = collectResultPrivacyPolicy();
          const updated = await invoke("update_privacy_scope_result_policy", {
            scopeId: scope.id,
            resultPrivacy,
          });
          const index = privacyScopes.findIndex((item) => item.id === updated.id);
          if (index >= 0) privacyScopes[index] = updated;

          renderResultPrivacyPolicy();
          byId("privacy-output").value = "";
          setInlineStatus(
              "privacy-result-policy-status",
              "success",
              "Result privacy policy сохранена в scope.",
          );
        },
        {
          startMessage: null,
          onError: (errorText) => {
            setInlineStatus("privacy-result-policy-status", "error", errorText);
          },
        },
    );
  });

  byId("privacy-input").addEventListener("input", () => {
    clearInlineStatus(
        "privacy-transform-status",
    );
  });

  byId("privacy-anonymize-mode").addEventListener("change", () => {
    clearInlineStatus(
        "privacy-transform-status",
    );
  });

  byId("privacy-deanonymize-mode").addEventListener("change", () => {
    clearInlineStatus(
        "privacy-transform-status",
    );
  });

  byId("privacy-anonymize").addEventListener("click", () => {
    clearInlineStatus(
        "privacy-transform-status",
    );

    void action(
        async () => {
          await runPrivacyTransform(
              "anonymize",
          );

          setInlineStatus(
              "privacy-transform-status",
              "success",
              "Данные анонимизированы.",
          );
        },
        {
          startMessage: null,
          onError: (errorText) => {
            setInlineStatus(
                "privacy-transform-status",
                "error",
                errorText,
            );
          },
        },
    );
  });

  byId("privacy-deanonymize").addEventListener("click", () => {
    clearInlineStatus(
        "privacy-transform-status",
    );

    void action(
        async () => {
          await runPrivacyTransform(
              "deanonymize",
          );

          setInlineStatus(
              "privacy-transform-status",
              "success",
              "Данные деанонимизированы.",
          );
        },
        {
          startMessage: null,
          onError: (errorText) => {
            setInlineStatus(
                "privacy-transform-status",
                "error",
                errorText,
            );
          },
        },
    );
  });

  byId("privacy-copy-output").addEventListener("click", () => {
    void action(
        async () => {
          const text = byId("privacy-output").value;

          if (!navigator.clipboard?.writeText) {
            throw new Error(
                "Clipboard API недоступен в текущем WebView.",
            );
          }

          await navigator.clipboard.writeText(
              text,
          );

          message("Результат скопирован.");
        },
        {
          startMessage: null,
        },
    );
  });

  byId("pev2-case").addEventListener("change", () => {
    const caseId = selectedPositiveId("pev2-case");

    void action(
        async () => {
          await loadPev2Artifacts(
              caseId,
          );
        },
        {
          startMessage: null,
        },
    );
  });

  byId("pev2-open").addEventListener("click", () => {
    const caseId = selectedPositiveId("pev2-case");
    const artifactId = selectedPositiveId("pev2-artifact");

    if (!caseId || !artifactId) {
      showPev2Status(
          "Выберите case и исходный EXPLAIN JSON artifact.",
          true,
      );
      return;
    }

    showPev2Status(
        "Открываем отдельный Plan Workspace…",
    );

    void action(
        async () => {
          await invoke(
              "open_plan_workspace",
              {
                caseId,
                artifactId,
              },
          );

          showPev2Status(
              "Plan Workspace открыт отдельным окном. Локальный просмотр использует исходные реальные данные.",
          );
        },
        {
          startMessage: null,
          onError: (errorText) => {
            showPev2Status(
                errorText,
                true,
            );
          },
        },
    );
  });

}