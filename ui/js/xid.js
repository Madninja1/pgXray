"use strict";

let xidCurrentSnapshot = null;
let xidCurrentConnectionId = null;
let xidCurrentSnapshotId = null;
let xidSelectedRelationOid = null;
let xidRelationFilterTimer = null;

const XID_HISTORY_PAGE_SIZES = [20, 50, 100, 500];
let xidHistoryPage = 1;
let xidHistoryPageSize = 20;
let xidRelationComparisonRows = [];
let xidRelationPage = 1;
let xidRelationPageSize = 20;

function setXidTab(name) {
  document.querySelectorAll(".xid-tab").forEach((button) => {
    const active = button.dataset.xidTab === name;

    button.classList.toggle("active", active);
    button.setAttribute("aria-selected", String(active));
  });

  for (const candidate of [
    "snapshot",
    "relation",
    "methodology",
    "queries",
  ]) {
    byId(`xid-${candidate}-tab`).hidden = candidate !== name;
  }
}

function renderXidConnectionOptions(connections) {
  const select = byId("xid-connection");
  const previous = select.value;

  select.replaceChildren();

  if (connections.length === 0) {
    const option = document.createElement("option");

    option.value = "";
    option.textContent = "Нет сохранённых подключений";

    select.append(option);
    select.disabled = true;
    byId("collect-xid").disabled = true;

    return;
  }

  select.disabled = false;
  byId("collect-xid").disabled = false;

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
  }
}

function xidRelationType(value) {
  if (value === "r") {
    return "table";
  }

  if (value === "m") {
    return "matview";
  }

  if (value === "t") {
    return "TOAST";
  }

  return value || "—";
}

function renderXidDatabases(section) {
  renderSectionError(
      "xid-databases-error",
      section,
  );

  const body = byId("xid-database-list");
  const rows = section?.data ?? [];

  body.replaceChildren();

  if (rows.length === 0) {
    tableEmpty(body, 6, section?.error ? "Секция возрастов баз недоступна." : "Базы не найдены.");
    return;
  }

  for (const database of rows) {
    const row = document.createElement("tr");

    appendCells(
        row,
        [
          String(database.database_oid),
          database.database,
          formatInteger(database.xid_age),
          formatPercent(database.pct_of_global_freeze_max),
          formatInteger(database.mxid_age),
          formatPercent(database.pct_of_global_multixact_freeze_max),
        ],
    );

    body.append(row);
  }
}

function xidRelationDisplayName(relation) {
  const name = `${relation.schema_name}.${relation.relation_name}`;

  if (relation.relation_type === "t" && relation.parent_relation) {
    return `${name} → ${relation.parent_schema}.${relation.parent_relation}`;
  }

  return name;
}

function renderXidRelations(section) {
  renderSectionError(
      "xid-relations-error",
      section,
  );

  const body = byId("xid-relation-list");
  const rows = section?.data ?? [];
  const visibleRows = rows.slice(0, 20);

  body.replaceChildren();
  byId("xid-relations-summary").textContent = rows.length === 0
      ? (section?.error ? "Секция отношений недоступна; отсутствие данных не означает отсутствие риска." : "Relations не найдены.")
      : `Snapshot сохраняет все ${formatInteger(rows.length)} relations; здесь показаны 20 самых старых по age(relfrozenxid).`;

  if (visibleRows.length === 0) {
    tableEmpty(body, 9, section?.error ? "Секция отношений недоступна." : "Отношения не найдены.");
    return;
  }

  for (const relation of visibleRows) {
    const row = document.createElement("tr");
    const effectiveFreeze =
        `${formatInteger(relation.effective_freeze_max_age)} (${relation.freeze_max_source})`;
    const vacuumState = [
      `manual: ${formatInteger(relation.vacuum_count)}`,
      `auto: ${formatInteger(relation.autovacuum_count)}`,
      `last auto: ${formatTimestamp(relation.last_autovacuum)}`,
    ].join(" · ");

    appendCells(
        row,
        [
          xidRelationDisplayName(relation),
          String(relation.relation_oid),
          xidRelationType(relation.relation_type),
          formatInteger(relation.xid_age),
          effectiveFreeze,
          formatPercent(relation.pct_of_freeze_max),
          formatBytes(relation.total_size_bytes),
          vacuumState,
        ],
    );

    const actionCell = document.createElement("td");
    const button = document.createElement("button");

    button.className = "secondary compact-button";
    button.type = "button";
    button.textContent = "Разобрать";
    button.addEventListener("click", () => {
      void openXidRelation(relation.relation_oid);
    });

    actionCell.append(button);
    row.append(actionCell);
    body.append(row);
  }
}

function renderXidTriggers(section) {
  renderSectionError(
      "xid-triggers-error",
      section,
  );

  const body = byId("xid-trigger-list");
  const allRows = section?.data ?? [];
  const rows = allRows.slice(0, 100);

  byId("xid-triggers-summary").textContent = allRows.length > 100
      ? `Расчёт сохранён для всех ${formatInteger(allRows.length)} relations; в технической таблице показаны первые 100 по XID age.`
      : `Расчёт сохранён для ${formatInteger(allRows.length)} relations. Это отдельный механизм от anti-wraparound VACUUM.`;

  body.replaceChildren();

  if (rows.length === 0) {
    tableEmpty(
        body,
        8,
        "Нет данных по normal autovacuum triggers.",
    );
    return;
  }

  for (const trigger of rows) {
    const row = document.createElement("tr");

    const dead =
        `${formatInteger(trigger.estimated_dead_rows)} / ${formatInteger(trigger.estimated_dead_tuple_trigger)}`;

    const insertTrigger = trigger.thresholds_reliable === false
        ? "неизвестен (нет reltuples)"
        : (trigger.effective_insert_threshold < 0
            ? "отключён"
            : formatInteger(trigger.estimated_insert_trigger));

    const inserted =
        `${formatInteger(trigger.inserts_since_vacuum)} / ${insertTrigger}`;

    const dml = [
      `I ${formatInteger(trigger.tuples_inserted)}`,
      `U ${formatInteger(trigger.tuples_updated)}`,
      `D ${formatInteger(trigger.tuples_deleted)}`,
      `HOT ${formatInteger(trigger.hot_updates)}`,
    ].join(" · ");

    appendCells(
        row,
        [
          `${trigger.schema_name}.${trigger.relation_name}`,
          trigger.autovacuum_enabled ? "on" : "off",
          formatInteger(trigger.estimated_rows),
          dead,
          inserted,
          trigger.thresholds_reliable === false ? "неизвестно" : yesNo(trigger.dead_trigger_reached),
          trigger.thresholds_reliable === false ? "неизвестно" : yesNo(trigger.insert_trigger_reached),
          dml,
        ],
    );

    body.append(row);
  }
}

function renderXidVacuums(section) {
  renderSectionError(
      "xid-vacuums-error",
      section,
  );

  const body = byId("xid-vacuum-list");
  const rows = section?.data ?? [];

  body.replaceChildren();

  if (rows.length === 0) {
    tableEmpty(
        body,
        7,
        "Обычный VACUUM сейчас не найден.",
    );
    return;
  }

  for (const vacuum of rows) {
    const row = document.createElement("tr");

    const wait = [
      vacuum.wait_event_type,
      vacuum.wait_event,
    ]
        .filter(Boolean)
        .join(" / ") || "—";

    const backend = vacuum.is_wraparound_vacuum
        ? `${vacuum.backend_type ?? "autovacuum worker"} · anti-wraparound (по тексту запроса)`
        : (vacuum.wraparound_detection === "unknown"
            ? `${vacuum.backend_type ?? "—"} · цель VACUUM не определена`
            : (vacuum.backend_type ?? "—"));

    appendCells(
        row,
        [
          String(vacuum.pid),
          `${vacuum.schema_name}.${vacuum.relation_name}`,
          backend,
          formatInteger(vacuum.xid_age),
          vacuum.phase,
          formatPercent(vacuum.scan_pct),
          wait,
        ],
    );

    body.append(row);
  }
}

