<script setup lang="ts">
import { t } from "../i18n/locale-service.js";
import {computed, onMounted, ref} from "vue";
import type {PgXrayInvoke} from "../../api/invoke.js";
import {formatBytes, formatInteger, formatSnapshotTime} from "../../core/formatters.js";
import {maintenanceActions, type MaintenanceAction} from "./storage-maintenance-ui.js";

interface Section {
  kind:string;count:number;json_bytes:number;last_week_bytes:number;
  oldest_at_ms:number|null;newest_at_ms:number|null;pinned_count:number;
}
interface StorageHealth {
  db_bytes:number;wal_bytes:number;shm_bytes:number;live_bytes_estimate:number;reusable_bytes:number;
  last_maintenance_ms:number|null;case_protected_connections:number;projected_days_to_budget:number|null;
  policy:{max_bytes:number|null;auto_prune:boolean;retention_days:number|null};sections:Section[];
}
interface Operation {action:string;deleted:number;detail:string}
type Feedback = {type:"success"|"error";message:string};
const props=defineProps<{invoke:PgXrayInvoke}>();
const health=ref<StorageHealth|null>(null);
const busy=ref(false);
const loadingAction=ref<string|null>(null);
const loadFeedback=ref<Feedback|null>(null);
const policyFeedback=ref<Feedback|null>(null);
const pinFeedback=ref<Feedback|null>(null);
const maintenanceFeedback=ref<Feedback|null>(null);
const pendingAction=ref<MaintenanceAction|null>(null);
const budgetMiB=ref("1024");
const budgetEnabled=ref(false);
const autoPrune=ref(false);
const protectedSection=ref("pgss");
const protectedSnapshot=ref("");
const labels:Record<string,string>={xid:"XID",pgss:"PGSS",autovacuum:"Autovacuum",pg_settings:"Settings",io_wal:"I/O & WAL",index_health:"Index Health"};
const physicalBytes=computed(()=>health.value?health.value.db_bytes+health.value.wal_bytes+health.value.shm_bytes:0);
const usedPercent=computed(()=>!health.value?.policy.max_bytes?null:
  Math.min(100,Math.round(health.value.live_bytes_estimate/health.value.policy.max_bytes*100)));
