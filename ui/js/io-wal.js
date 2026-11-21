"use strict";

function renderIoWalConnectionOptions(connections) {
  const select = byId("io-wal-connection");
  const previous = select.value;
  select.replaceChildren();
  if (connections.length === 0) {
    const option = document.createElement("option");
    option.value = "";
    option.textContent = "Нет сохранённых подключений";
    select.append(option);
    select.disabled = true;
    byId("collect-io-wal").disabled = true;
    return;
  }
  select.disabled = false;
  byId("collect-io-wal").disabled = false;
  for (const connection of connections) {
    const option = document.createElement("option");
    option.value = String(connection.id);
    option.textContent = `${connection.name} — ${connection.username}@${connection.host}:${connection.port}/${connection.database}`;
    select.append(option);
  }
  if (previous && connections.some((connection) => String(connection.id) === previous)) {
    select.value = previous;
  }
}

function renderIoWalSectionError(id, section) {
  const target = byId(id);
  target.hidden = !section?.error;
  target.textContent = section?.error ? `Раздел недоступен: ${section.error}` : "";
}

function renderIoWalInterpretation(snapshot) {
  const target = byId("io-wal-interpretation-list");
  target.replaceChildren();
  for (const item of snapshot.interpretation?.items ?? []) {
    const card = document.createElement("article");
    const title = document.createElement("h4");
    title.textContent = item.title;
    card.className = "interpretation-card";
    card.append(title);
    for (const [label, values] of [
      ["Что обнаружено", item.observed],
      ["Что это значит", item.meaning],
      ["Что проверить", item.next_steps],
    ]) {
      const group = document.createElement("div");
      const heading = document.createElement("strong");
      const list = document.createElement("ul");
      heading.textContent = label;
      for (const value of values ?? []) {
        const row = document.createElement("li");
        row.textContent = value;
        list.append(row);
      }
      group.append(heading, list);
      card.append(group);
    }
    target.append(card);
  }
  const limitations = byId("io-wal-limitations-list");
  limitations.replaceChildren();
  for (const value of snapshot.interpretation?.limitations ?? []) {
    const row = document.createElement("li");
    row.textContent = value;
    limitations.append(row);
  }
  byId("io-wal-limitations").hidden = limitations.children.length === 0;
}