function renderXidBackends(section) {
  renderSectionError(
      "xid-backends-error",
      section,
  );

  const body = byId("xid-backend-list");
  const rows = section?.data ?? [];

  body.replaceChildren();

  if (rows.length === 0) {
    tableEmpty(
        body,
        6,
        "Backend с backend_xmin не найден.",
    );
    return;
  }

  for (const backend of rows) {
    const row = document.createElement("tr");

    const wait = [
      backend.wait_event_type,
      backend.wait_event,
    ]
        .filter(Boolean)
        .join(" / ") || "—";

    const identity = [
      backend.username,
      backend.application_name,
    ]
        .filter(Boolean)
        .join(" / ") || "—";

    appendCells(
        row,
        [
          String(backend.pid),
          identity,
          backend.state ?? "—",
          formatInteger(backend.xmin_age),
          formatDuration(backend.transaction_age_seconds),
          wait,
        ],
    );

    body.append(row);
  }
}

function renderXidSlots(section) {
  renderSectionError(
      "xid-slots-error",
      section,
  );

  const body = byId("xid-slot-list");
  const rows = section?.data ?? [];

  body.replaceChildren();

  if (rows.length === 0) {
    tableEmpty(
        body,
        5,
        "Replication slot, удерживающий XMIN, не найден.",
    );
    return;
  }

  for (const slot of rows) {
    const row = document.createElement("tr");

    appendCells(
        row,
        [
          slot.slot_name,
          [slot.slot_type, slot.database]
              .filter(Boolean)
              .join(" / "),
          yesNo(slot.active),
          formatInteger(slot.xmin_age),
          formatInteger(slot.catalog_xmin_age),
        ],
    );

    body.append(row);
  }
}

function renderXidPrepared(section) {
  renderSectionError(
      "xid-prepared-error",
      section,
  );

  const summary = byId("xid-prepared-summary");
  const data = section?.data;

  const wrap = byId("xid-prepared-examples-wrap");
  const body = byId("xid-prepared-examples");
  body.replaceChildren();
  wrap.hidden = true;
  if (!data) {
    summary.textContent = section?.error
        ? "Секция недоступна; число prepared transactions неизвестно."
        : "Нет данных.";
    return;
  }

  summary.textContent =
      `Количество: ${formatInteger(data.count)} · самая старая: ${formatDuration(data.oldest_age_seconds)}`;
  const examples = data.examples ?? [];
  wrap.hidden = examples.length === 0;
  for (const transaction of examples) {
    const row = document.createElement("tr");
    appendCells(row, [
      transaction.gid, transaction.xid, transaction.database,
      transaction.owner, formatTimestamp(transaction.prepared_at),
      formatDuration(transaction.age_seconds),
    ]);
    body.append(row);
  }
}

function postgresDocsVersion() {
  if (!xidServerVersionNum) {
    return "current";
  }

  const major = Math.floor(
      Number(xidServerVersionNum) / 10_000,
  );

  return major >= 10
      ? String(major)
      : "current";
}

function postgresDocsUrl(path) {
  return `https://www.postgresql.org/docs/${postgresDocsVersion()}/${path}`;
}

function renderXidSources() {
  const list = byId("xid-source-list");
  const versionElement = byId("xid-source-version");

  list.replaceChildren();

  if (!xidReference) {
    versionElement.textContent =
        "Справка ещё не загружена.";
    return;
  }

  const version = postgresDocsVersion();

  versionElement.textContent = version === "current"
      ? "До первого снимка ссылки построены на current-документацию PostgreSQL."
      : `Ссылки построены для PostgreSQL ${version}, соответствующего последнему снимку.`;

  for (const source of xidReference.sources ?? []) {
    const card = document.createElement("article");
    const title = document.createElement("strong");
    const purpose = document.createElement("p");
    const url = document.createElement("a");

    card.className = "source-card";
    title.textContent = source.title;
    purpose.textContent = source.purpose;
    url.className = "source-url";
    url.href = postgresDocsUrl(source.path);
    url.target = "_blank";
    url.rel = "noopener noreferrer";
    url.textContent = url.href;

    card.append(
        title,
        purpose,
        url,
    );

    list.append(card);
  }
}

function renderXidMethodology() {
  const list = byId("xid-methodology-list");

  list.replaceChildren();

  if (!xidReference) {
    const empty = document.createElement("p");

    empty.className = "empty-state";
    empty.textContent = "Методика недоступна.";

    list.append(empty);
    return;
  }

  xidReference.methodology.forEach((step, index) => {
    const card = document.createElement("article");
    const heading = document.createElement("div");
    const number = document.createElement("span");
    const title = document.createElement("h3");
    const purpose = document.createElement("p");
    const items = document.createElement("ul");

    card.className = "reference-card";
    heading.className = "reference-heading";
    number.className = "reference-index";

    number.textContent = String(index + 1);
    title.textContent = step.title;
    purpose.textContent = step.purpose;

    heading.append(
        number,
        title,
    );

    for (const text of step.how_to_read) {
      const item = document.createElement("li");

      item.textContent = text;
      items.append(item);
    }

    card.append(
        heading,
        purpose,
        items,
    );

    list.append(card);
  });
}

function renderXidQueries() {
  const list = byId("xid-query-list");

  list.replaceChildren();

  if (!xidReference) {
    const empty = document.createElement("p");

    empty.className = "empty-state";
    empty.textContent = "SQL reference недоступен.";

    list.append(empty);
    return;
  }

  for (const query of xidReference.queries) {
    const details = document.createElement("details");
    const summary = document.createElement("summary");
    const purpose = document.createElement("p");
    const outputTitle = document.createElement("strong");
    const output = document.createElement("ul");
    const readTitle = document.createElement("strong");
    const read = document.createElement("ul");
    const sqlTitle = document.createElement("strong");
    const pre = document.createElement("pre");
    const code = document.createElement("code");

    details.className = "query-reference";
    summary.textContent = query.title;
    purpose.textContent = query.purpose;
    outputTitle.textContent = "На выходе";
    readTitle.textContent = "Как читать";
    sqlTitle.textContent = "SQL";

    for (const value of query.output) {
      const item = document.createElement("li");

      item.textContent = value;
      output.append(item);
    }

    for (const value of query.how_to_read) {
      const item = document.createElement("li");

      item.textContent = value;
      read.append(item);
    }

    code.textContent = query.sql;
    pre.className = "sql-block";
    pre.append(code);

    details.append(
        summary,
        purpose,
        outputTitle,
        output,
        readTitle,
        read,
        sqlTitle,
        pre,
    );

    list.append(details);
  }
}

function renderXidReference() {
  renderXidMethodology();
  renderXidQueries();
  renderXidSources();
}

async function loadXidReference() {
  if (xidReference) {
    renderXidReference();
    return xidReference;
  }

  if (xidReferencePromise) {
    return xidReferencePromise;
  }

  xidReferencePromise = invoke("xid_reference")
      .then((reference) => {
        xidReference = reference;
        renderXidReference();
        return reference;
      })
      .catch((error) => {
        const text =
            `Не удалось загрузить XID reference: ${String(error)}`;

        for (const id of [
          "xid-methodology-list",
          "xid-query-list",
        ]) {
          const target = byId(id);

          target.replaceChildren();

          const status = document.createElement("p");

          status.className = "section-error";
          status.textContent = text;

          target.append(status);
        }

        return null;
      })
      .finally(() => {
        xidReferencePromise = null;
      });

  return xidReferencePromise;
}