const actionTitle = (action: MaintenanceAction): string => ({
  check: t('storage.action_check'), analyze: t('storage.action_analyze'),
  checkpoint: t('storage.action_checkpoint'), vacuum: t('storage.action_vacuum'),
  retention: t('storage.action_retention'), prune: t('storage.action_prune'),
})[action];
const diagnosticActions:MaintenanceAction[]=["check","analyze","checkpoint"];
const cleanupActions:MaintenanceAction[]=["vacuum","retention","prune"];
function actionDisabled(action:MaintenanceAction):boolean {
  if(busy.value)return true;
  if(action==="retention"&&!health.value?.policy.retention_days)return true;
  if(action==="prune"&&!health.value?.policy.max_bytes)return true;
  return false;
}
async function load():Promise<void>{
  if(busy.value)return;
  busy.value=true;loadingAction.value="load";loadFeedback.value=null;
  try{
    const value=await props.invoke<StorageHealth>("history_storage_health");
    if(!value||!Array.isArray(value.sections)||!value.policy)throw Error(t('storage.invalid_sqlite_diagnostics_response'));
    health.value=value;
    budgetEnabled.value=value.policy.max_bytes!==null;
    budgetMiB.value=String(Math.round((value.policy.max_bytes??1024*1024*1024)/(1024*1024)));
    autoPrune.value=value.policy.auto_prune;
  }catch(e){loadFeedback.value={type:"error",message:t('storage.loadError', { error: String(e) })};}
  finally{busy.value=false;loadingAction.value=null;}
}
async function savePolicy():Promise<void>{
  if(busy.value)return;
  const mib=Number(budgetMiB.value);
  policyFeedback.value=null;
  if(budgetEnabled.value&&(!Number.isInteger(mib)||mib<100||mib>51200)){
    policyFeedback.value={type:"error",message:t('storage.budget_must_be_from_100_to_51200_mib')};return;
  }
  if(autoPrune.value&&!budgetEnabled.value&&!health.value?.policy.retention_days){
    policyFeedback.value={type:"error",message:t('storage.set_a_retention_period_below_or_enable_a_budget_for_')};return;
  }
  busy.value=true;loadingAction.value="policy";
  try{
    await props.invoke("update_history_storage_policy",{maxBytes:budgetEnabled.value?mib*1024*1024:null,autoPrune:autoPrune.value});
    policyFeedback.value={type:"success",message:t('storage.storage_policy_saved_automatic_cleanup_only_runs_whe')};
  }catch(e){policyFeedback.value={type:"error",message:t('storage.saveError', { error: String(e) })};}
  finally{busy.value=false;loadingAction.value=null;}
  if(policyFeedback.value?.type==="success")await load();
}
function requestMaintenance(action:MaintenanceAction):void{
  if(actionDisabled(action))return;
  maintenanceFeedback.value=null;
  if(maintenanceActions[action].destructive){pendingAction.value=action;return;}
  pendingAction.value=null;
  void maintain(action);
}
function confirmPending():void {
  const action=pendingAction.value;
  if(action)void maintain(action);
}
async function maintain(action:MaintenanceAction):Promise<void>{
  if(busy.value)return;
  pendingAction.value=null;
  busy.value=true;loadingAction.value=action;maintenanceFeedback.value=null;
  try{
    const result=await props.invoke<Operation>("maintain_history_storage",{action});
    if(!result||typeof result.detail!=="string"||!Number.isSafeInteger(result.deleted))
      throw Error(t('storage.invalid_sqlite_maintenance_response'));
    maintenanceFeedback.value={type:"success",message:(action === 'retention' || action === 'prune' ? t('storage.maintenanceDeleted', { action: actionTitle(action), detail: result.detail, deleted: String(result.deleted) }) : t('storage.maintenanceResult', { action: actionTitle(action), detail: result.detail }))};
  }catch(e){maintenanceFeedback.value={type:"error",message:`${actionTitle(action)}: ${String(e)}`};}
  finally{busy.value=false;loadingAction.value=null;}
  // Successful operations refresh the metrics, but never hide their local result.
  if(maintenanceFeedback.value?.type==="success")await load();
}
async function pin(protect:boolean):Promise<void>{
  if(busy.value)return;
  pinFeedback.value=null;
  const id=Number(protectedSnapshot.value);
  if(!Number.isSafeInteger(id)||id<=0){pinFeedback.value={type:"error",message:t('storage.enter_a_positive_snapshot_id_from_history')};return;}
  busy.value=true;loadingAction.value="pin";
  try{
    await props.invoke("protect_history_snapshot",{section:protectedSection.value,snapshotId:id,protect});
    pinFeedback.value={type:"success",message:protect?t('storage.pinned', { id: String(id) }):t('storage.unpinned', { id: String(id) })};
  }catch(e){pinFeedback.value={type:"error",message:t('storage.pinError', { error: String(e) })};}
  finally{busy.value=false;loadingAction.value=null;}
  if(pinFeedback.value?.type==="success")await load();
}
onMounted(()=>{void load();});
</script>

