"use strict";

const invoke = window.pgxrayInvoke ?? window.__TAURI__?.core?.invoke;
const byId = (id) => document.getElementById(id);

const params = new URLSearchParams(window.location.search);
const caseId = Number(params.get("case_id"));
const artifactId = Number(params.get("artifact_id"));

let workspace = null;
let references = [];
let referenceByKey = new Map();
let shareJson = null;
let pev2App = null;
let currentRawView = "local";
let findingFilter = "all";
let planMappingEntries = [];
let planMappingSearchTimer = null;

function assertRuntime() {
  if (!invoke) {
    throw new Error(
        "Plan Workspace нужно открывать из pgXray/Tauri, а не как отдельный HTML-файл.",
    );
  }

  if (!Number.isInteger(caseId) || caseId <= 0) {
    throw new Error("Некорректный case_id в Plan Workspace URL.");
  }

  if (!Number.isInteger(artifactId) || artifactId <= 0) {
    throw new Error("Некорректный artifact_id в Plan Workspace URL.");
  }
}

function setStatus(text, error = false) {
  const element = byId("plan-status");

  element.textContent = text ?? "";
  element.classList.toggle("visible", Boolean(text));
  element.classList.toggle("error", Boolean(error));
}

function setButtonBusy(button, busy, label = null) {
  if (!button) {
    return;
  }

  if (busy) {
    button.dataset.previousText = button.textContent;
    button.disabled = true;

    if (label) {
      button.textContent = label;
    }

    return;
  }

  if (button.dataset.previousText !== undefined) {
    button.textContent = button.dataset.previousText;
    delete button.dataset.previousText;
  }

  button.disabled = false;
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

function formatMs(value) {
  if (value === null || value === undefined) {
    return "—";
  }

  const number = Number(value);

  if (number >= 1000) {
    return `${formatNumber(number / 1000, 2)} s`;
  }

  return `${formatNumber(number, 2)} ms`;
}

function formatBytes(value) {
  if (value === null || value === undefined) {
    return "—";
  }

  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let size = Number(value);
  let index = 0;

  while (size >= 1024 && index < units.length - 1) {
    size /= 1024;
    index += 1;
  }

  return `${formatNumber(size, index === 0 ? 0 : 1)} ${units[index]}`;
}

function element(tag, options = {}) {
  const node = document.createElement(tag);

  if (options.className) {
    node.className = options.className;
  }

  if (options.text !== undefined) {
    node.textContent = String(options.text);
  }

  if (options.title) {
    node.title = options.title;
  }

  return node;
}

function replaceWithText(target, text) {
  target.replaceChildren(
      document.createTextNode(text ?? ""),
  );
}

function setPlanTab(name) {
  document.querySelectorAll("[data-plan-tab]").forEach((button) => {
    button.classList.toggle(
        "active",
        button.dataset.planTab === name,
    );
  });

  for (const candidate of [
    "visual",
    "analysis",
    "compare",
    "raw",
    "reference",
  ]) {
    byId(`plan-panel-${candidate}`).hidden = candidate !== name;
  }

  if (name === "visual") {
    requestAnimationFrame(() => {
      window.dispatchEvent(new Event("resize"));
    });
  }
}

function mountPev2(planJson) {
  if (!window.Vue) {
    throw new Error("Vue vendor asset не загружен.");
  }

  if (!window.pev2?.Plan) {
    throw new Error("PEV2 vendor asset не загружен.");
  }

  const source = String(planJson ?? "").trim();

  if (!source) {
    throw new Error("Пустой EXPLAIN JSON.");
  }

  const parsed = JSON.parse(source);
  const root = Array.isArray(parsed)
      ? parsed[0]
      : parsed;

  if (!root?.Plan?.["Node Type"]) {
    throw new Error("EXPLAIN JSON не содержит top-level Plan/Node Type.");
  }

  if (pev2App) {
    pev2App.unmount();
    pev2App = null;
  }

  const rootElement = byId("plan-pev2-root");

  rootElement.replaceChildren();

  pev2App = window.Vue.createApp({
    data() {
      return {
        plan: source,
        query: "",
      };
    },
    template: `
      <div style="width:100%;height:100%;min-width:0;min-height:0;">
        <pev2
          :plan-source="plan"
          :plan-query="query"
        />
      </div>
    `,
  });

  pev2App.component("pev2", window.pev2.Plan);
  pev2App.mount(rootElement);

  requestAnimationFrame(() => {
    window.dispatchEvent(new Event("resize"));
  });
}

function summaryCard(label, value) {
  const card = element("article", {
    className: "summary-card",
  });

  card.append(
      element("span", { text: label }),
      element("strong", { text: value }),
  );

  return card;
}

function renderAnalysisSummary() {
  const summary = workspace?.inspection?.analysis?.summary;
  const document = workspace?.inspection?.document;
  const root = byId("analysis-summary");

  root.replaceChildren();

  if (!summary || !document) {
    root.append(
        element("div", {
          className: "plan-empty",
          text: "Plan Analyzer не вернул summary.",
        }),
    );
    return;
  }

  root.append(
      summaryCard("Execution", formatMs(summary.execution_time_ms)),
      summaryCard("Nodes", formatNumber(summary.node_count, 0)),
      summaryCard("High", formatNumber(summary.high_count, 0)),
      summaryCard("Warnings", formatNumber(summary.warning_count, 0)),
      summaryCard("Shared read", formatNumber(summary.root_shared_read_blocks, 0)),
      summaryCard("Temp written", formatNumber(summary.root_temp_written_blocks, 0)),
      summaryCard("WAL", formatBytes(summary.root_wal_bytes)),
      summaryCard(
          "Analyze",
          document.source_has_analyze ? "есть actual" : "только estimate",
      ),
  );
}

function referenceKeyForNodeType(nodeType) {
  if (!nodeType) {
    return null;
  }

  if (nodeType === "Seq Scan" || nodeType === "Parallel Seq Scan") {
    return "seq_scan";
  }

  if (nodeType === "Index Scan") {
    return "index_scan";
  }

  if (nodeType === "Index Only Scan") {
    return "index_only_scan";
  }

  if (nodeType === "Bitmap Index Scan") {
    return "bitmap_index_scan";
  }

  if (nodeType === "Bitmap Heap Scan") {
    return "bitmap_heap_scan";
  }

  if (nodeType.startsWith("Nested Loop")) {
    return "nested_loop";
  }

  if (nodeType.includes("Hash Join")) {
    return "hash_join";
  }

  if (nodeType.includes("Merge Join")) {
    return "merge_join";
  }

  const direct = new Map([
    ["Hash", "hash"],
    ["Sort", "sort"],
    ["Incremental Sort", "incremental_sort"],
    ["Aggregate", "aggregate"],
    ["GroupAggregate", "aggregate"],
    ["HashAggregate", "aggregate"],
    ["Materialize", "materialize"],
    ["Memoize", "memoize"],
    ["Gather", "gather"],
    ["Gather Merge", "gather_merge"],
    ["Limit", "limit"],
    ["Append", "append"],
    ["Merge Append", "append"],
    ["CTE Scan", "cte_scan"],
    ["Subquery Scan", "subquery_scan"],
    ["Function Scan", "function_scan"],
    ["WindowAgg", "window_agg"],
    ["Unique", "unique"],
    ["SetOp", "setop"],
  ]);

  return direct.get(nodeType) ?? null;
}

function helpButton(referenceKey, label = "?") {
  const button = element("button", {
    className: "reference-link-button",
    text: label,
    title: "Открыть краткий справочник по этому узлу/понятию",
  });

  button.type = "button";
  button.disabled = !referenceKey;

  if (referenceKey) {
    button.dataset.referenceKey = referenceKey;
  }

  return button;
}

function renderHotspots() {
  const body = byId("analysis-hotspots");
  const hotspots = workspace?.inspection?.analysis?.hotspots ?? [];

  body.replaceChildren();

  if (hotspots.length === 0) {
    const row = document.createElement("tr");
    const cell = document.createElement("td");

    cell.colSpan = 6;
    cell.textContent = "Hotspots не выделены: для estimate-only плана runtime metrics могут отсутствовать.";
    row.append(cell);
    body.append(row);
    return;
  }

  for (const hotspot of hotspots) {
    const row = document.createElement("tr");
    const labelCell = document.createElement("td");
    const nodeType = String(hotspot.label)
        .replace(/^#\d+\s+/, "")
        .split(" · ")[0];

    const labelButton = helpButton(
        referenceKeyForNodeType(nodeType),
        hotspot.label,
    );

    labelButton.className = "node-help-button";
    labelCell.append(labelButton);

    for (const value of [
      formatMs(hotspot.repeated_time_ms),
      formatNumber(hotspot.actual_rows_total, 0),
      formatNumber(hotspot.actual_loops, 0),
      formatNumber(hotspot.shared_read_blocks, 0),
      formatNumber(hotspot.temp_written_blocks, 0),
    ]) {
      row.append(
          element("td", { text: value }),
      );
    }

    row.prepend(labelCell);
    body.append(row);
  }
}

function renderFindings() {
  const container = byId("analysis-findings");
  const findings = workspace?.inspection?.analysis?.findings ?? [];
  const visible = findings.filter(
      (finding) => findingFilter === "all" || finding.severity === findingFilter,
  );

  container.replaceChildren();

  if (visible.length === 0) {
    container.append(
        element("div", {
          className: "plan-empty",
          text: findings.length === 0
              ? "Analyzer не обнаружил эвристических сигналов. Это не означает, что план оптимален."
              : "Для выбранного severity findings нет.",
        }),
    );
    return;
  }

  for (const finding of visible) {
    const card = element("article", {
      className: `finding-card ${finding.severity}`,
    });
    const titleRow = element("div", {
      className: "finding-title-row",
    });
    const title = element("div");
    const severity = element("span", {
      className: "finding-severity",
      text: finding.severity,
    });

    title.append(
        severity,
        element("strong", { text: finding.title }),
    );

    titleRow.append(
        title,
        helpButton(
            finding.reference_key
                ?? referenceKeyForNodeType(finding.node_type),
            "Что это значит?",
        ),
    );

    card.append(
        titleRow,
        element("p", { text: finding.summary }),
    );

    if (finding.evidence?.length) {
      const evidence = element("ul", {
        className: "finding-evidence",
      });

      for (const item of finding.evidence) {
        evidence.append(
            element("li", { text: item }),
        );
      }

      card.append(evidence);
    }

    if (finding.next_checks?.length) {
      card.append(
          element("strong", { text: "Что проверить дальше" }),
      );

      const next = element("ul", {
        className: "finding-next",
      });

      for (const item of finding.next_checks) {
        next.append(
            element("li", { text: item }),
        );
      }

      card.append(next);
    }

    container.append(card);
  }
}

function renderAnalysis() {
  renderAnalysisSummary();
  renderHotspots();
  renderFindings();
}


function mappingKind(entry) {
  return entry.kind ?? entry.category ?? "value";
}

function mappingTotal(info) {
  return [
    info?.identifiers,
    info?.strings,
    info?.integers,
    info?.floats,
    info?.bitstrings,
    info?.redactions,
  ].reduce((total, value) => total + Number(value ?? 0), 0);
}

function renderMapping() {
  const list = byId("mapping-list");

  list.replaceChildren();

  if (planMappingEntries.length === 0) {
    list.append(
        element("div", {
          className: "plan-empty",
          text: byId("mapping-filter").value.trim()
              ? "Ничего не найдено."
              : "Mapping этого privacy scope пока пуст.",
        }),
    );
    return;
  }

  for (const entry of planMappingEntries) {
    const item = element("article", {
      className: "mapping-entry",
    });

    item.append(
        element("span", {
          className: "mapping-entry-kind",
          text: mappingKind(entry),
        }),
        element("span", {
          className: "mapping-entry-real",
          text: entry.original,
        }),
        element("span", {
          className: "mapping-entry-alias",
          text: entry.alias,
        }),
    );

    list.append(item);
  }
}

function renderMappingError(error) {
  planMappingEntries = [];
  const list = byId("mapping-list");

  list.replaceChildren(
      element("div", {
        className: "plan-empty",
        text: `Не удалось загрузить mapping: ${String(error)}`,
      }),
  );
}

async function refreshMappingCount() {
  const scopeId = workspace?.case?.scope_id;

  if (!scopeId) {
    byId("plan-mapping-count").textContent = "—";
    return;
  }

  try {
    const info = await invoke("privacy_mapping_info", { scopeId });
    byId("plan-mapping-count").textContent = String(mappingTotal(info));
  } catch (_) {
    byId("plan-mapping-count").textContent = "—";
  }
}

async function refreshMapping() {
  const scopeId = workspace?.case?.scope_id;

  if (!scopeId) {
    planMappingEntries = [];
    renderMapping();
    return;
  }

  const query = byId("mapping-filter").value.trim();
  const list = byId("mapping-list");

  list.replaceChildren(
      element("div", {
        className: "plan-empty",
        text: "Загрузка mapping…",
      }),
  );

  planMappingEntries = await invoke(
      "privacy_mapping_entries",
      {
        scopeId,
        query: query || null,
        category: null,
        limit: 500,
      },
  );

  renderMapping();
  await refreshMappingCount();
}

function artifactContextEntries() {
  return workspace?.context?.artifacts ?? [];
}

function isExplainArtifact(context) {
  const artifact = context.artifact;
  const name = artifact.original_name.toLowerCase();

  return artifact.visibility === "sensitive"
      && (
        artifact.kind === "explain"
        || name.endsWith(".json")
      );
}

function isSqlArtifact(context) {
  const artifact = context.artifact;
  const name = artifact.original_name.toLowerCase();

  return artifact.visibility === "sensitive"
      && (
        artifact.kind === "sql"
        || name.endsWith(".sql")
      );
}

function comparisonArtifactId(purpose, side) {
  const comparisons = workspace?.context?.comparisons ?? [];

  for (const comparison of comparisons) {
    const member = comparison.members.find(
        (item) => item.purpose === purpose && item.side === side,
    );

    if (member) {
      return member.artifact_id;
    }
  }

  return null;
}

function fillArtifactSelect(
    select,
    contexts,
    preferredArtifactId = null,
    fallbackIndex = 0,
) {
  const previous = Number(select.value);

  select.replaceChildren();

  if (contexts.length === 0) {
    const option = document.createElement("option");

    option.value = "";
    option.textContent = "Нет подходящих artifacts";
    select.append(option);
    select.disabled = true;
    return;
  }

  select.disabled = false;

  for (const context of contexts) {
    const option = document.createElement("option");

    option.value = String(context.artifact.id);
    option.textContent = `#${context.artifact.id} · ${context.artifact.original_name}${context.role ? ` · ${context.role}` : ""}`;
    select.append(option);
  }

  const preferred = contexts.find((context) => context.artifact.id === previous)
      ?? contexts.find((context) => context.artifact.id === preferredArtifactId)
      ?? contexts[Math.min(fallbackIndex, contexts.length - 1)];

  select.value = String(preferred.artifact.id);
}

function renderCompareSelectors() {
  const contexts = artifactContextEntries();
  const plans = contexts.filter(isExplainArtifact);
  const queries = contexts.filter(isSqlArtifact);
  const currentPlan = plans.some((context) => context.artifact.id === artifactId)
      ? artifactId
      : null;

  fillArtifactSelect(
      byId("compare-plan-baseline"),
      plans,
      comparisonArtifactId("plan", "baseline") ?? currentPlan,
  );
  fillArtifactSelect(
      byId("compare-plan-candidate"),
      plans,
      comparisonArtifactId("plan", "candidate"),
      1,
  );
  fillArtifactSelect(
      byId("compare-query-baseline"),
      queries,
      comparisonArtifactId("query", "baseline"),
  );
  fillArtifactSelect(
      byId("compare-query-candidate"),
      queries,
      comparisonArtifactId("query", "candidate"),
      1,
  );
}

function comparisonKindLabel(kind) {
  return {
    saved_plan: "Saved plans",
    query_result: "Query results",
    query_explain: "EXPLAIN",
    query_analyze: "EXPLAIN ANALYZE",
    legacy_query: "Legacy SQL pair",
    legacy_plan: "Legacy plan pair",
  }[kind] ?? kind;
}

function comparisonMember(comparison, purpose, side) {
  return comparison.members.find(
      (member) => member.purpose === purpose && member.side === side,
  ) ?? null;
}

function comparisonArtifactContext(artifactId) {
  return artifactContextEntries().find(
      (item) => item.artifact.id === artifactId,
  ) ?? null;
}

function comparisonMemberLabel(member) {
  if (!member) {
    return "—";
  }

  const context = comparisonArtifactContext(member.artifact_id);
  const artifactLabel = context
      ? `#${member.artifact_id} · ${context.artifact.original_name}`
      : `artifact #${member.artifact_id}`;

  return `${member.purpose}: ${artifactLabel}`;
}

function comparisonSideMembers(comparison, side) {
  return comparison.members.filter((member) => member.side === side);
}

function comparisonHasStaleMembers(comparison) {
  return comparison.members.some((member) => member.stale);
}

function setComparisonSelect(selectId, artifactId) {
  if (!artifactId) {
    return;
  }

  const select = byId(selectId);
  const exists = Array.from(select.options).some(
      (option) => Number(option.value) === artifactId,
  );

  if (exists) {
    select.value = String(artifactId);
  }
}

function applyComparisonSelection(comparison) {
  const queryBaseline = comparisonMember(comparison, "query", "baseline");
  const queryCandidate = comparisonMember(comparison, "query", "candidate");
  const planBaseline = comparisonMember(comparison, "plan", "baseline");
  const planCandidate = comparisonMember(comparison, "plan", "candidate");

  setComparisonSelect("compare-query-baseline", queryBaseline?.artifact_id);
  setComparisonSelect("compare-query-candidate", queryCandidate?.artifact_id);
  setComparisonSelect("compare-plan-baseline", planBaseline?.artifact_id);
  setComparisonSelect("compare-plan-candidate", planCandidate?.artifact_id);

  setStatus(`Comparison #${comparison.id} восстановлен в selectors.`);
}

function comparisonPlanPair(comparison) {
  const baseline = comparisonMember(comparison, "plan", "baseline");
  const candidate = comparisonMember(comparison, "plan", "candidate");

  return baseline && candidate ? { baseline, candidate } : null;
}

function comparisonCanShowResult(comparison) {
  if (comparison.result) {
    return true;
  }

  const pair = comparisonPlanPair(comparison);
  return Boolean(pair) && !pair.baseline.stale && !pair.candidate.stale;
}

async function comparisonResultForDisplay(comparison) {
  if (comparison.result) {
    return comparison.result;
  }

  const pair = comparisonPlanPair(comparison);

  if (!pair) {
    throw new Error(
        `У comparison #${comparison.id} нет сохранённого результата. Старые Query results автоматически не выполняются повторно.`,
    );
  }

  if (pair.baseline.stale || pair.candidate.stale) {
    throw new Error(
        `Plan artifacts comparison #${comparison.id} изменились или недоступны, поэтому текущий пересчёт уже не будет историческим результатом.`,
    );
  }

  return invoke(
      "preview_plan_artifacts",
      {
        baselineArtifactId: pair.baseline.artifact_id,
        candidateArtifactId: pair.candidate.artifact_id,
      },
  );
}

function renderStoredComparisonResult(comparison, result, target) {
  target.replaceChildren();

  if (comparison.kind === "query_result") {
    renderResultComparison(result, target);
    return;
  }

  if (
    ["saved_plan", "query_explain", "query_analyze", "legacy_plan"].includes(
        comparison.kind,
    )
  ) {
    renderPlanComparison(result, target);
    return;
  }

  target.append(
      element("pre", {
        className: "raw-plan",
        text: JSON.stringify(result, null, 2),
      }),
  );
}

async function toggleComparisonResult(comparison, button) {
  const target = document.querySelector(
      `[data-comparison-result="${comparison.id}"]`,
  );

  if (!target) {
    throw new Error(`Result panel comparison #${comparison.id} не найден.`);
  }

  if (!target.hidden) {
    target.hidden = true;
    button.textContent = "Показать результат";
    return;
  }

  setButtonBusy(button, true, "Загрузка…");

  try {
    const result = await comparisonResultForDisplay(comparison);
    renderStoredComparisonResult(comparison, result, target);
    target.hidden = false;
  } finally {
    setButtonBusy(button, false);
  }

  if (!target.hidden) {
    button.textContent = "Скрыть результат";
  }
}

async function openComparisonPlan(comparison, side) {
  const member = comparisonMember(comparison, "plan", side);

  if (!member) {
    throw new Error(`В comparison #${comparison.id} нет ${side} plan.`);
  }

  await invoke(
      "open_plan_workspace",
      {
        caseId,
        artifactId: member.artifact_id,
      },
  );
}

async function deleteComparisonHistory(comparison) {
  if (
    !window.confirm(
        `Удалить comparison #${comparison.id} из истории?\n\nArtifacts и их файлы останутся. Будет удалена только связь baseline/candidate.`,
    )
  ) {
    return;
  }

  await invoke(
      "delete_case_comparison",
      { comparisonId: comparison.id },
  );

  await loadWorkspace({ quiet: true });
  setPlanTab("compare");
  setStatus(`Comparison #${comparison.id} удалён из истории.`);
}

function comparisonMemberNode(member) {
  const row = element("div", {
    className: `comparison-history-member${member.stale ? " stale" : ""}`,
  });
  const label = element("span", {
    text: comparisonMemberLabel(member),
  });
  const state = element("strong", {
    text: member.stale ? "STALE" : "current",
  });

  row.append(label, state);
  return row;
}

function comparisonSideNode(comparison, side) {
  const block = element("div", {
    className: "comparison-history-side",
  });
  const title = element("strong", {
    text: side === "baseline" ? "Baseline" : "Candidate",
  });
  const members = comparisonSideMembers(comparison, side);

  block.append(title);

  if (members.length === 0) {
    block.append(element("span", { text: "—" }));
    return block;
  }

  for (const member of members) {
    block.append(comparisonMemberNode(member));
  }

  return block;
}

function renderComparisonHistory() {
  const target = byId("compare-history");
  const comparisons = workspace?.context?.comparisons ?? [];

  target.replaceChildren();

  if (comparisons.length === 0) {
    target.append(
        element("p", {
          text: "Сохранённых сравнений пока нет.",
        }),
    );
    return;
  }

  const list = element("div", {
    className: "comparison-history-list",
  });

  for (const comparison of comparisons) {
    const card = element("article", {
      className: `comparison-history-card${comparisonHasStaleMembers(comparison) ? " stale" : ""}`,
    });
    const header = element("div", {
      className: "comparison-history-header",
    });
    const identity = element("div", {
      className: "comparison-history-identity",
    });
    const actions = element("div", {
      className: "comparison-history-actions",
    });
    const date = new Date(Number(comparison.created_at_ms));

    identity.append(
        element("strong", {
          text: `#${comparison.id} · ${comparisonKindLabel(comparison.kind)}`,
        }),
        element("small", {
          text: date.toLocaleString("ru-RU"),
        }),
    );

    if (comparisonHasStaleMembers(comparison)) {
      identity.append(
          element("span", {
            className: "comparison-history-warning",
            text: "STALE · один или несколько artifacts изменились или недоступны",
          }),
      );
    }

    const showResult = element("button", {
      className: "plan-button secondary",
      text: "Показать результат",
    });
    showResult.type = "button";
    showResult.dataset.comparisonAction = "show-result";
    showResult.dataset.comparisonId = String(comparison.id);
    showResult.disabled = !comparisonCanShowResult(comparison);
    if (!comparison.result && comparisonPlanPair(comparison)) {
      const pair = comparisonPlanPair(comparison);
      showResult.title = pair.baseline.stale || pair.candidate.stale
          ? "Plan artifacts изменились: исторический результат нельзя восстановить пересчётом."
          : "Старый comparison: результат будет локально пересчитан из сохранённых plan artifacts без новой записи в историю.";
    } else if (!comparison.result) {
      showResult.title = "Результат этого старого comparison не был сохранён.";
    }
    actions.append(showResult);

    const reuse = element("button", {
      className: "plan-button secondary",
      text: "Использовать снова",
    });
    reuse.type = "button";
    reuse.dataset.comparisonAction = "reuse";
    reuse.dataset.comparisonId = String(comparison.id);
    actions.append(reuse);

    for (const side of ["baseline", "candidate"]) {
      if (!comparisonMember(comparison, "plan", side)) {
        continue;
      }

      const open = element("button", {
        className: "plan-button secondary",
        text: side === "baseline" ? "Открыть baseline plan" : "Открыть candidate plan",
      });
      open.type = "button";
      open.dataset.comparisonAction = "open-plan";
      open.dataset.comparisonId = String(comparison.id);
      open.dataset.comparisonSide = side;
      actions.append(open);
    }

    const remove = element("button", {
      className: "plan-button danger",
      text: "Удалить из истории",
    });
    remove.type = "button";
    remove.dataset.comparisonAction = "delete";
    remove.dataset.comparisonId = String(comparison.id);
    actions.append(remove);

    header.append(identity, actions);

    const sides = element("div", {
      className: "comparison-history-sides",
    });
    sides.append(
        comparisonSideNode(comparison, "baseline"),
        comparisonSideNode(comparison, "candidate"),
    );

    const storedResult = element("div", {
      className: "comparison-history-result",
    });
    storedResult.dataset.comparisonResult = String(comparison.id);
    storedResult.hidden = true;

    card.append(header, sides, storedResult);
    list.append(card);
  }

  target.append(list);
}

function renderRaw() {
  byId("raw-plan").textContent = currentRawView === "local"
      ? workspace?.local_plan_json ?? ""
      : workspace?.anonymous_plan_json ?? "";

  document.querySelectorAll("[data-raw-view]").forEach((button) => {
    button.classList.toggle(
        "active",
        button.dataset.rawView === currentRawView,
    );
  });
}

function renderReferenceList(filter = "") {
  const list = byId("reference-list");
  const normalized = filter.trim().toLowerCase();

  list.replaceChildren();

  const visible = references.filter((entry) => {
    if (!normalized) {
      return true;
    }

    return [
      entry.title,
      entry.category,
      entry.short,
      entry.how_it_works,
      ...(entry.related ?? []),
    ]
        .join(" ")
        .toLowerCase()
        .includes(normalized);
  });

  for (const entry of visible) {
    const button = element("button", {
      className: "reference-list-item",
    });

    button.type = "button";
    button.dataset.referenceKey = entry.key;
    button.append(
        element("strong", { text: entry.title }),
        element("small", { text: entry.category }),
    );

    list.append(button);
  }

  if (visible.length === 0) {
    list.append(
        element("div", {
          className: "plan-empty",
          text: "В справочнике ничего не найдено.",
        }),
    );
  }
}

function appendReferenceSection(container, title, values) {
  if (!values?.length) {
    return;
  }

  const section = element("section", {
    className: "reference-section",
  });
  const list = document.createElement("ul");

  section.append(
      element("h2", { text: title }),
  );

  for (const value of values) {
    list.append(
        element("li", { text: value }),
    );
  }

  section.append(list);
  container.append(section);
}

const referenceHistoryByTarget = new WeakMap();

function referenceHistory(target) {
  let history = referenceHistoryByTarget.get(target);

  if (!history) {
    history = [];
    referenceHistoryByTarget.set(target, history);
  }

  return history;
}

function resetReferenceHistory(target) {
  referenceHistoryByTarget.set(target, []);
}

function scrollReferenceTargetToTop(target) {
  if (target.id === "reference-document") {
    target.scrollTo({
      top: 0,
      behavior: "auto",
    });
    return;
  }

  target
      .closest(".reference-drawer")
      ?.scrollTo({
        top: 0,
        behavior: "auto",
      });
}

function appendReferenceBack(target) {
  const history = referenceHistory(target);

  if (!history.length) {
    return;
  }

  const previousKey = history[history.length - 1];
  const previousEntry = referenceByKey.get(previousKey);

  if (!previousEntry) {
    return;
  }

  const button = element("button", {
    className: "reference-back",
    text: `← Вернуться: ${previousEntry.title}`,
    title: `Вернуться к статье «${previousEntry.title}»`,
  });

  button.type = "button";
  button.addEventListener("click", () => {
    history.pop();

    if (target.id === "reference-document") {
      selectReference(
          previousKey,
          {
            resetHistory: false,
          },
      );
      return;
    }

    renderReferenceDocument(
        previousEntry,
        target,
    );
    scrollReferenceTargetToTop(target);
  });

  target.append(button);
}

function appendReferenceTerms(target, keys) {
  if (!keys?.length) {
    return;
  }

  const section = element("section", {
    className: "reference-section",
  });
  const terms = element("div", {
    className: "reference-term-list",
  });

  section.append(
      element("h2", { text: "Термины в этой статье" }),
  );

  for (const key of keys) {
    const entry = referenceByKey.get(key);

    if (!entry) {
      continue;
    }

    const button = element("button", {
      className: "reference-term-link",
      text: entry.title,
      title: `Открыть определение: ${entry.title}`,
    });

    button.type = "button";
    button.addEventListener("click", () => {
      const currentKey = target.dataset.currentReferenceKey;

      if (currentKey && currentKey !== key) {
        referenceHistory(target).push(currentKey);
      }

      if (target.id === "reference-document") {
        selectReference(
            key,
            {
              resetHistory: false,
            },
        );
        return;
      }

      renderReferenceDocument(entry, target);
      scrollReferenceTargetToTop(target);
    });

    terms.append(button);
  }

  if (!terms.childElementCount) {
    return;
  }

  section.append(terms);
  target.append(section);
}

function renderReferenceDocument(entry, target = byId("reference-document")) {
  target.replaceChildren();

  if (!entry) {
    delete target.dataset.currentReferenceKey;

    target.append(
        element("div", {
          className: "plan-empty",
          text: "Выберите термин слева.",
        }),
    );
    return;
  }

  target.dataset.currentReferenceKey = entry.key;
  appendReferenceBack(target);

  target.append(
      element("h1", { text: entry.title }),
      element("p", { text: entry.short }),
      element("p", { text: entry.how_it_works }),
  );

  appendReferenceTerms(target, entry.terms);
  appendReferenceSection(target, "Когда это нормально", entry.good_when);
  appendReferenceSection(target, "Когда подозрительно", entry.suspicious_when);
  appendReferenceSection(target, "Что смотреть рядом", entry.inspect);
  appendReferenceSection(target, "Типичные ошибки интерпретации", entry.misconceptions);
  appendReferenceSection(target, "Связанные темы", entry.related);

  if (entry.docs_url) {
    const section = element("section", {
      className: "reference-section",
    });
    const url = element("code", {
      className: "reference-docs-url",
      text: entry.docs_url,
      title: "Нажмите, чтобы скопировать URL официальной документации",
    });

    url.tabIndex = 0;
    url.dataset.copyUrl = entry.docs_url;

    section.append(
        element("h2", { text: "PostgreSQL docs" }),
        url,
    );
    target.append(section);
  }
}

function selectReference(
    key,
    {
      resetHistory = true,
    } = {},
) {
  const entry = referenceByKey.get(key);

  if (!entry) {
    return;
  }

  const documentPanel = byId("reference-document");

  if (resetHistory) {
    resetReferenceHistory(documentPanel);
  }

  document.querySelectorAll(".reference-list-item").forEach((button) => {
    button.classList.toggle(
        "active",
        button.dataset.referenceKey === key,
    );
  });

  renderReferenceDocument(
      entry,
      documentPanel,
  );
  scrollReferenceTargetToTop(documentPanel);
}

function openReferenceDrawer(key) {
  const entry = referenceByKey.get(key);

  if (!entry) {
    setStatus(`Справочник для ${key} не найден.`, true);
    return;
  }

  const drawerBody = byId("reference-drawer-body");

  resetReferenceHistory(drawerBody);
  renderReferenceDocument(
      entry,
      drawerBody,
  );

  byId("mapping-drawer").hidden = true;
  byId("reference-drawer").hidden = false;
  scrollReferenceTargetToTop(drawerBody);
}

function metricDeltaClass(metric) {
  if (metric.delta === null || metric.delta === undefined) {
    return "delta-neutral";
  }

  if (Math.abs(metric.delta) < Number.EPSILON) {
    return "delta-neutral";
  }

  const improvement = metric.lower_is_better
      ? metric.delta < 0
      : metric.delta > 0;

  return improvement ? "delta-good" : "delta-bad";
}

function renderPlanComparison(comparison, target) {
  target.replaceChildren();

  if (!comparison) {
    return;
  }

  const summary = element("div", {
    className: "comparison-summary",
  });

  for (const line of comparison.summary ?? []) {
    summary.append(
        element("p", { text: line }),
    );
  }

  target.append(summary);

  const tableWrap = element("div", {
    className: "plan-table-wrap",
  });
  const table = element("table", {
    className: "plan-table",
  });
  const thead = document.createElement("thead");
  const header = document.createElement("tr");

  for (const title of [
    "Metric",
    "Baseline",
    "Candidate",
    "Δ",
  ]) {
    header.append(
        element("th", { text: title }),
    );
  }

  thead.append(header);
  table.append(thead);

  const body = document.createElement("tbody");

  for (const metric of comparison.metrics ?? []) {
    const row = document.createElement("tr");
    const delta = metric.delta_pct !== null && metric.delta_pct !== undefined
        ? `${metric.delta_pct >= 0 ? "+" : ""}${formatNumber(metric.delta_pct, 1)}%`
        : metric.delta !== null && metric.delta !== undefined
            ? formatNumber(metric.delta, 2)
            : "—";
    const deltaCell = element("td", {
      className: metricDeltaClass(metric),
      text: delta,
    });

    row.append(
        element("td", { text: metric.label }),
        element("td", { text: formatNumber(metric.baseline, 2) }),
        element("td", { text: formatNumber(metric.candidate, 2) }),
        deltaCell,
    );

    body.append(row);
  }

  table.append(body);
  tableWrap.append(table);
  target.append(tableWrap);

  if (comparison.relation_strategies?.length) {
    const relationCard = element("div", {
      className: "plan-card",
    });
    const relationList = document.createElement("ul");

    relationCard.append(
        element("h2", { text: "Изменения access strategy" }),
    );

    for (const change of comparison.relation_strategies) {
      relationList.append(
          element("li", {
            text: `${change.relation}: ${change.baseline_strategies.join(", ") || "—"} → ${change.candidate_strategies.join(", ") || "—"}`,
          }),
      );
    }

    relationCard.append(relationList);
    target.append(relationCard);
  }

  if (comparison.node_types?.length) {
    const nodeCard = element("div", {
      className: "plan-card",
    });
    const nodeList = document.createElement("ul");

    nodeCard.append(
        element("h2", { text: "Node Type delta" }),
    );

    for (const change of comparison.node_types) {
      nodeList.append(
          element("li", {
            text: `${change.node_type}: ${change.baseline_count} → ${change.candidate_count} (${change.delta >= 0 ? "+" : ""}${change.delta})`,
          }),
      );
    }

    nodeCard.append(nodeList);
    target.append(nodeCard);
  }
}

function renderResultComparison(result, target) {
  target.replaceChildren();

  if (!result) {
    return;
  }

  target.append(
      element("div", {
        className: result.equivalent ? "result-equivalent" : "result-different",
        text: result.equivalent
            ? "RESULTS EQUIVALENT ✓"
            : "RESULTS DIFFER — performance comparison нельзя считать semantic-equivalent rewrite.",
      }),
  );

  const table = element("table", {
    className: "plan-table",
  });
  const body = document.createElement("tbody");

  for (const [label, value] of [
    ["Baseline rows", result.baseline_rows],
    ["Candidate rows", result.candidate_rows],
    ["Baseline \\ Candidate", result.baseline_only_rows],
    ["Candidate \\ Baseline", result.candidate_only_rows],
  ]) {
    const row = document.createElement("tr");

    row.append(
        element("td", { text: label }),
        element("td", { text: formatNumber(value, 0) }),
    );
    body.append(row);
  }

  table.append(body);
  target.append(table);

  if (!result.equivalent) {
    for (const [label, sample] of [
      ["Baseline only sample", result.baseline_only_sample],
      ["Candidate only sample", result.candidate_only_sample],
    ]) {
      target.append(
          element("h3", { text: label }),
      );
      const pre = element("pre", {
        className: "raw-plan",
        text: JSON.stringify(sample ?? [], null, 2),
      });

      pre.style.minHeight = "auto";
      pre.style.maxHeight = "320px";
      target.append(pre);
    }
  }

  if (result.warnings?.length) {
    const warning = element("div", {
      className: "compare-note",
      text: result.warnings.join("\n"),
    });

    target.append(warning);
  }
}

async function loadReferences() {
  references = await invoke("plan_reference");
  referenceByKey = new Map(
      references.map((entry) => [entry.key, entry]),
  );

  renderReferenceList();

  if (references.length) {
    selectReference(references[0].key);
  }
}

async function loadShareJson() {
  const share = workspace?.share;

  if (!share?.artifact?.id) {
    shareJson = null;
    return;
  }

  try {
    shareJson = await invoke(
        "read_case_artifact",
        {
          artifactId: share.artifact.id,
        },
    );
  } catch (_) {
    shareJson = null;
  }
}

function renderShareState() {
  const share = workspace?.share;
  const copy = byId("plan-copy-share");
  const exportButton = byId("plan-export-share");
  const revealButton = byId("plan-reveal-share");

  copy.disabled = !share;
  exportButton.disabled = !share;
  revealButton.disabled = !share;

  if (!share) {
    byId("plan-prepare-share").textContent = "Подготовить share";
    return;
  }

  byId("plan-prepare-share").textContent = share.stale
      ? "Обновить stale share"
      : "Обновить share";

  const state = share.stale ? "STALE" : "current";

  setStatus(
      `Локально открыт исходный план. Share artifact #${share.artifact.id} (${state}). Mapping управляется в Tools → Анонимизация.`,
      false,
  );
}

function renderWorkspace() {
  const artifact = workspace.artifact;
  const currentShare = workspace.share;

  byId("plan-title").textContent = artifact.original_name;
  byId("plan-meta").textContent = [
    `case #${workspace.case.id} · ${workspace.case.title}`,
    `artifact #${artifact.id}`,
    `scope #${workspace.case.scope_id}`,
    `nodes ${workspace.inspection.document.node_count}`,
    currentShare
        ? `share #${currentShare.artifact.id}${currentShare.stale ? " STALE" : ""}`
        : "share not persisted",
  ].join(" · ");

  mountPev2(workspace.local_plan_json);
  renderAnalysis();
  renderRaw();
  renderCompareSelectors();
  renderComparisonHistory();
  renderShareState();
  void refreshMappingCount();
}

async function loadWorkspace({ quiet = false } = {}) {
  if (!quiet) {
    setStatus("Загрузка Plan Workspace…");
  }

  workspace = await invoke(
      "load_plan_workspace",
      {
        caseId,
        artifactId,
      },
  );

  await loadShareJson();
  renderWorkspace();

  if (!quiet && !workspace.share) {
    setStatus(
        `Локально открыт исходный plan #${artifactId}. Anonymous preview не изменяет mapping.json; mapping управляется в Tools → Анонимизация.`,
    );
  }
}

async function prepareShare() {
  const button = byId("plan-prepare-share");

  setButtonBusy(button, true, "Подготовка…");

  try {
    const result = await invoke(
        "prepare_plan_share",
        {
          caseId,
          artifactId,
        },
    );

    setStatus(
        `Share artifact #${result.share.artifact.id} подготовлен. Mapping persisted локально.`,
    );

    await loadWorkspace({ quiet: true });
  } catch (error) {
    setStatus(String(error), true);
  } finally {
    setButtonBusy(button, false);
  }
}

async function exportShare() {
  const share = workspace?.share;

  if (!share) {
    return;
  }

  try {
    const path = await invoke(
        "export_case_artifact",
        {
          artifactId: share.artifact.id,
        },
    );

    setStatus(`Share скопирован: ${path}`);
  } catch (error) {
    setStatus(String(error), true);
  }
}

async function revealShare() {
  const share = workspace?.share;

  if (!share) {
    return;
  }

  try {
    const path = await invoke(
        "reveal_case_artifact",
        {
          artifactId: share.artifact.id,
        },
    );

    setStatus(`Открыт каталог: ${path}`);
  } catch (error) {
    setStatus(String(error), true);
  }
}

async function copyText(text, success) {
  const clipboard =
      window.__TAURI__?.clipboardManager;

  if (!clipboard?.writeText) {
    throw new Error(
        "Tauri Clipboard API недоступен.",
    );
  }

  await clipboard.writeText(text);

  setStatus(success);
}

async function copyAiAnalysis() {
  const button = byId("analysis-copy-ai");

  setButtonBusy(button, true, "Копирование…");

  try {
    const prompt = await invoke("get_explain_analysis_prompt");
    const planJson = String(workspace?.anonymous_plan_json ?? "").trim();

    if (!planJson) {
      throw new Error("Анонимизированный EXPLAIN JSON недоступен.");
    }

    const text = [
      String(prompt ?? "").trim(),
      "---",
      "## EXPLAIN JSON (anonymized by pgXray)",
      `\`\`\`json\n${planJson}\n\`\`\``,
    ].join("\n\n");

    await copyText(
        `${text}\n`,
        "AI prompt + anonymized EXPLAIN JSON скопированы.",
    );
  } finally {
    setButtonBusy(button, false);
  }
}

async function compareSavedPlans() {
  const baseline = Number(byId("compare-plan-baseline").value);
  const candidate = Number(byId("compare-plan-candidate").value);

  if (!baseline || !candidate) {
    setStatus("Выберите baseline и candidate plan.", true);
    return;
  }

  if (baseline === candidate) {
    setStatus("Baseline и candidate должны быть разными artifacts.", true);
    return;
  }

  const button = byId("compare-plans-run");

  setButtonBusy(button, true, "Сравнение…");

  try {
    const result = await invoke(
        "compare_plan_artifacts",
        {
          baselineArtifactId: baseline,
          candidateArtifactId: candidate,
        },
    );

    renderPlanComparison(
        result.report,
        byId("compare-plan-result"),
    );
    setStatus(`Сохранённые EXPLAIN планы сравнены. Comparison #${result.comparison.id}.`);

    await loadWorkspace({ quiet: true });
    setPlanTab("compare");
  } catch (error) {
    setStatus(String(error), true);
  } finally {
    setButtonBusy(button, false);
  }
}

function selectedSqlComparison() {
  const baselineArtifactId = Number(byId("compare-query-baseline").value);
  const candidateArtifactId = Number(byId("compare-query-candidate").value);
  const timeoutMs = Number(byId("compare-timeout").value);

  if (!baselineArtifactId || !candidateArtifactId) {
    throw new Error("Выберите baseline и candidate SQL artifacts.");
  }

  if (baselineArtifactId === candidateArtifactId) {
    throw new Error("Baseline и candidate SQL должны быть разными artifacts.");
  }

  return {
    baselineArtifactId,
    candidateArtifactId,
    timeoutMs,
  };
}

async function compareQueryPlans(analyzeQueries) {
  const ids = selectedSqlComparison();
  const button = analyzeQueries
      ? byId("compare-query-analyze")
      : byId("compare-query-explain");

  if (analyzeQueries && !byId("compare-execution-confirm").checked) {
    throw new Error("Подтвердите выполнение запросов перед EXPLAIN ANALYZE.");
  }

  setButtonBusy(
      button,
      true,
      analyzeQueries ? "ANALYZE…" : "EXPLAIN…",
  );

  try {
    const result = await invoke(
        "compare_case_query_plans",
        {
          caseId,
          baselineArtifactId: ids.baselineArtifactId,
          candidateArtifactId: ids.candidateArtifactId,
          analyzeQueries,
          timeoutMs: ids.timeoutMs,
        },
    );

    renderPlanComparison(
        result.report.comparison,
        byId("compare-query-result"),
    );

    const warning = element("div", {
      className: "compare-note",
      text: result.report.warnings.join("\n"),
    });

    byId("compare-query-result").append(warning);

    setStatus(
        analyzeQueries
            ? `EXPLAIN ANALYZE выполнен. Comparison #${result.comparison.id}; generated plans: #${result.baseline_artifact.id}, #${result.candidate_artifact.id}.`
            : `EXPLAIN выполнен без запуска SELECT. Comparison #${result.comparison.id}; generated plans: #${result.baseline_artifact.id}, #${result.candidate_artifact.id}.`,
    );

    await loadWorkspace({ quiet: true });
    setPlanTab("compare");
  } finally {
    setButtonBusy(button, false);
  }
}

async function compareQueryResults() {
  const ids = selectedSqlComparison();

  if (!byId("compare-execution-confirm").checked) {
    throw new Error("Подтвердите выполнение запросов перед EXCEPT ALL comparison.");
  }

  const button = byId("compare-query-results");

  setButtonBusy(button, true, "Сравнение данных…");

  try {
    const result = await invoke(
        "compare_case_query_results",
        {
          caseId,
          baselineArtifactId: ids.baselineArtifactId,
          candidateArtifactId: ids.candidateArtifactId,
          timeoutMs: ids.timeoutMs,
        },
    );

    renderResultComparison(
        result.report,
        byId("compare-query-result"),
    );

    setStatus(
        result.report.equivalent
            ? `EXCEPT ALL: результаты эквивалентны. Comparison #${result.comparison.id}.`
            : `EXCEPT ALL обнаружил различия. Comparison #${result.comparison.id}.`,
        !result.report.equivalent,
    );

    await loadWorkspace({ quiet: true });
    setPlanTab("compare");
  } finally {
    setButtonBusy(button, false);
  }
}

function bindEvents() {
  document.querySelectorAll("[data-plan-tab]").forEach((button) => {
    button.addEventListener("click", () => {
      setPlanTab(button.dataset.planTab);
    });
  });

  byId("plan-refresh").addEventListener("click", async () => {
    try {
      await loadWorkspace();
    } catch (error) {
      setStatus(String(error), true);
    }
  });

  byId("plan-mapping-toggle").addEventListener("click", async () => {
    byId("reference-drawer").hidden = true;
    byId("mapping-drawer").hidden = false;

    try {
      await refreshMapping();
    } catch (error) {
      renderMappingError(error);
    }
  });

  byId("mapping-close").addEventListener("click", () => {
    byId("mapping-drawer").hidden = true;
  });

  byId("mapping-filter").addEventListener("input", () => {
    if (planMappingSearchTimer !== null) {
      window.clearTimeout(planMappingSearchTimer);
    }

    planMappingSearchTimer = window.setTimeout(async () => {
      planMappingSearchTimer = null;

      if (byId("mapping-drawer").hidden) {
        return;
      }

      try {
        await refreshMapping();
      } catch (error) {
        renderMappingError(error);
      }
    }, 250);
  });

  byId("reference-drawer-close").addEventListener("click", () => {
    byId("reference-drawer").hidden = true;
  });

  byId("analysis-copy-ai").addEventListener("click", async () => {
    try {
      await copyAiAnalysis();
    } catch (error) {
      setStatus(String(error), true);
    }
  });

  byId("plan-prepare-share").addEventListener("click", prepareShare);
  byId("plan-export-share").addEventListener("click", exportShare);
  byId("plan-reveal-share").addEventListener("click", revealShare);

  byId("plan-copy-share").addEventListener("click", async () => {
    try {
      const text = shareJson ?? workspace?.anonymous_plan_json ?? "";
      await copyText(text, "Anonymized JSON скопирован.");
    } catch (error) {
      setStatus(String(error), true);
    }
  });

  document.querySelectorAll("[data-raw-view]").forEach((button) => {
    button.addEventListener("click", () => {
      currentRawView = button.dataset.rawView;
      renderRaw();
    });
  });

  byId("raw-copy").addEventListener("click", async () => {
    try {
      const text = currentRawView === "local"
          ? workspace?.local_plan_json ?? ""
          : workspace?.anonymous_plan_json ?? "";

      await copyText(text, "Raw JSON скопирован.");
    } catch (error) {
      setStatus(String(error), true);
    }
  });

  document.addEventListener("click", async (event) => {
    const referenceButton = event.target.closest(
        "button[data-reference-key]",
    );

    if (referenceButton) {
      const key = referenceButton.dataset.referenceKey;

      if (referenceButton.classList.contains("reference-list-item")) {
        selectReference(key);
      } else {
        openReferenceDrawer(key);
      }
      return;
    }

    const url = event.target.closest("[data-copy-url]");

    if (url) {
      try {
        await copyText(url.dataset.copyUrl, "URL PostgreSQL documentation скопирован.");
      } catch (error) {
        setStatus(String(error), true);
      }
    }
  });

  byId("reference-search").addEventListener("input", (event) => {
    renderReferenceList(event.currentTarget.value);
  });

  document.querySelectorAll("[data-finding-filter]").forEach((button) => {
    button.addEventListener("click", () => {
      findingFilter = button.dataset.findingFilter;

      document.querySelectorAll("[data-finding-filter]").forEach((candidate) => {
        candidate.classList.toggle(
            "active",
            candidate === button,
        );
      });

      renderFindings();
    });
  });

  byId("compare-history").addEventListener("click", async (event) => {
    const button = event.target.closest("button[data-comparison-action]");

    if (!button) {
      return;
    }

    const comparisonId = Number(button.dataset.comparisonId);
    const comparison = (workspace?.context?.comparisons ?? []).find(
        (item) => item.id === comparisonId,
    );

    if (!comparison) {
      setStatus(`Comparison #${comparisonId} не найден.`, true);
      return;
    }

    try {
      switch (button.dataset.comparisonAction) {
        case "show-result":
          await toggleComparisonResult(comparison, button);
          break;
        case "reuse":
          applyComparisonSelection(comparison);
          break;
        case "open-plan":
          await openComparisonPlan(
              comparison,
              button.dataset.comparisonSide,
          );
          break;
        case "delete":
          await deleteComparisonHistory(comparison);
          break;
        default:
          break;
      }
    } catch (error) {
      setStatus(String(error), true);
    }
  });

  byId("compare-plans-run").addEventListener("click", compareSavedPlans);

  byId("compare-execution-confirm").addEventListener("change", (event) => {
    byId("compare-query-results").disabled = !event.currentTarget.checked;
    byId("compare-query-analyze").disabled = !event.currentTarget.checked;
  });

  byId("compare-query-explain").addEventListener("click", async () => {
    try {
      await compareQueryPlans(false);
    } catch (error) {
      setStatus(String(error), true);
    }
  });

  byId("compare-query-analyze").addEventListener("click", async () => {
    try {
      await compareQueryPlans(true);
    } catch (error) {
      setStatus(String(error), true);
    }
  });

  byId("compare-query-results").addEventListener("click", async () => {
    try {
      await compareQueryResults();
    } catch (error) {
      setStatus(String(error), true);
    }
  });
}

async function initialize() {
  assertRuntime();
  bindEvents();

  await Promise.all([
    loadReferences(),
    loadWorkspace(),
  ]);
}

window.addEventListener("DOMContentLoaded", () => {
  void initialize().catch((error) => {
    setStatus(String(error), true);
    replaceWithText(
        byId("plan-pev2-root"),
        `Не удалось открыть plan: ${error}`,
    );
  });
});