function renderXidInterpretation(interpretation, snapshot) {
  const list = byId("xid-interpretation-list");
  const limitationsDetails = byId(
      "xid-interpretation-limitations",
  );
  const limitationsList = byId(
      "xid-interpretation-limitations-list",
  );

  list.replaceChildren();
  limitationsList.replaceChildren();

  if (!interpretation) {
    const empty = document.createElement("p");

    empty.className = "empty-state";
    empty.textContent = "Интерпретация отсутствует в этом snapshot.";

    list.append(empty);
    limitationsDetails.hidden = true;
    return;
  }

  const items = interpretation.items ?? [];

  if (items.length === 0) {
    const empty = document.createElement("p");

    empty.className = "empty-state";
    empty.textContent = "В snapshot нет сигналов, требующих отдельного пояснения.";
    list.append(empty);
  }

  items.forEach((item, index) => {
    const card = document.createElement("details");
    const summary = document.createElement("summary");
    const summaryText = document.createElement("span");
    const preview = document.createElement("small");

    card.className = "interpretation-card xid-finding";
    card.open = index === 0;
    summaryText.textContent = item.title;
    preview.textContent = item.observed?.[0] ?? "Открыть подробности";
    summary.append(summaryText, preview);
    card.append(summary);

    const groups = [
      ["Что обнаружено", item.observed ?? []],
      ["Что это может означать", item.meaning ?? []],
      ["Что проверить следующим", item.next_steps ?? []],
    ];

    for (const [headingText, lines] of groups) {
      const group = document.createElement("div");
      const heading = document.createElement("strong");
      const values = document.createElement("ul");

      group.className = "interpretation-group";
      heading.textContent = headingText;

      for (const line of lines) {
        const li = document.createElement("li");
        li.textContent = line;
        values.append(li);
      }

      group.append(heading, values);
      card.append(group);
    }

    if (item.id === "oldest-relation") {
      const oldest = snapshot?.relations?.data?.[0];

      if (oldest) {
        const action = document.createElement("button");
        action.className = "secondary xid-finding-action";
        action.type = "button";
        action.textContent = "Разобрать relation и историю";
        action.addEventListener("click", () => {
          void openXidRelation(oldest.relation_oid);
        });
        card.append(action);
      }
    }

    list.append(card);
  });

  const limitations = interpretation.limitations ?? [];

  limitationsDetails.hidden = limitations.length === 0;

  for (const limitation of limitations) {
    const li = document.createElement("li");
    li.textContent = limitation;
    limitationsList.append(li);
  }
}

function renderXidRelationOptions(snapshot, query = "") {
  const select = byId("xid-relation-select");
  const relations = snapshot?.relations?.data ?? [];
  const normalizedQuery = query.trim().toLocaleLowerCase("ru");
  const filtered = normalizedQuery
      ? relations.filter((relation) => {
        const searchable = [
          relation.schema_name,
          relation.relation_name,
          relation.parent_schema,
          relation.parent_relation,
          relation.relation_oid,
        ]
            .filter((value) => value !== null && value !== undefined)
            .join(" ")
            .toLocaleLowerCase("ru");

        return searchable.includes(normalizedQuery);
      })
      : relations;
  const previous = xidSelectedRelationOid;

  select.replaceChildren();

  if (filtered.length === 0) {
    const option = document.createElement("option");
    option.value = "";
    option.textContent = relations.length === 0
        ? "Relations недоступны"
        : "Совпадений нет";
    select.append(option);
    select.disabled = true;
    return;
  }

  select.disabled = false;

  for (const relation of filtered) {
    const option = document.createElement("option");
    option.value = String(relation.relation_oid);
    option.textContent =
        `${xidRelationDisplayName(relation)} · age ${formatInteger(relation.xid_age)} · ${formatPercent(relation.pct_of_freeze_max)}`;
    select.append(option);
  }

  const selected = filtered.some(
      (relation) => relation.relation_oid === previous,
  )
      ? previous
      : filtered[0].relation_oid;

  xidSelectedRelationOid = selected;
  select.value = String(selected);
}

function xidRelationFromCurrentSnapshot(relationOid) {
  return (xidCurrentSnapshot?.relations?.data ?? [])
      .find((relation) => relation.relation_oid === relationOid) ?? null;
}

function xidTriggerFromCurrentSnapshot(relationOid) {
  return (xidCurrentSnapshot?.autovacuum_triggers?.data ?? [])
      .find((trigger) => trigger.relation_oid === relationOid) ?? null;
}

function xidVacuumFromCurrentSnapshot(relationOid) {
  return (xidCurrentSnapshot?.vacuums?.data ?? [])
      .find((vacuum) => vacuum.relation_oid === relationOid) ?? null;
}

function xidTableHealthTarget(relation) {
  if (!relation) {
    return null;
  }

  if (relation.relation_type === "t") {
    if (
        !relation.parent_oid
        || relation.parent_schema === "pg_catalog"
        || relation.parent_schema === "information_schema"
        || relation.parent_schema?.startsWith("pg_toast")
    ) {
      return null;
    }

    return relation.parent_oid;
  }

  if (relation.relation_type !== "r") {
    return null;
  }

  if (
      relation.schema_name === "pg_catalog"
      || relation.schema_name === "information_schema"
      || relation.schema_name.startsWith("pg_toast")
  ) {
    return null;
  }

  return relation.relation_oid;
}

function renderXidRelationContext(relation) {
  const trigger = xidTriggerFromCurrentSnapshot(relation.relation_oid);
  const vacuum = xidVacuumFromCurrentSnapshot(relation.relation_oid);
  const headroom = relation.effective_freeze_max_age - relation.xid_age;

  resetXidPolicyAdvisor("Policy Advisor ожидает сохранённую history выбранной relation.");

  byId("xid-relation-title").textContent = xidRelationDisplayName(relation);
  byId("xid-relation-meta").textContent =
      `OID ${relation.relation_oid} · ${xidRelationType(relation.relation_type)} · snapshot ${formatTimestamp(xidCurrentSnapshot?.overview?.measured_at)}`;
  byId("xid-relation-age-detail").textContent = formatInteger(relation.xid_age);
  byId("xid-relation-headroom").textContent = formatSignedInteger(headroom);
  byId("xid-relation-mxid-age").textContent = formatInteger(relation.mxid_age);
  byId("xid-relation-mxid-pct").textContent = formatPercent(relation.pct_of_multixact_freeze_max);
  byId("xid-relation-pct").textContent = formatPercent(relation.pct_of_freeze_max);
  byId("xid-relation-size").textContent = formatBytes(relation.total_size_bytes);
  byId("xid-relation-visible").textContent = formatPercent(relation.all_visible_pct);

  byId("xid-relation-freeze-max").textContent = formatInteger(relation.effective_freeze_max_age);
  byId("xid-relation-freeze-max-source").textContent = relation.freeze_max_source ?? "—";
  byId("xid-relation-freeze-min").textContent = formatInteger(relation.effective_freeze_min_age);
  byId("xid-relation-freeze-min-source").textContent = relation.freeze_min_source ?? "—";
  byId("xid-relation-freeze-table-age").textContent = formatInteger(relation.effective_freeze_table_age);
  byId("xid-relation-freeze-table-source").textContent = relation.freeze_table_source ?? "—";
  byId("xid-relation-reloptions").textContent = (relation.reloptions ?? []).join(", ") || "global defaults";
  byId("xid-relation-parent").textContent = relation.parent_relation
      ? `${relation.parent_schema}.${relation.parent_relation} · parent age ${formatInteger(relation.parent_xid_age)}`
      : "—";

  byId("xid-relation-last-vacuum").textContent = formatTimestamp(relation.last_vacuum);
  byId("xid-relation-last-autovacuum").textContent = formatTimestamp(relation.last_autovacuum);
  byId("xid-relation-vacuum-counts").textContent =
      `${formatInteger(relation.vacuum_count)} / ${formatInteger(relation.autovacuum_count)}`;
  byId("xid-relation-last-analyze").textContent = formatTimestamp(relation.last_analyze);
  byId("xid-relation-last-autoanalyze").textContent = formatTimestamp(relation.last_autoanalyze);
  byId("xid-relation-analyze-counts").textContent =
      `${formatInteger(relation.analyze_count)} / ${formatInteger(relation.autoanalyze_count)}`;
  byId("xid-relation-mod-since-analyze").textContent = formatInteger(relation.modifications_since_analyze);

  if (trigger) {
    byId("xid-relation-trigger-dead").textContent = trigger.thresholds_reliable === false
        ? "Неизвестно: reltuples ещё не оценён PostgreSQL"
        : `${formatInteger(trigger.estimated_dead_rows)} / ${formatInteger(trigger.estimated_dead_tuple_trigger)} · ${trigger.dead_trigger_reached ? "reached" : "not reached"}`;
    byId("xid-relation-trigger-insert").textContent = trigger.thresholds_reliable === false
        ? "Неизвестно: reltuples ещё не оценён PostgreSQL"
        : (trigger.effective_insert_threshold < 0)
            ? "disabled"
        : `${formatInteger(trigger.inserts_since_vacuum)} / ${formatInteger(trigger.estimated_insert_trigger)} · ${trigger.insert_trigger_reached ? "reached" : "not reached"}`;
  } else {
    byId("xid-relation-trigger-dead").textContent = "—";
    byId("xid-relation-trigger-insert").textContent = "—";
  }

  byId("xid-relation-current-vacuum").textContent = vacuum
      ? `PID ${vacuum.pid} · ${vacuum.phase} · ${formatPercent(vacuum.scan_pct)}${vacuum.is_wraparound_vacuum ? " · anti-wraparound" : ""}`
      : "не виден в snapshot";

  byId("xid-relation-estimated-rows").textContent = formatInteger(relation.estimated_rows);
  byId("xid-relation-heap-pages").textContent = formatInteger(relation.heap_pages);
  byId("xid-relation-all-visible-pages").textContent = formatInteger(relation.all_visible_pages);
  byId("xid-relation-all-frozen-pages").textContent = formatInteger(relation.all_frozen_pages);
  byId("xid-relation-all-visible-pct").textContent = formatPercent(relation.all_visible_pct);

  let nextStep = "Наблюдать динамику XID age по следующим snapshots; менять freeze policy по одной точке не требуется.";

  if (relation.xid_age >= relation.effective_freeze_max_age) {
    nextStep = "Relation достигла configured freeze max: сначала проверить anti-wraparound VACUUM, blockers и динамику age, затем решать, нужна ли ручная операция или изменение policy.";
  } else if (trigger && !trigger.autovacuum_enabled) {
    nextStep = "Normal autovacuum для relation выключен. Freeze protection нужно оценивать отдельно по XID age, anti-wraparound VACUUM и history; normal DML trigger здесь не является ожидаемым запуском worker.";
  } else if (trigger?.dead_trigger_reached || trigger?.insert_trigger_reached) {
    nextStep = vacuum
        ? "Normal autovacuum trigger достигнут и VACUUM уже виден. Дождаться следующего snapshot и проверить, продвинулся ли freeze horizon."
        : "Normal autovacuum trigger достигнут, но VACUUM в этом мгновенном snapshot не виден. Повторить snapshot и при устойчивом состоянии перейти в Table Health.";
  }

  byId("xid-relation-next-step").textContent = nextStep;

  const tableHealthTarget = xidTableHealthTarget(relation);
  const tableHealthButton = byId("xid-open-table-health");
  tableHealthButton.disabled = tableHealthTarget === null;
  tableHealthButton.dataset.relationOid = tableHealthTarget === null
      ? ""
      : String(tableHealthTarget);

  byId("xid-relation-empty").hidden = true;
  byId("xid-relation-content").hidden = false;
}