<template>
  <section class="storage-panel" :aria-label="t('storage.sqlite_history_status')">
    <header class="storage-heading">
      <div>
        <h3>{{ t('storage.sqlite_status') }}</h3>
        <p class="storage-muted">{{ t('storage.local_pgxray_history_is_not_the_server_postgresql_da') }}</p>
      </div>
      <button type="button" class="secondary" :disabled="busy" @click="load">
        {{loadingAction==='load'?t('storage.refreshing'):t('storage.refresh_status')}}
      </button>
    </header>
    <p v-if="loadFeedback" :class="['storage-message',loadFeedback.type]" :role="loadFeedback.type==='error'?'alert':'status'">{{loadFeedback.message}}</p>
    <template v-if="health">
      <div class="storage-metrics" :aria-label="t('storage.storage_metrics')">
        <div class="storage-metric"><span>{{ t('storage.on_disk_sqlite_wal_shm') }}</span><strong>{{formatBytes(physicalBytes)}}</strong><small>{{ t('storage.physical_size') }}</small></div>
        <div class="storage-metric"><span>{{ t('storage.allocated_pages') }}</span><strong>{{formatBytes(health.live_bytes_estimate)}}</strong><small>{{ t('storage.estimated_logical_size') }}</small></div>
        <div class="storage-metric"><span>{{ t('storage.free_within_file') }}</span><strong>{{formatBytes(health.reusable_bytes)}}</strong><small>{{ t('storage.reusable_space') }}</small></div>
        <div class="storage-metric"><span>{{ t('storage.connections_with_case') }}</span><strong>{{formatInteger(health.case_protected_connections)}}</strong><small>{{ t('storage.history_protected_from_automatic_cleanup') }}</small></div>
      </div>
      <p class="storage-muted storage-summary">
        {{ t('storage.time_to_budget') }} <strong>{{health.projected_days_to_budget===null?t('storage.insufficient_data'):t('storage.daysValue', { days: health.projected_days_to_budget.toFixed(1) })}}</strong>
        <span aria-hidden="true"> · </span>
        {{ t('storage.last_automatic_maintenance') }} <strong>{{health.last_maintenance_ms?formatSnapshotTime(health.last_maintenance_ms):t('storage.never')}}</strong>
      </p>
      <div v-if="usedPercent!==null" class="storage-meter">
        <div><span>{{ t('storage.used_space_against_budget') }}</span><strong>{{usedPercent}}%</strong></div>
        <progress :value="usedPercent" max="100" />
      </div>
      <section class="storage-section">
        <h4>{{ t('storage.snapshots_by_module') }}</h4>
        <div class="storage-table-scroll"><table class="storage-table">
          <thead><tr><th>{{ t('storage.module') }}</th><th>{{ t('storage.snapshots') }}</th><th>JSON</th><th>{{ t('storage.over_7_days') }}</th><th>{{ t('storage.pinnedCount') }}</th><th>{{ t('storage.period_2') }}</th></tr></thead>
          <tbody><tr v-for="s in health.sections" :key="s.kind">
            <th scope="row">{{labels[s.kind]??s.kind}}</th><td>{{formatInteger(s.count)}}</td>
            <td>{{formatBytes(s.json_bytes)}}</td><td>{{formatBytes(s.last_week_bytes)}}</td>
            <td>{{formatInteger(s.pinned_count)}}</td>
            <td class="storage-period">{{s.count?`${formatSnapshotTime(s.oldest_at_ms)} – ${formatSnapshotTime(s.newest_at_ms)}`:'—'}}</td>
          </tr></tbody>
        </table></div>
      </section>
      <section class="storage-section">
        <h4>{{ t('storage.storage_budget') }}</h4>
        <p class="storage-muted">{{ t('storage.the_budget_tracks_used_sqlite_space_not_the_immediat') }}</p>
        <div class="storage-policy-controls">
          <label class="storage-check"><input v-model="budgetEnabled" type="checkbox" :disabled="busy" /> {{ t('storage.set_budget') }}</label>
          <label v-if="budgetEnabled" class="storage-field">{{ t('storage.budget_mib') }} <input v-model="budgetMiB" type="number" min="100" max="51200" :disabled="busy" /></label>
          <label class="storage-check"><input v-model="autoPrune" type="checkbox" :disabled="busy" /> {{ t('storage.automatic_cleanup_at_most_once_daily') }}</label>
        </div>
        <p class="storage-muted">{{ t('storage.retention') }} {{health.policy.retention_days===null?t('storage.unlimited'):t('storage.daysRetention', { days: String(health.policy.retention_days) })}}. {{ t('storage.retentionHint') }}</p>
        <div class="storage-action-line"><button type="button" :disabled="busy" @click="savePolicy">{{loadingAction==='policy'?t('storage.saving'):t('storage.save_policy')}}</button></div>
        <p v-if="policyFeedback" :class="['storage-message',policyFeedback.type]" :role="policyFeedback.type==='error'?'alert':'status'">{{policyFeedback.message}}</p>
      </section>
      <section class="storage-section">
        <h4>{{ t('storage.protect_an_individual_snapshot') }}</h4>
        <p class="storage-muted">{{ t('storage.connections_with_an_investigation_case_already_have_') }}</p>
        <div class="storage-pin-controls">
          <label class="storage-field">{{ t('storage.module') }} <select v-model="protectedSection" :disabled="busy"><option v-for="(name,key) in labels" :key="key" :value="key">{{name}}</option></select></label>
          <label class="storage-field">{{ t('storage.snapshot_id') }} <input v-model="protectedSnapshot" type="number" min="1" :placeholder="t('storage.for_example_42')" :disabled="busy" /></label>
          <button type="button" class="secondary" :disabled="busy" @click="pin(true)">{{ t('storage.pin') }}</button>
          <button type="button" class="secondary" :disabled="busy" @click="pin(false)">{{ t('storage.unpin') }}</button>
        </div>
        <p v-if="pinFeedback" :class="['storage-message',pinFeedback.type]" :role="pinFeedback.type==='error'?'alert':'status'">{{pinFeedback.message}}</p>
      </section>
      <section class="storage-section storage-maintenance">
        <h4>{{ t('storage.sqlite_maintenance') }}</h4>
        <p class="storage-muted">{{ t('storage.diagnostics_and_maintenance_are_performed_locally_ea') }}</p>
        <div class="storage-operation-group">
          <h5>{{ t('storage.without_deleting_snapshots') }}</h5>
          <div class="storage-buttons"><button v-for="action in diagnosticActions" :key="action" type="button" class="secondary" :disabled="busy" @click="requestMaintenance(action)">
            {{loadingAction===action?t('storage.running'):actionTitle(action)}}
          </button></div>
          <p class="storage-muted">{{ t('storage.integrity_check_sqlite_statistics_and_non_blocking_p') }}</p>
        </div>
        <div class="storage-operation-group">
          <h5>{{ t('storage.cleanup_and_compaction') }}</h5>
          <div class="storage-buttons"><button v-for="action in cleanupActions" :key="action" type="button" class="secondary" :disabled="actionDisabled(action)" @click="requestMaintenance(action)">
            {{loadingAction===action?t('storage.running'):actionTitle(action)}}
          </button></div>
          <p class="storage-muted">{{ t('storage.retention_requires_a_retention_period_budget_cleanup') }}</p>
        </div>
        <div v-if="pendingAction" class="storage-confirm" role="group" :aria-label="t('storage.confirm_maintenance')">
          <strong>{{ t('storage.confirm') }} {{actionTitle(pendingAction)}}</strong>
          <p>{{pendingAction === 'vacuum' ? t('storage.confirmVacuum') : pendingAction === 'retention' ? t('storage.confirmRetention') : pendingAction === 'prune' ? t('storage.confirmPrune') : t('storage.confirmOther')}}</p>
          <div class="storage-buttons"><button type="button" :disabled="busy" @click="confirmPending">{{ t('storage.confirm_action') }}</button>
            <button type="button" class="secondary" :disabled="busy" @click="pendingAction=null">{{ t('storage.cancel') }}</button></div>
        </div>
        <p v-if="maintenanceFeedback" :class="['storage-message',maintenanceFeedback.type]" :role="maintenanceFeedback.type==='error'?'alert':'status'">{{maintenanceFeedback.message}}</p>
      </section>
    </template>
  </section>