function renderIoWalSnapshot(result) {
  const snapshot = result.snapshot;
  ioWalCurrentSnapshot = snapshot;
  ioWalServerVersionNum = snapshot.overview.server_version_num;
  const db = snapshot.database_stats?.data;
  const wal = snapshot.wal?.data;
  const checkpoints = snapshot.checkpoints?.data;
  const sizes = snapshot.relation_sizes?.data;

  byId("io-wal-database").textContent = snapshot.overview.database;
  byId("io-wal-temp").textContent = db ? `${formatInteger(db.temp_files)} · ${formatBytes(db.temp_bytes)}` : "—";
  byId("io-wal-wal").textContent = wal ? formatBytes(wal.wal_bytes) : "—";
  byId("io-wal-checkpoints").textContent = checkpoints ? `${formatInteger(checkpoints.timed)} / ${formatInteger(checkpoints.requested)}` : "—";
  byId("io-wal-db-size").textContent = snapshot.database_size_bytes?.data === null || snapshot.database_size_bytes?.data === undefined ? "—" : formatBytes(snapshot.database_size_bytes.data);
  byId("io-wal-relations").textContent = sizes ? `${formatInteger(sizes.sampled_relation_count)} / ${formatInteger(sizes.total_relation_count)}` : "—";
  byId("io-wal-meta").textContent = `Snapshot #${result.history.id} · ${formatTimestamp(snapshot.overview.measured_at)} · PostgreSQL ${snapshot.overview.server_version}`;

  renderIoWalSectionError("io-wal-database-error", snapshot.database_stats);
  renderIoWalSectionError("io-wal-wal-error", snapshot.wal);
  renderIoWalSectionError("io-wal-checkpoint-error", snapshot.checkpoints);
  renderIoWalSectionError("io-wal-db-size-error", snapshot.database_size_bytes);
  renderIoWalSectionError("io-wal-size-error", snapshot.relation_sizes);

  byId("io-wal-db-stats-reset").textContent = db ? formatTimestamp(db.stats_reset) : "—";
  byId("io-wal-db-blocks").textContent = db ? `${formatInteger(db.blks_read)} / ${formatInteger(db.blks_hit)}` : "—";
  byId("io-wal-db-time").textContent = db ? `${formatNumber(db.blk_read_time_ms, 1)} / ${formatNumber(db.blk_write_time_ms, 1)} ms` : "—";
  byId("io-wal-track-timing").textContent = `${yesNo(snapshot.overview.track_io_timing)} / ${snapshot.overview.track_wal_io_timing === null ? "n/a" : yesNo(snapshot.overview.track_wal_io_timing)}`;
  byId("io-wal-wal-stats-reset").textContent = wal ? formatTimestamp(wal.stats_reset) : "—";
  byId("io-wal-wal-records").textContent = wal ? `${formatInteger(wal.wal_records)} · FPI ${formatInteger(wal.wal_fpi)} · full ${formatInteger(wal.wal_buffers_full)}` : "—";
  byId("io-wal-wal-time").textContent = wal ? `${formatNumber(wal.wal_write_time_ms, 1)} / ${formatNumber(wal.wal_sync_time_ms, 1)} ms` : "—";
  byId("io-wal-checkpoint-source").textContent = checkpoints?.source ?? "—";
  byId("io-wal-checkpoint-time").textContent = checkpoints ? `${formatNumber(checkpoints.write_time_ms, 1)} / ${formatNumber(checkpoints.sync_time_ms, 1)} ms` : "—";
  byId("io-wal-checkpoint-buffers").textContent = checkpoints ? formatInteger(checkpoints.buffers_written) : "—";
  byId("io-wal-checkpoint-config").textContent = `${snapshot.overview.checkpoint_timeout} / ${formatNumber(snapshot.overview.checkpoint_completion_target, 2)}`;
  byId("io-wal-wal-config").textContent = `${snapshot.overview.min_wal_size} / ${snapshot.overview.max_wal_size}`;

  const body = byId("io-wal-relation-list");
  body.replaceChildren();
  const relations = sizes?.relations ?? [];
  if (relations.length === 0) {
    tableEmpty(body, 6, "Relation sizes недоступны или user relations не найдены.");
  } else {
    for (const item of relations) {
      const row = document.createElement("tr");
      appendCells(row, [
        `${item.schema_name}.${item.relation_name}`,
        item.relation_type,
        formatBytes(item.heap_size_bytes),
        formatBytes(item.indexes_size_bytes),
        formatBytes(item.toast_size_bytes),
        formatBytes(item.total_size_bytes),
      ]);
      body.append(row);
    }
  }
  renderIoWalInterpretation(snapshot);
  byId("io-wal-empty").hidden = true;
  byId("io-wal-content").hidden = false;
}

function ioWalDocsVersion() {
  if (!ioWalServerVersionNum) return "current";
  const major = Math.floor(Number(ioWalServerVersionNum) / 10_000);
  return major >= 10 ? String(major) : "current";
}

function renderIoWalReference() {
  const methodology = byId("io-wal-methodology");
  const sources = byId("io-wal-sources");
  methodology.replaceChildren();
  sources.replaceChildren();
  for (const value of ioWalReference?.methodology ?? []) {
    const row = document.createElement("li"); row.textContent = value; methodology.append(row);
  }
  for (const source of ioWalReference?.sources ?? []) {
    const card = document.createElement("article");
    card.className = "source-card";
    const title = document.createElement("strong"); title.textContent = source.title;
    const purpose = document.createElement("p"); purpose.textContent = source.purpose;
    const url = document.createElement("code"); url.className = "source-url"; url.textContent = `https://www.postgresql.org/docs/${ioWalDocsVersion()}/${source.path}`;
    card.append(title, purpose, url); sources.append(card);
  }
}

async function loadIoWalReference() {
  if (!ioWalReferencePromise && !ioWalReference) {
    ioWalReferencePromise = invoke("io_wal_reference").then((value) => { ioWalReference = value; return value; }).finally(() => { ioWalReferencePromise = null; });
  }
  if (ioWalReferencePromise) await ioWalReferencePromise;
  renderIoWalReference();
}

function resetIoWalView() {
  ioWalCurrentSnapshot = null;
  ioWalServerVersionNum = null;
  byId("io-wal-empty").hidden = false;
  byId("io-wal-content").hidden = true;
}

function initIoWalUi() {
  byId("collect-io-wal").addEventListener("click", () => {
    const connectionId = Number(byId("io-wal-connection").value);
    if (!Number.isInteger(connectionId) || connectionId <= 0) {
      message("Сначала выберите подключение PostgreSQL.", true);
      return;
    }
    void action(async () => {
      const result = await invoke("collect_io_wal_snapshot", {connectionId});
      renderIoWalSnapshot(result);
      message(`I/O & WAL snapshot #${result.history.id} собран и сохранён.`);
    });
  });
}