function xidPolicyStatusLabel(status) {
  const labels = {
    attention: "требует внимания",
    candidate: "candidate",
    insufficient_history: "нужна history",
    insufficient_quality: "данные несопоставимы",
    observe: "наблюдать",
    review: "проверить",
    working: "maintenance работает",
  };

  return labels[status] ?? status ?? "—";
}

function resetXidPolicyAdvisor(message = "Policy evidence ещё не рассчитан.") {
  const status = byId("xid-policy-status");
  const candidates = byId("xid-policy-candidates");
  const observations = byId("xid-policy-observations");
  const limitations = byId("xid-policy-limitations");
  const error = byId("xid-policy-error");

  status.textContent = "—";
  status.dataset.state = "";
  byId("xid-policy-summary").textContent = message;
  byId("xid-policy-history").textContent = "—";
  byId("xid-policy-rate").textContent = "—";
  byId("xid-policy-horizon-advance").textContent = "—";
  byId("xid-policy-vacuum-cycles").textContent = "—";
  candidates.replaceChildren();
  observations.replaceChildren();
  limitations.replaceChildren();
  byId("xid-policy-evidence").open = false;
  error.hidden = true;
  error.textContent = "";
}

function renderXidPolicyTextList(container, values, emptyText) {
  container.replaceChildren();

  const rows = values ?? [];

  if (rows.length === 0) {
    const item = document.createElement("li");
    item.textContent = emptyText;
    container.append(item);
    return;
  }

  for (const value of rows) {
    const item = document.createElement("li");
    item.textContent = value;
    container.append(item);
  }
}

async function applyXidPolicyCandidate(candidate) {
  const connectionId = Number(xidCurrentConnectionId);
  const snapshotId = Number(xidCurrentSnapshotId);
  const relationOid = Number(xidSelectedRelationOid);

  if (
      !Number.isInteger(connectionId)
      || connectionId <= 0
      || !Number.isInteger(snapshotId)
      || snapshotId <= 0
      || !Number.isInteger(relationOid)
      || relationOid <= 0
  ) {
    message("Для apply нужен сохранённый XID snapshot и выбранная relation.", true);
    return;
  }

  const confirmed = window.confirm([
    "Применить XID freeze policy?",
    "",
    `Target: ${candidate.ddl_target}`,
    `${candidate.parameter}: ${formatInteger(candidate.current_value)} → ${formatInteger(candidate.candidate_value)}`,
    "",
    "Перед ALTER pgXray повторно проверит relation identity, effective policy и active VACUUM. При несовпадении изменение не выполняется.",
    "",
    candidate.sql,
  ].join("\n"));

  if (!confirmed) {
    return;
  }

  await action(async () => {
    const result = await invoke("apply_xid_policy_candidate", {
      connectionId,
      snapshotId,
      relationOid,
      candidateId: candidate.id,
    });

    const beforeValue = result.verification?.before
        ? result.candidate.current_value
        : candidate.current_value;
    const afterValue = result.verification?.candidate_value
        ?? candidate.candidate_value;

    if (result.snapshot && result.history) {
      renderXidSnapshot(result.snapshot, result.history);
      await openXidRelation(relationOid, false);
      void refreshXidHistory();
    } else {
      resetXidPolicyAdvisor(
          "Изменение применено и verified, но post-change snapshot не сохранён. Соберите новый XID snapshot вручную перед следующим apply.",
      );
    }

    const suffix = result.warning
        ? ` ${result.warning}`
        : ` Post-change snapshot #${result.history.id} сохранён.`;

    message(
        `XID policy применена: ${result.candidate.parameter} ${formatInteger(beforeValue)} → ${formatInteger(afterValue)}.${suffix}`,
    );
  }, {
    startMessage: "Проверка guardrails и применение XID freeze policy…",
  });
}

function renderXidPolicyCandidate(candidate, primaryCandidateId) {
  const card = document.createElement("article");
  const title = document.createElement("h4");
  const target = document.createElement("p");
  const change = document.createElement("div");
  const parameter = document.createElement("div");
  const current = document.createElement("div");
  const next = document.createElement("div");
  const sql = document.createElement("pre");
  const actions = document.createElement("div");
  const applyButton = document.createElement("button");
  const details = document.createElement("details");
  const summary = document.createElement("summary");
  const evidence = document.createElement("div");

  card.className = "xid-policy-candidate";

  if (candidate.id === primaryCandidateId) {
    card.classList.add("primary");
  }

  title.textContent = candidate.title;
  target.className = "hint";
  target.textContent = `DDL target: ${candidate.ddl_target}`;
  change.className = "xid-policy-change";

  for (const [container, label, value] of [
    [parameter, "Параметр", candidate.parameter],
    [current, "Сейчас", formatInteger(candidate.current_value)],
    [next, "Candidate", formatInteger(candidate.candidate_value)],
  ]) {
    const name = document.createElement("span");
    const strong = document.createElement("strong");
    name.textContent = label;
    strong.textContent = value;
    container.append(name, strong);
  }

  change.append(parameter, current, next);
  sql.textContent = candidate.sql;
  actions.className = "xid-policy-actions";
  applyButton.type = "button";
  applyButton.className = candidate.id === primaryCandidateId ? "danger" : "secondary";
  applyButton.textContent = "Применить policy";
  applyButton.addEventListener("click", () => {
    void applyXidPolicyCandidate(candidate);
  });
  actions.append(applyButton);
  summary.textContent = "Почему, ожидаемый эффект и trade-offs";
  evidence.className = "xid-policy-evidence-grid";

  for (const [heading, values] of [
    ["Почему появился candidate", candidate.why],
    ["Ожидаемый эффект", candidate.expected_effect],
    ["Trade-offs", candidate.tradeoffs],
  ]) {
    const group = document.createElement("div");
    const strong = document.createElement("strong");
    const list = document.createElement("ul");
    strong.textContent = heading;
    renderXidPolicyTextList(list, values, "нет");
    group.append(strong, list);
    evidence.append(group);
  }

  details.append(summary, evidence);
  card.append(title, target, change, sql, actions, details);

  return card;
}

