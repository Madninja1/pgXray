"use strict";

function renderIndexHealthConnectionOptions(connections) {
  const select = byId("index-health-connection");
  const previous = select.value;
  select.replaceChildren();
  if (connections.length === 0) {
    const option = document.createElement("option"); option.value = ""; option.textContent = "Нет сохранённых подключений";
    select.append(option); select.disabled = true; byId("collect-index-health").disabled = true; return;
  }
  select.disabled = false; byId("collect-index-health").disabled = false;
  for (const connection of connections) {
    const option = document.createElement("option"); option.value = String(connection.id);
    option.textContent = `${connection.name} — ${connection.username}@${connection.host}:${connection.port}/${connection.database}`;
    select.append(option);
  }
  if (previous && connections.some((connection) => String(connection.id) === previous)) select.value = previous;
}

function indexHealthLabel(value) {
  const labels = {
    actively_used: "actively used",
    low_evidence: "low evidence",
    suspiciously_unused: "возможно не используется (не DROP)",
    possible_duplicate: "возможный структурный дубликат (не DROP)",
    high_write_overhead: "возможные расходы на запись (косвенная оценка)",
    needs_investigation: "needs investigation",
  };
  return labels[value] ?? value;
}

function indexHealthVisible(row) {
  const text = byId("index-health-filter").value.trim().toLowerCase();
  const mode = byId("index-health-mode").value;
  if (text && !`${row.schema_name}.${row.relation_name} ${row.index_name} ${row.index_oid}`.toLowerCase().includes(text)) return false;
  if (mode === "all") return true;
  return row.classifications?.includes(mode) ?? false;
}

function renderIndexHealthRows() {
  const body = byId("index-health-list");
  body.replaceChildren();
  const rows = (indexHealthCurrentSnapshot?.collection?.indexes ?? []).filter(indexHealthVisible);
  if (rows.length === 0) { tableEmpty(body, 12, "Indexes для текущего фильтра не найдены."); return; }
  for (const item of rows) {
    const row = document.createElement("tr");
    const classification = document.createElement("td");
    classification.textContent = (item.classifications ?? []).map(indexHealthLabel).join(", ");
    const action = document.createElement("td");
    const button = document.createElement("button");
    button.type = "button"; button.className = "secondary index-health-table-button"; button.textContent = "Table Health";
    button.dataset.indexRelationOid = String(item.relation_oid);
    action.append(button);
    appendCells(row, [
      `${item.schema_name}.${item.relation_name}`,
      item.index_name,
      formatBytes(item.index_size_bytes),
      formatInteger(item.idx_scan),
      `${formatInteger(item.idx_tup_read)} / ${formatInteger(item.idx_tup_fetch)}`,
      `${item.is_primary ? "PK " : ""}${item.is_unique ? "UNIQUE " : ""}${item.is_valid ? "valid" : "INVALID"}${item.is_ready ? "" : " / not ready"}`,
      `${(item.key_columns ?? []).join(", ")}${item.predicate ? ` WHERE ${item.predicate}` : ""}`,
      `${formatInteger(item.tuples_inserted)} / ${formatInteger(item.tuples_updated)} / ${formatInteger(item.tuples_deleted)}`,
      item.tuples_updated > 0 ? formatPercent(item.hot_updates / item.tuples_updated * 100) : "—",
      `${formatTimestamp(item.last_analyze)} / ${formatTimestamp(item.last_autoanalyze)}`,
    ]);
    row.append(classification, action);
    body.append(row);
  }
}