</template>
<style scoped>
.storage-panel{display:grid;gap:1.3rem;min-width:0}
.storage-heading{display:flex;justify-content:space-between;align-items:start;flex-wrap:wrap;gap:1rem}
.storage-heading h3{margin:0 0 .4rem;font-size:1.15rem}
.storage-muted{font-size:.88rem;line-height:1.5;color:var(--text-secondary);margin:.3rem 0}
.storage-heading .storage-muted{max-width:72ch}
.storage-metrics{display:grid;grid-template-columns:repeat(auto-fit,minmax(190px,1fr));gap:.75rem}
.storage-metric{min-width:0;border:1px solid var(--border);border-radius:.65rem;padding:1rem;display:grid;gap:.35rem}
.storage-metric span{font-size:.82rem;color:var(--text-secondary)}
.storage-metric strong{font-size:1.45rem;font-variant-numeric:tabular-nums}
.storage-metric small{font-size:.75rem;color:var(--text-secondary)}
.storage-summary{margin:0}
.storage-summary strong{color:var(--text);font-weight:600}
.storage-meter{display:grid;gap:.4rem}
.storage-meter>div{display:flex;gap:1rem;justify-content:space-between;font-size:.85rem}
.storage-meter progress{width:100%;height:.65rem}
.storage-section{display:grid;gap:.8rem;border-top:1px solid var(--border);padding-top:1.25rem;min-width:0}
.storage-section h4{font-size:1.03rem;margin:0;font-weight:650}
.storage-table-scroll{width:100%;max-width:100%;overflow-x:auto}
.storage-table{width:100%;border-collapse:collapse;font-size:.88rem;min-width:700px}
.storage-table th,.storage-table td{text-align:left;vertical-align:middle;border-bottom:1px solid var(--border);padding:.7rem .6rem;white-space:nowrap}
.storage-table thead th{font-size:.77rem;font-weight:600;color:var(--text-secondary)}
.storage-table tbody th{font-weight:600}
.storage-period{color:var(--text-secondary)}
.storage-policy-controls,.storage-pin-controls{display:flex;flex-wrap:wrap;align-items:end;gap:.85rem 1.2rem}
.storage-check{display:flex;align-items:center;gap:.5rem;line-height:1.4;min-height:2.5rem}
.storage-field{display:grid;gap:.3rem;font-size:.85rem;min-width:150px}
.storage-field input,.storage-field select{box-sizing:border-box;width:100%;min-height:2.5rem;max-width:230px}
.storage-pin-controls button{min-height:2.5rem}
.storage-action-line{display:flex;flex-wrap:wrap;gap:.7rem}
.storage-operation-group{display:grid;gap:.5rem}
.storage-operation-group+.storage-operation-group{padding-top:.8rem;border-top:1px dashed var(--border)}
.storage-operation-group h5{font-size:.91rem;margin:0}
.storage-buttons{display:flex;flex-wrap:wrap;gap:.55rem}
.storage-buttons button{min-height:2.5rem}
.storage-message{margin:0;padding:.65rem .85rem;border:1px solid var(--border);border-radius:.5rem;line-height:1.45;font-size:.9rem;overflow-wrap:anywhere}
.storage-message.success{border-left:3px solid var(--success)}
.storage-message.error{color:var(--error);border-left:3px solid var(--error)}
.storage-confirm{padding:1rem;border:1px solid var(--border-strong);border-radius:.65rem;display:grid;gap:.6rem}
.storage-confirm p{font-size:.9rem;line-height:1.5;margin:0}
@media(max-width:720px){.storage-metrics{grid-template-columns:repeat(2,minmax(0,1fr))}.storage-metric{padding:.7rem}}
@media(max-width:440px){.storage-metrics{grid-template-columns:1fr}}
</style>