function renderXidPolicyAdvice(advice) {
  const status = byId("xid-policy-status");
  const candidates = byId("xid-policy-candidates");

  status.textContent = xidPolicyStatusLabel(advice.status);
  status.dataset.state = advice.status ?? "";
  byId("xid-policy-summary").textContent = `${advice.title}. ${advice.summary}`;
  byId("xid-policy-history").textContent = advice.history_span_seconds === null
      ? `${formatInteger(advice.history_points)} point(s)`
      : `${formatInteger(advice.history_points)} · ${formatDuration(Math.round(advice.history_span_seconds))}`;
  byId("xid-policy-rate").textContent = advice.xid_rate_per_hour === null
      ? "—"
      : `${formatInteger(Math.round(advice.xid_rate_per_hour))} / hour`;
  byId("xid-policy-horizon-advance").textContent = advice.estimated_horizon_advance === null
      ? "—"
      : formatInteger(advice.estimated_horizon_advance);
  byId("xid-policy-vacuum-cycles").textContent = advice.vacuum_cycles_delta === null
      ? "—"
      : formatInteger(advice.vacuum_cycles_delta);

  candidates.replaceChildren();

  if ((advice.candidates ?? []).length === 0) {
    const empty = document.createElement("p");
    empty.className = "empty-state";
    empty.textContent = "Числовой policy candidate не сформирован. Это ожидаемо, если history недостаточна или freeze maintenance уже продвигает horizon.";
    candidates.append(empty);
  } else {
    for (const candidate of advice.candidates) {
      candidates.append(
          renderXidPolicyCandidate(candidate, advice.primary_candidate_id),
      );
    }
  }

  renderXidPolicyTextList(
      byId("xid-policy-observations"),
      advice.observations,
      "наблюдения отсутствуют",
  );
  renderXidPolicyTextList(
      byId("xid-policy-limitations"),
      advice.limitations,
      "ограничения не заявлены",
  );
}

async function loadXidRelationPolicy(snapshotId, relationOid) {
  resetXidPolicyAdvisor("Расчёт policy evidence по сохранённой XID history…");

  try {
    const advice = await invoke("xid_relation_policy", {
      snapshotId,
      relationOid,
      limit: 100,
    });

    if (xidSelectedRelationOid !== relationOid || Number(xidCurrentSnapshotId) !== snapshotId) {
      return;
    }

    renderXidPolicyAdvice(advice);
  } catch (policyError) {
    if (xidSelectedRelationOid !== relationOid) {
      return;
    }

    const error = byId("xid-policy-error");
    error.hidden = false;
    error.textContent = `Не удалось рассчитать Policy Advisor: ${String(policyError)}`;
    byId("xid-policy-summary").textContent = "Policy Advisor недоступен для выбранной relation.";
  }
}

function renderXidRelationHistory(points) {
  const body = byId("xid-relation-history-list");
  const note = byId("xid-relation-history-note");

  body.replaceChildren();

  if (points.length === 0) {
    tableEmpty(body, 9, "Для relation ещё нет сохранённой истории.");
    note.textContent = "Нужен минимум один сохранённый XID snapshot с этой relation.";
    return;
  }

  const hasLegacy = points.some((point) => point.schema_version < 2);
  note.textContent = hasLegacy
      ? "Есть legacy snapshot schema=1: он сохранял только TOP-15 relations, поэтому пропуски в старой истории не означают отсутствие relation."
      : `Найдено ${formatInteger(points.length)} сохранённых точек. Новые snapshots содержат полный relation set.`;

  points.forEach((point, index) => {
    const row = document.createElement("tr");
    const older = points[index + 1];
    const delta = older
        ? point.relation.xid_age - older.relation.xid_age
        : null;
    const headroom = point.relation.effective_freeze_max_age - point.relation.xid_age;
    const trigger = point.trigger;
    const triggerState = trigger
        ? [
          trigger.dead_trigger_reached ? "dead reached" : null,
          trigger.insert_trigger_reached ? "insert reached" : null,
        ].filter(Boolean).join(" · ") || "not reached"
        : "—";

    const vacuumState = point.active_vacuum
        ? `PID ${point.active_vacuum.pid} · ${point.active_vacuum.phase}${point.active_vacuum.is_wraparound_vacuum ? " · anti-wraparound" : ""}`
        : "—";

    appendCells(row, [
      `#${point.snapshot_id}`,
      formatTimestamp(point.measured_at),
      formatInteger(point.relation.xid_age),
      formatSignedInteger(delta),
      formatSignedInteger(headroom),
      formatPercent(point.relation.pct_of_freeze_max),
      formatTimestamp(point.relation.last_autovacuum),
      triggerState,
      vacuumState,
    ]);

    body.append(row);
  });
}

async function openXidRelation(relationOid, switchTab = true) {
  const relation = xidRelationFromCurrentSnapshot(relationOid);

  if (!relation) {
    return;
  }

  xidSelectedRelationOid = relationOid;
  byId("xid-relation-select").value = String(relationOid);
  renderXidRelationContext(relation);

  if (switchTab) {
    setXidTab("relation");
  }

  const error = byId("xid-relation-history-error");
  error.hidden = true;
  error.textContent = "";
  tableEmpty(byId("xid-relation-history-list"), 9, "Загрузка history…");

  try {
    const snapshotId = Number(xidCurrentSnapshotId);

    if (!Number.isInteger(snapshotId) || snapshotId <= 0) {
      tableEmpty(byId("xid-relation-history-list"), 9, "Snapshot ещё не сохранён.");
      resetXidPolicyAdvisor("Snapshot ещё не сохранён; Policy Advisor использует persisted history.");
      return;
    }

    void loadXidRelationPolicy(snapshotId, relationOid);

    const points = await invoke("xid_relation_history", {
      snapshotId,
      relationOid,
      limit: 100,
    });

    if (xidSelectedRelationOid !== relationOid) {
      return;
    }

    renderXidRelationHistory(points);
  } catch (historyError) {
    error.hidden = false;
    error.textContent = `Не удалось загрузить relation history: ${String(historyError)}`;
    tableEmpty(byId("xid-relation-history-list"), 9, "History недоступна.");
  }
}

function historyConnectionLabel(connectionId, historyRows) {
  const current =
      connectionsById.get(connectionId);

  if (current) {
    return `${current.name} — ${current.username}@${current.host}:${current.port}/${current.database}`;
  }

  const latest =
      historyRows.find(
          (row) => row.connection_id === connectionId,
      );

  if (!latest) {
    return `Подключение #${connectionId}`;
  }

  return `${latest.connection_name} — ${latest.username}@${latest.host}:${latest.port}/${latest.database} · сохранённая история`;
}