function renderIndexHealthSnapshot(result) {
  indexHealthCurrentSnapshot = result.snapshot;
  indexHealthCurrentConnectionId = result.history.connection_id;
  indexHealthServerVersionNum = result.snapshot.overview.server_version_num;
  const indexes = result.snapshot.collection.indexes ?? [];
  const count = (label) => indexes.filter((row) => row.classifications?.includes(label)).length;
  byId("index-health-indexes").textContent = `${formatInteger(result.snapshot.collection.sampled_index_count)} / ${formatInteger(result.snapshot.collection.total_index_count)}`;
  byId("index-health-used").textContent = formatInteger(count("actively_used"));
  byId("index-health-unused").textContent = formatInteger(count("suspiciously_unused"));
  byId("index-health-duplicates").textContent = formatInteger(count("possible_duplicate"));
  byId("index-health-write-overhead").textContent = formatInteger(count("high_write_overhead"));
  byId("index-health-stats-reset").textContent = formatTimestamp(result.snapshot.overview.stats_reset);
  byId("index-health-meta").textContent = `Snapshot #${result.history.id} · ${formatTimestamp(result.snapshot.overview.measured_at)} · PostgreSQL ${result.snapshot.overview.server_version}`;
  const limitations = byId("index-health-limitations-list"); limitations.replaceChildren();
  for (const value of result.snapshot.interpretation?.limitations ?? []) { const li=document.createElement("li"); li.textContent=value; limitations.append(li); }
  byId("index-health-limitations").hidden = limitations.children.length === 0;
  renderIndexHealthRows();
  byId("index-health-empty").hidden = true; byId("index-health-content").hidden = false;
}

function indexHealthDocsVersion() { if (!indexHealthServerVersionNum) return "current"; const major=Math.floor(Number(indexHealthServerVersionNum)/10000); return major>=10?String(major):"current"; }
function renderIndexHealthReference() {
  const method=byId("index-health-methodology"); const sources=byId("index-health-sources"); method.replaceChildren(); sources.replaceChildren();
  for (const value of indexHealthReference?.methodology ?? []) { const li=document.createElement("li"); li.textContent=value; method.append(li); }
  for (const source of indexHealthReference?.sources ?? []) { const card=document.createElement("article"); card.className="source-card"; const strong=document.createElement("strong"); strong.textContent=source.title; const p=document.createElement("p"); p.textContent=source.purpose; const code=document.createElement("code"); code.className="source-url"; code.textContent=`https://www.postgresql.org/docs/${indexHealthDocsVersion()}/${source.path}`; card.append(strong,p,code); sources.append(card); }
}
async function loadIndexHealthReference() { if(!indexHealthReferencePromise&&!indexHealthReference){indexHealthReferencePromise=invoke("index_health_reference").then(v=>{indexHealthReference=v;return v;}).finally(()=>{indexHealthReferencePromise=null;});} if(indexHealthReferencePromise)await indexHealthReferencePromise; renderIndexHealthReference(); }
function resetIndexHealthView(){indexHealthCurrentSnapshot=null;indexHealthCurrentConnectionId=null;indexHealthServerVersionNum=null;byId("index-health-empty").hidden=false;byId("index-health-content").hidden=true;}
function initIndexHealthUi(){
  byId("collect-index-health").addEventListener("click",()=>{const connectionId=Number(byId("index-health-connection").value);if(!Number.isInteger(connectionId)||connectionId<=0){message("Сначала выберите подключение PostgreSQL.",true);return;}void action(async()=>{const result=await invoke("collect_index_health_snapshot",{connectionId});renderIndexHealthSnapshot(result);message(`Index Health snapshot #${result.history.id} собран и сохранён.`);});});
  byId("index-health-filter").addEventListener("input",renderIndexHealthRows); byId("index-health-mode").addEventListener("change",renderIndexHealthRows);
  byId("index-health-list").addEventListener("click",(event)=>{const button=event.target.closest("button[data-index-relation-oid]");if(!button)return;const oid=Number(button.dataset.indexRelationOid);if(!Number.isInteger(oid)||oid<=0)return;void action(async()=>{setDiagnosticModule("autovacuum");byId("autovacuum-connection").value=String(indexHealthCurrentConnectionId??"");await loadAutovacuumTableDetail(oid);message("Открыт Table Health из Index Health.");});});
}