function renderXidHistoryConnectionOptions(historyRows) {
  const select =
      byId("xid-history-connection");

  const previous =
      select.value;

  const connectionIds =
      new Set(
          Array.from(
              connectionsById.keys(),
          ),
      );

  for (const row of historyRows) {
    connectionIds.add(
        row.connection_id,
    );
  }

  select.replaceChildren();

  const sortedIds =
      Array.from(
          connectionIds,
      )
          .sort((left, right) => {
            const leftLabel =
                historyConnectionLabel(
                    left,
                    historyRows,
                );

            const rightLabel =
                historyConnectionLabel(
                    right,
                    historyRows,
                );

            return leftLabel.localeCompare(
                rightLabel,
                "ru",
            );
          });

  if (sortedIds.length === 0) {
    const option =
        document.createElement("option");

    option.value = "";
    option.textContent =
        "Нет подключений и сохранённой истории";

    select.append(option);
    select.disabled = true;
    byId("refresh-xid-history").disabled = true;

    return;
  }

  select.disabled = false;
  byId("refresh-xid-history").disabled = false;

  for (const connectionId of sortedIds) {
    const option =
        document.createElement("option");

    option.value =
        String(connectionId);

    option.textContent =
        historyConnectionLabel(
            connectionId,
            historyRows,
        );

    select.append(option);
  }

  if (
      previous
      && sortedIds.some(
          (id) => String(id) === previous,
      )
  ) {
    select.value = previous;
  }
}

function xidPageWindow(total, page, pageSize) {
  const totalPages = Math.max(1, Math.ceil(total / pageSize));
  const safePage = Math.min(Math.max(page, 1), totalPages);
  const start = (safePage - 1) * pageSize;
  const end = Math.min(start + pageSize, total);

  return {
    page: safePage,
    totalPages,
    start,
    end,
  };
}

function renderXidPagers(kind, total, page, pageSize) {
  const range = xidPageWindow(total, page, pageSize);

  document.querySelectorAll(`[data-xid-pager="${kind}"]`).forEach((container) => {
    container.replaceChildren();
    container.hidden = total === 0;

    if (total === 0) {
      return;
    }

    const navigation = document.createElement("div");
    const previous = document.createElement("button");
    const status = document.createElement("span");
    const next = document.createElement("button");
    const sizeLabel = document.createElement("label");
    const sizeSelect = document.createElement("select");

    navigation.className = "history-pager-navigation";
    previous.type = "button";
    previous.className = "secondary";
    previous.dataset.xidPagerKind = kind;
    previous.dataset.xidPagerAction = "previous";
    previous.textContent = "← Назад";
    previous.disabled = range.page <= 1;

    status.className = "history-pager-status";
    const totalLabel = kind === "history"
        ? `${formatInteger(total)} загруженных`
        : formatInteger(total);
    status.textContent =
        `${formatInteger(range.start + 1)}–${formatInteger(range.end)} из ${totalLabel} · страница ${formatInteger(range.page)}/${formatInteger(range.totalPages)}`;

    next.type = "button";
    next.className = "secondary";
    next.dataset.xidPagerKind = kind;
    next.dataset.xidPagerAction = "next";
    next.textContent = "Вперёд →";
    next.disabled = range.page >= range.totalPages;

    sizeLabel.className = "history-pager-size";
    sizeLabel.append("На странице ");
    sizeSelect.dataset.xidPagerSize = kind;

    for (const size of XID_HISTORY_PAGE_SIZES) {
      const option = document.createElement("option");

      option.value = String(size);
      option.textContent = size === 500 ? "500 (макс.)" : String(size);
      sizeSelect.append(option);
    }

    sizeSelect.value = String(pageSize);
    sizeLabel.append(sizeSelect);
    navigation.append(previous, status, next);
    container.append(navigation, sizeLabel);
  });
}

function scrollXidPagerBlock(kind) {
  const target = kind === "history"
      ? byId("xid-saved-snapshots")
      : byId("xid-compare-relations-section");

  target?.scrollIntoView({
    behavior: "smooth",
    block: "start",
  });
}

function resetXidComparison() {
  xidRelationComparisonRows = [];
  xidRelationPage = 1;
  byId("xid-comparison-content").hidden = true;
  byId("xid-comparison-error").hidden = true;
  byId("xid-comparison-error").textContent = "";
  byId("xid-compare-relation-list").replaceChildren();
  renderXidPagers("relations", 0, 1, xidRelationPageSize);
}

function renderXidHistoryComparisonOptions(snapshots) {
  const first =
      byId("xid-compare-first");

  const second =
      byId("xid-compare-second");

  const previousFirst =
      first.value;

  const previousSecond =
      second.value;

  first.replaceChildren();
  second.replaceChildren();

  for (const snapshot of snapshots) {
    const text =
        `#${snapshot.id} · ${formatSnapshotTime(snapshot.collection_started_at_ms)} · DB age ${formatInteger(snapshot.database_xid_age)}`;

    for (const select of [
      first,
      second,
    ]) {
      const option =
          document.createElement("option");

      option.value =
          String(snapshot.id);

      option.textContent =
          text;

      select.append(option);
    }
  }

  const canCompare =
      snapshots.length >= 2;

  first.disabled =
      !canCompare;

  second.disabled =
      !canCompare;

  byId("compare-xid").disabled =
      !canCompare;

  if (!canCompare) {
    return;
  }

  const ids =
      new Set(
          snapshots.map(
              (snapshot) =>
                  String(snapshot.id),
          ),
      );

  first.value =
      ids.has(previousFirst)
          ? previousFirst
          : String(
              snapshots[1].id,
          );

  second.value =
      ids.has(previousSecond)
          ? previousSecond
          : String(
              snapshots[0].id,
          );

  if (first.value === second.value) {
    first.value =
        String(
            snapshots[1].id,
        );

    second.value =
        String(
            snapshots[0].id,
        );
  }
}

function renderXidHistoryPage() {
  const body = byId("xid-history-list");
  const range = xidPageWindow(
      xidHistorySnapshots.length,
      xidHistoryPage,
      xidHistoryPageSize,
  );

  xidHistoryPage = range.page;
  body.replaceChildren();

  for (const snapshot of xidHistorySnapshots.slice(range.start, range.end)) {
    const row = document.createElement("tr");
    const oldest = snapshot.oldest_relation
        ? `${snapshot.oldest_relation.relation_name} · ${formatInteger(snapshot.oldest_relation.xid_age)} / ${formatInteger(snapshot.oldest_relation.effective_freeze_max_age)}`
        : "—";

    appendCells(
        row,
        [
          `#${snapshot.id}`,
          formatSnapshotTime(
              snapshot.collection_started_at_ms,
          ),
          `${snapshot.host}:${snapshot.port}/${snapshot.database} · db oid ${snapshot.database_oid} · PG ${snapshot.server_version}`,
          snapshot.current_xid,
          formatInteger(
              snapshot.database_xid_age,
          ),
          oldest,
          formatInteger(
              snapshot.unavailable_sections,
          ),
        ],
    );

    body.append(row);
  }

  renderXidPagers(
      "history",
      xidHistorySnapshots.length,
      xidHistoryPage,
      xidHistoryPageSize,
  );
}

function renderXidHistory(snapshots) {
  xidHistorySnapshots = snapshots;

  const empty = byId("xid-history-empty");
  const content = byId("xid-history-content");
  const hasSnapshots = snapshots.length > 0;

  empty.hidden = hasSnapshots;
  content.hidden = !hasSnapshots;

  resetXidComparison();
  renderXidHistoryComparisonOptions(
      snapshots,
  );

  if (!hasSnapshots) {
    byId("xid-history-list").replaceChildren();
    renderXidPagers("history", 0, 1, xidHistoryPageSize);
    return;
  }

  renderXidHistoryPage();
}

async function refreshXidHistory() {
  const index =
      await invoke(
          "list_xid_snapshots",
          {
            connectionId: null,
            limit: 500,
          },
      );

  renderXidHistoryConnectionOptions(
      index,
  );

  const connectionId =
      Number(
          byId("xid-history-connection").value,
      );

  if (
      !Number.isInteger(connectionId)
      || connectionId <= 0
  ) {
    renderXidHistory([]);
    return;
  }

  const snapshots =
      await invoke(
          "list_xid_snapshots",
          {
            connectionId,
            limit: 500,
          },
      );

  renderXidHistory(
      snapshots,
  );
}

function appendTextList(
    container,
    title,
    values,
) {
  const group =
      document.createElement("div");

  const strong =
      document.createElement("strong");

  const list =
      document.createElement("ul");

  strong.textContent =
      title;

  const rows =
      values ?? [];

  if (rows.length === 0) {
    const item =
        document.createElement("li");

    item.textContent = "нет";
    list.append(item);
  } else {
    for (const value of rows) {
      const item =
          document.createElement("li");

      item.textContent =
          value;

      list.append(item);
    }
  }

  group.append(
      strong,
      list,
  );

  container.append(group);
}

function renderOldestRelationComparison(comparison) {
  const target =
      byId(
          "xid-compare-oldest-relation",
      );

  target.replaceChildren();

  const older =
      comparison.oldest_relation.older;

  const newer =
      comparison.oldest_relation.newer;

  appendTextList(
      target,
      "Older",
      [
        older
            ? `${older.relation_name} · age ${formatInteger(older.xid_age)} · headroom ${formatSignedInteger(comparison.oldest_relation.older_configured_headroom)}`
            : "нет данных",
      ],
  );

  appendTextList(
      target,
      "Newer",
      [
        newer
            ? `${newer.relation_name} · age ${formatInteger(newer.xid_age)} · headroom ${formatSignedInteger(comparison.oldest_relation.newer_configured_headroom)}`
            : "нет данных",
      ],
  );

  appendTextList(
      target,
      "Δ",
      [
        comparison.oldest_relation.xid_age_delta === null
            ? "Старейшая relation изменилась или секция недоступна — прямой Δ age не вычисляется."
            : `XID age: ${formatSignedInteger(comparison.oldest_relation.xid_age_delta)}`,
      ],
  );
}

function renderBlockerComparison(comparison) {
  const target =
      byId(
          "xid-compare-blockers",
      );

  target.replaceChildren();

  appendTextList(
      target,
      "Counts",
      [
        `backend_xmin: ${comparison.blockers.older_backend_count ?? "—"} → ${comparison.blockers.newer_backend_count ?? "—"}`,
        `slots: ${comparison.blockers.older_slot_count ?? "—"} → ${comparison.blockers.newer_slot_count ?? "—"}`,
        `prepared: ${comparison.blockers.older_prepared_count ?? "—"} → ${comparison.blockers.newer_prepared_count ?? "—"}`,
      ],
  );

  appendTextList(
      target,
      "Появились в новом sample",
      [
        ...(comparison.blockers.backend_new_in_sample ?? []),
        ...(comparison.blockers.slots_new_in_sample ?? []),
      ],
  );

  appendTextList(
      target,
      "Не видны в новом sample",
      [
        ...(comparison.blockers.backend_not_in_new_sample ?? []),
        ...(comparison.blockers.slots_not_in_new_sample ?? []),
      ],
  );
}

function renderVacuumComparison(comparison) {
  const target =
      byId(
          "xid-compare-vacuums",
      );

  target.replaceChildren();

  appendTextList(
      target,
      "Counts",
      [
        `active VACUUM: ${comparison.vacuums.older_active_count ?? "—"} → ${comparison.vacuums.newer_active_count ?? "—"}`,
        `anti-wraparound: ${comparison.vacuums.older_wraparound_count ?? "—"} → ${comparison.vacuums.newer_wraparound_count ?? "—"}`,
      ],
  );

  appendTextList(
      target,
      "В новом sample",
      comparison.vacuums
          .active_new_in_sample,
  );

  appendTextList(
      target,
      "Не видны в новом sample",
      comparison.vacuums
          .active_not_in_new_sample,
  );
}

function renderXidRelationComparisonPage() {
  const body = byId("xid-compare-relation-list");
  const range = xidPageWindow(
      xidRelationComparisonRows.length,
      xidRelationPage,
      xidRelationPageSize,
  );

  xidRelationPage = range.page;
  body.replaceChildren();

  if (xidRelationComparisonRows.length === 0) {
    tableEmpty(
        body,
        8,
        "Relation delta недоступна.",
    );
    renderXidPagers("relations", 0, 1, xidRelationPageSize);
    return;
  }

  for (const relation of xidRelationComparisonRows.slice(range.start, range.end)) {
    const row = document.createElement("tr");
    const name =
        relation.newer_name
        ?? relation.older_name
        ?? "—";

    appendCells(
        row,
        [
          String(
              relation.relation_oid,
          ),
          name,
          formatInteger(
              relation.older_xid_age,
          ),
          formatInteger(
              relation.newer_xid_age,
          ),
          formatSignedInteger(
              relation.xid_age_delta,
          ),
          formatPercent(
              relation.older_pct_of_freeze_max,
          ),
          formatPercent(
              relation.newer_pct_of_freeze_max,
          ),
          relationSampleStateLabel(
              relation.sample_state,
          ),
        ],
    );

    body.append(row);
  }

  renderXidPagers(
      "relations",
      xidRelationComparisonRows.length,
      xidRelationPage,
      xidRelationPageSize,
  );
}

function renderRelationComparison(comparison) {
  xidRelationComparisonRows = comparison.relation_changes ?? [];
  xidRelationPage = 1;
  renderXidRelationComparisonPage();
}

function renderXidComparison(comparison) {
  const rate =
      comparison.xid_rate;

  byId("xid-compare-interval").textContent =
      formatDuration(
          comparison.elapsed_seconds,
      );

  byId("xid-compare-delta").textContent =
      rate
          ? formatInteger(
              rate.xid_delta,
          )
          : "не рассчитан";

  byId("xid-compare-rate").textContent =
      rate
          ? formatNumber(
              rate.xid_per_minute,
              2,
          )
          : "не рассчитан";

  byId("xid-compare-db-age").textContent =
      formatSignedInteger(
          comparison
              .database_xid_age_delta,
      );

  byId("xid-compare-headroom").textContent =
      comparison
          .oldest_relation
          .newer_configured_headroom
      === null
          ? "—"
          : formatSignedInteger(
              comparison
                  .oldest_relation
                  .newer_configured_headroom,
          );

  byId("xid-compare-meta").textContent =
      `#${comparison.older.id} (${formatSnapshotTime(comparison.older.collection_started_at_ms)}) → #${comparison.newer.id} (${formatSnapshotTime(comparison.newer.collection_started_at_ms)}) · ${comparison.newer.host}:${comparison.newer.port}/${comparison.newer.database} · db oid ${comparison.newer.database_oid}`;

  renderOldestRelationComparison(
      comparison,
  );

  renderBlockerComparison(
      comparison,
  );

  renderVacuumComparison(
      comparison,
  );

  renderRelationComparison(
      comparison,
  );

  const limitations =
      byId(
          "xid-compare-limitations",
      );

  limitations.replaceChildren();

  for (
      const text
      of comparison.limitations ?? []
      ) {
    const item =
        document.createElement("li");

    item.textContent =
        text;

    limitations.append(item);
  }

  byId("xid-comparison-error").hidden = true;
  byId("xid-comparison-content").hidden = false;
}

function renderXidSnapshot(snapshot, history = null) {
  const overview = snapshot.overview;
  const relations = snapshot.relations?.data ?? [];
  const oldest = relations[0] ?? null;

  xidCurrentSnapshot = snapshot;
  xidCurrentConnectionId = history?.connection_id
      ?? Number(byId("xid-connection").value);
  xidCurrentSnapshotId = history?.id ?? null;
  xidServerVersionNum = overview.server_version_num;

  byId("xid-server-version").textContent = overview.server_version;
  byId("xid-database-name").textContent =
      `${overview.database} · OID ${overview.database_oid} · ${overview.username}`;
  byId("xid-current-xid").textContent = overview.current_xid;
  byId("xid-database-age").textContent = formatInteger(overview.database_xid_age);
  byId("xid-database-mxid-age").textContent = formatInteger(overview.database_mxid_age);
  byId("xid-oldest-relation").textContent = oldest
      ? `${oldest.schema_name}.${oldest.relation_name} · ${formatInteger(oldest.xid_age)}`
      : "—";
  byId("xid-relation-count").textContent = snapshot.relations?.data
      ? formatInteger(relations.length)
      : "—";
  byId("xid-freeze-max-age").textContent = formatInteger(overview.autovacuum_freeze_max_age);
  byId("xid-mxid-freeze-max").textContent = formatInteger(overview.autovacuum_multixact_freeze_max_age);
  byId("xid-autovacuum-state").textContent = overview.autovacuum ? "on" : "off";
  byId("xid-freeze-table-age").textContent = formatInteger(overview.vacuum_freeze_table_age);
  byId("xid-freeze-min-age").textContent = formatInteger(overview.vacuum_freeze_min_age);
  byId("xid-autovacuum-workers").textContent = formatInteger(overview.autovacuum_max_workers);
  byId("xid-autovacuum-cost-limit").textContent = formatInteger(overview.autovacuum_vacuum_cost_limit);
  byId("xid-autovacuum-cost-delay").textContent = overview.autovacuum_vacuum_cost_delay;
  byId("xid-stats-reset").textContent = formatTimestamp(overview.stats_reset);
  byId("xid-snapshot-meta").textContent =
      `Снимок: ${formatTimestamp(overview.measured_at)} · ${snapshot.relations?.data ? `${formatInteger(relations.length)} отношений` : "секция отношений недоступна"} · server_version_num=${overview.server_version_num}. Секции получены разными SQL в разное время; ошибки не означают отсутствие риска.`;

  byId("xid-relation-filter").value = "";
  renderXidRelationOptions(snapshot);
  renderXidInterpretation(snapshot.interpretation, snapshot);
  renderXidRelations(snapshot.relations);
  renderXidDatabases(snapshot.databases);
  renderXidBackends(snapshot.backends);
  renderXidSlots(snapshot.slots);
  renderXidPrepared(snapshot.prepared);
  renderXidVacuums(snapshot.vacuums);
  renderXidTriggers(snapshot.autovacuum_triggers);
  renderXidSources();

  byId("xid-empty").hidden = true;
  byId("xid-content").hidden = false;
  byId("xid-relation-empty").hidden = false;
  byId("xid-relation-content").hidden = true;
}

function resetXidSnapshot() {
  if (xidRelationFilterTimer !== null) {
    clearTimeout(xidRelationFilterTimer);
    xidRelationFilterTimer = null;
  }

  xidCurrentSnapshot = null;
  xidCurrentConnectionId = null;
  xidCurrentSnapshotId = null;
  xidSelectedRelationOid = null;
  xidServerVersionNum = null;

  byId("xid-empty").hidden = false;
  byId("xid-content").hidden = true;
  byId("xid-relation-empty").hidden = false;
  byId("xid-relation-content").hidden = true;
  byId("xid-relation-filter").value = "";
  byId("xid-relation-select").replaceChildren();
  byId("xid-relation-select").disabled = true;
  byId("xid-open-table-health").disabled = true;
  byId("xid-open-table-health").dataset.relationOid = "";
  byId("xid-interpretation-list").replaceChildren();
  byId("xid-interpretation-limitations-list").replaceChildren();
  byId("xid-interpretation-limitations").hidden = true;
  byId("xid-relation-history-list").replaceChildren();
  resetXidPolicyAdvisor();
  renderXidSources();
}

function initXidUi() {
  byId("xid-relation-filter").addEventListener("input", () => {
    if (xidRelationFilterTimer !== null) {
      clearTimeout(xidRelationFilterTimer);
    }

    xidRelationFilterTimer = setTimeout(() => {
      xidRelationFilterTimer = null;
      renderXidRelationOptions(
          xidCurrentSnapshot,
          byId("xid-relation-filter").value,
      );

      const relationOid = Number(byId("xid-relation-select").value);

      if (Number.isInteger(relationOid) && relationOid > 0) {
        void openXidRelation(relationOid, false);
      }
    }, 250);
  });

  byId("xid-relation-select").addEventListener("change", () => {
    const relationOid = Number(byId("xid-relation-select").value);

    if (Number.isInteger(relationOid) && relationOid > 0) {
      void openXidRelation(relationOid, false);
    }
  });

  byId("xid-open-table-health").addEventListener("click", () => {
    const relationOid = Number(byId("xid-open-table-health").dataset.relationOid);
    const connectionId = String(xidCurrentConnectionId ?? "");

    if (!Number.isInteger(relationOid) || relationOid <= 0) {
      return;
    }

    void action(async () => {
      setDiagnosticModule("autovacuum");
      byId("autovacuum-connection").value = connectionId;
      await loadAutovacuumTableDetail(relationOid);
      message("Открыт Table Health для relation из XID investigation.");
    });
  });

  byId("collect-xid").addEventListener("click", () => {
    const connectionId = Number(
        byId("xid-connection").value,
    );

    if (!Number.isInteger(connectionId) || connectionId <= 0) {
      message(
          "Сначала сохраните и выберите подключение PostgreSQL.",
          true,
      );
      return;
    }

    void action(async () => {
      const result = await invoke(
          "collect_xid_snapshot",
          {
            connectionId,
          },
      );

      renderXidSnapshot(
          result.snapshot,
          result.history,
      );

      message(
          `XID / Freeze snapshot #${result.history.id} собран и сохранён.`,
      );
    });
  });

  byId("refresh-xid-history").addEventListener("click", () => {
    void action(async () => {
      await refreshXidHistory();

      message(
          "История XID / Freeze обновлена.",
      );
    });
  });

  byId("xid-history-connection").addEventListener("change", () => {
    xidHistoryPage = 1;

    void action(async () => {
      await refreshXidHistory();

      message(
          "История XID / Freeze загружена.",
      );
    });
  });

  byId("xid-history-panel").addEventListener("click", (event) => {
    const button = event.target.closest("button[data-xid-pager-action]");

    if (!button) {
      return;
    }

    const kind = button.dataset.xidPagerKind;
    const direction = button.dataset.xidPagerAction === "next" ? 1 : -1;

    if (kind === "history") {
      xidHistoryPage += direction;
      renderXidHistoryPage();
    } else if (kind === "relations") {
      xidRelationPage += direction;
      renderXidRelationComparisonPage();
    } else {
      return;
    }

    scrollXidPagerBlock(kind);
  });

  byId("xid-history-panel").addEventListener("change", (event) => {
    const select = event.target.closest("select[data-xid-pager-size]");

    if (!select) {
      return;
    }

    const size = Number(select.value);
    if (!XID_HISTORY_PAGE_SIZES.includes(size)) {
      return;
    }

    const kind = select.dataset.xidPagerSize;

    if (kind === "history") {
      xidHistoryPageSize = size;
      xidHistoryPage = 1;
      renderXidHistoryPage();
    } else if (kind === "relations") {
      xidRelationPageSize = size;
      xidRelationPage = 1;
      renderXidRelationComparisonPage();
    } else {
      return;
    }

    scrollXidPagerBlock(kind);
  });

  byId("compare-xid").addEventListener("click", () => {
    const firstId =
        Number(
            byId("xid-compare-first").value,
        );

    const secondId =
        Number(
            byId("xid-compare-second").value,
        );

    if (
        !Number.isInteger(firstId)
        || !Number.isInteger(secondId)
        || firstId <= 0
        || secondId <= 0
    ) {
      return;
    }

    if (firstId === secondId) {
      const error =
          byId("xid-comparison-error");

      error.hidden = false;
      error.textContent =
          "Выберите два разных snapshots.";

      return;
    }

    resetXidComparison();

    void action(
        async () => {
          const comparison =
              await invoke(
                  "compare_xid_snapshots",
                  {
                    firstId,
                    secondId,
                  },
              );

          renderXidComparison(
              comparison,
          );
        },
        {
          startMessage: null,
          onError: (errorText) => {
            const error =
                byId(
                    "xid-comparison-error",
                );

            error.hidden = false;
            error.textContent =
                `Не удалось сравнить snapshots: ${errorText}`;
          },
        },
    );
  });
  document.querySelectorAll(".xid-tab").forEach((button) => {
    button.addEventListener("click", () => {
      const name = button.dataset.xidTab;

      setXidTab(name);

      if (name === "relation" && xidCurrentSnapshot) {
        const relationOid = Number(byId("xid-relation-select").value);

        if (Number.isInteger(relationOid) && relationOid > 0) {
          void openXidRelation(relationOid, false);
        }
      } else if (name === "methodology" || name === "queries") {
        void loadXidReference();
      }
    });
  });
}
