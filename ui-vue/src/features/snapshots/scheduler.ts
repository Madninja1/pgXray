import { t } from "../i18n/locale-service.js";
import type { PgXrayInvoke } from "../../api/invoke.js";
import type { ShellState } from "../../core/shell-state.js";
import {
  CHECK_EVERY_MS, CONNECTION_RETRY_MS, KIND_LABELS, SLOT_GRACE_MS,
  SNAPSHOT_COMMANDS, inCaptureWindow, isIntervalSlot, isValidSavedSchedule,
  makeSchedule, newScheduleKey, nextDaily, nextIntervalSlot, scheduleDue,
  seriesExpired, seriesNextAt, snapshotTargetKey, type SavedTarget,
  type ScheduleDraft, type SnapshotKind, type SnapshotSchedule,
} from "./schedule-model.js";

const MAX_RULES = 60;
type StoredRule = SnapshotSchedule & { dbRevision: number };
export interface SnapshotSchedulerState {
  owner: string | null;
  connections: SavedTarget[];
  rules: StoredRule[];
  loading: boolean;
  collectingKey: string | null;
  error: string | null;
  notice: string | null;
}
export interface SnapshotSchedulerActions {
  syncAuth(): Promise<void>;
  refreshConnections(): Promise<void>;
  refreshRules(): Promise<void>;
  saveRule(draft: ScheduleDraft, editingKey?: string | null): Promise<boolean>;
  removeRule(key: string): Promise<boolean>;
  setEnabled(key: string, enabled: boolean): Promise<boolean>;
  check(): Promise<void>;
  start(): void;
  stop(): void;
}
export function createSnapshotSchedulerState(): SnapshotSchedulerState {
  return {owner:null,connections:[],rules:[],loading:false,collectingKey:null,error:null,notice:null};
}
export function parseScheduleConnections(raw: unknown): SavedTarget[] {
  if (!Array.isArray(raw)) throw new Error(t('tsScheduler.badConnections'));
  const seen = new Set<number>();
  return raw.map((value): SavedTarget => {
    if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(t('tsScheduler.badConnection'));
    const r = value as Record<string, unknown>;
    if (!Number.isSafeInteger(r.id) || (r.id as number) <= 0 || seen.has(r.id as number) ||
        !Number.isSafeInteger(r.port) || (r.port as number) < 1 || (r.port as number) > 65535 ||
        [r.name, r.host, r.database, r.username, r.sslmode].some(v => typeof v !== "string"))
      throw new Error(t('tsScheduler.badFields'));
    seen.add(r.id as number);
    return {id:r.id as number,port:r.port as number,name:r.name as string,host:r.host as string,
      database:r.database as string,username:r.username as string,sslmode:r.sslmode as string};
  }).sort((a,b)=>a.name.localeCompare(b.name,"ru"));
}
function parseRules(raw: unknown): StoredRule[] {
  if (!Array.isArray(raw) || raw.length > MAX_RULES) throw new Error(t('tsScheduler.badSqliteResponse'));
  const rows = raw as unknown[];
  if (rows.some(value => !isValidSavedSchedule(value) ||
      !Number.isSafeInteger((value as StoredRule).dbRevision) || (value as StoredRule).dbRevision < 1))
    throw new Error(t('tsScheduler.corruptSqlite'));
  const rules = rows as StoredRule[];
  if (new Set(rules.map(r => r.key)).size !== rules.length) throw new Error(t('tsScheduler.duplicateSqliteKey'));
  return rules;
}
export function createSnapshotScheduler(shell: ShellState, state: SnapshotSchedulerState,
  invoke: PgXrayInvoke | null, now: () => number = Date.now): SnapshotSchedulerActions {
  let timer: ReturnType<typeof setInterval> | null = null;
  let maintenanceTimer: ReturnType<typeof setInterval> | null = null;
  let revision = 0;
  let checking = false;
  let mutating = false;
  const active = () => shell.screen === "workspace" && shell.currentUsername !== null && !!invoke;
  const valid = (rev: number, owner: string, auth: number) =>
    active() && revision === rev && shell.currentUsername === owner && shell.authGeneration === auth;

  async function refreshRules(): Promise<void> {
    if (!active() || !invoke || !state.owner) return;
    const rev=revision, owner=state.owner, auth=shell.authGeneration;
    try {
      const rules=parseRules(await invoke("list_snapshot_schedules"));
      if (valid(rev,owner,auth)) state.rules=rules;
    } catch (error) {
      if (valid(rev,owner,auth)) { state.rules=[]; state.error=t('tsScheduler.sqliteError', { error: String(error) }); }
      throw error;
    }
  }
  async function save(rule: SnapshotSchedule, expected: number | null): Promise<StoredRule> {
    if (!invoke) throw Error(t('tsScheduler.noBackend'));
    const stored=parseRules([await invoke("put_snapshot_schedule", {rule,expectedRevision:expected})])[0];
    if (state.owner && active()) state.rules=[...state.rules.filter(r=>r.key!==stored.key),stored]
      .sort((a,b)=>a.key.localeCompare(b.key));
    return stored;
  }
  async function change(rule: StoredRule): Promise<boolean> {
    try { await save(rule,rule.dbRevision);return true; }
    catch (error) {state.error=String(error);await refreshRules().catch(()=>{});return false;}
  }
  async function syncAuth(): Promise<void> {
    if (!active()) {
      revision++;state.owner=null;state.connections=[];state.rules=[];state.collectingKey=null;
      state.loading=false;state.error=null;state.notice=null;return;
    }
    const owner=shell.currentUsername!;
    if (state.owner===owner) return;
    revision++;state.owner=owner;state.rules=[];state.connections=[];
    state.error=null;state.notice=null;state.collectingKey=null;
    try {await refreshRules();await refreshConnections();await check();}
    catch { /* fail closed: refreshRules has cleared rules */ }
  }
  async function refreshConnections(): Promise<void> {
    if (!active() || !invoke || !state.owner) return;
    const rev=revision, owner=state.owner, auth=shell.authGeneration;
    state.loading=true;
    try {
      const connections=parseScheduleConnections(await invoke("list_connections"));
      if (valid(rev,owner,auth)) {state.connections=connections;
        if ((state.error?.startsWith("Подключения:") || state.error?.startsWith("Connections:"))) state.error=null;}
    } catch(error) {if(valid(rev,owner,auth)) state.error=t('tsScheduler.connectionsError', { error: String(error) });}
    finally {if(valid(rev,owner,auth)) state.loading=false;}
  }
  async function saveRule(draft: ScheduleDraft, editingKey: string|null=null): Promise<boolean> {
    if (!active() || !state.owner || mutating) return false;
    mutating=true;state.error=null;state.notice=null;
    try {
      await refreshRules();
      const target=state.connections.find(c=>c.id===draft.connectionId);
      if(!target) throw Error(t('tsScheduler.targetMissing'));
      const original=editingKey===null?null:state.rules.find(r=>r.key===editingKey);
      if(editingKey!==null&&!original) throw Error(t('tsScheduler.editedElsewhere'));
      if(original && (original.connectionId!==draft.connectionId||original.kind!==draft.kind))
        throw Error(t('tsScheduler.differentTarget'));
      const duplicate=state.rules.some(r=>r.key!==editingKey&&r.enabled&&r.connectionId===draft.connectionId&&
        r.kind===draft.kind&&r.mode===draft.mode&& (draft.mode==="interval" ?
          r.intervalMinutes===draft.intervalMinutes&&r.windowStart===draft.windowStart&&
          r.windowEnd===draft.windowEnd&&r.intervalAnchor===draft.intervalAnchor&&
          [...r.daysOfWeek].sort().join()===[...draft.daysOfWeek].sort().join() : draft.mode==="daily" ?
          r.dailyTime===draft.dailyTime&&[...r.daysOfWeek].sort().join()===[...draft.daysOfWeek].sort().join() :
          r.seriesCount===draft.seriesCount&&r.seriesSpanMinutes===draft.seriesSpanMinutes));
      if(duplicate) throw Error(t('tsScheduler.duplicateActive'));
      if(!original&&state.rules.length>=MAX_RULES) throw Error(t('tsScheduler.limit60'));
      const rule=makeSchedule(draft,target,now());
      rule.key=original?.key??newScheduleKey(draft.connectionId,draft.kind,rule.createdAt,new Set(state.rules.map(r=>r.key)));
      if(original) rule.createdAt=original.createdAt;
      await save(rule,original?.dbRevision??null);
      state.notice=original?t('tsScheduler.updated', { kind: KIND_LABELS[draft.kind] }):t('tsScheduler.added', { kind: KIND_LABELS[draft.kind] });
      void check();return true;
    }catch(error){state.error=String(error);await refreshRules().catch(()=>{});return false;}
    finally{mutating=false;}
  }
  async function removeRule(key:string):Promise<boolean>{
    if(!active()||!invoke||mutating)return false;
    mutating=true;
    try{
      const rule=state.rules.find(r=>r.key===key);if(!rule)throw Error(t('tsScheduler.missingRule'));
      await invoke("remove_snapshot_schedule",{key,expectedRevision:rule.dbRevision});
      state.rules=state.rules.filter(r=>r.key!==key);state.notice=t('tsScheduler.removed');return true;
    }catch(error){state.error=String(error);await refreshRules().catch(()=>{});return false;}
    finally{mutating=false;}
  }
  async function setEnabled(key:string,enabled:boolean):Promise<boolean>{
    if(!active()||mutating)return false;
    mutating=true;
    try{
      const rule=state.rules.find(r=>r.key===key);
      if(!rule||(enabled&&rule.status!=="active"&&rule.status!=="paused"))return false;
      const next:StoredRule={...rule,enabled,status:enabled?"active":"paused"};
      if(enabled)next.nextAt=rule.mode==="daily"?nextDaily(now(),rule.dailyTime,rule.daysOfWeek):
        rule.mode==="interval"?nextIntervalSlot(now(),rule,true):now();
      return await change(next);
    }finally{mutating=false;}
  }
  async function check():Promise<void>{
    if(checking||mutating||!active()||!invoke||!state.owner||state.loading)return;
    checking=true;
    const owner=state.owner, rev=revision,auth=shell.authGeneration;
    try{
      // SQLite is authoritative even if another pgXray process edited the rules.
      await refreshRules();
      if(!valid(rev,owner,auth))return;
      const timestamp=now();
      for(const rule of state.rules){
        if(!rule.enabled||rule.status!=="active")continue;
        // A claim persisted before a crash must not be silently retried.
        // Allow active collectors in another process 30 minutes to finish first.
        if(rule.lastAttemptAt!==null&&rule.lastError===null&&
            (rule.lastSuccessAt===null||rule.lastSuccessAt<rule.lastAttemptAt)&&
            timestamp-rule.lastAttemptAt>30*60_000){
          await change({...rule,enabled:false,status:"paused",lastError:"Предыдущий сбор не подтверждён. Проверь History и возобнови вручную."});return;
        }
        if(rule.mode==="daily"&&timestamp>rule.nextAt+10*60_000){
          await change({...rule,nextAt:nextDaily(timestamp,rule.dailyTime,rule.daysOfWeek)});return;
        }
        if(seriesExpired(rule,timestamp)){
          await change({...rule,enabled:false,status:"expired",lastError:"Серия не завершена в отведённое время."});return;
        }
        if(rule.mode==="interval"&&(!isIntervalSlot(rule,rule.nextAt)||timestamp>rule.nextAt+SLOT_GRACE_MS||
           (timestamp>=rule.nextAt&&!inCaptureWindow(timestamp,rule.windowStart,rule.windowEnd)))){
          await change({...rule,nextAt:nextIntervalSlot(timestamp,rule,true)});continue;
        }
      }
      const due=[...state.rules].filter(r=>scheduleDue(r,timestamp)).sort((a,b)=>a.nextAt-b.nextAt)[0];
      if(!due||!valid(rev,owner,auth))return;
      const connections=parseScheduleConnections(await invoke("list_connections"));
      if(!valid(rev,owner,auth))return;
      state.connections=connections;
      const current=connections.find(c=>c.id===due.connectionId);
      if(!current||snapshotTargetKey(current)!==due.fingerprint){
        await change({...due,enabled:false,status:"paused",lastError:"Профиль подключения удалён или изменился."});return;
      }
      const started=now();
      if(due.mode==="interval"&&(started>due.nextAt+SLOT_GRACE_MS||
          !inCaptureWindow(started,due.windowStart,due.windowEnd))){
        await change({...due,nextAt:nextIntervalSlot(started,due,true)});return;
      }
      const reserved:StoredRule={...due,lastAttemptAt:started,lastError:null,
        nextAt:due.mode==="interval"?nextIntervalSlot(due.nextAt,due):
          due.mode==="daily"?nextDaily(started+60_000,due.dailyTime,due.daysOfWeek):
          Math.max(started+CONNECTION_RETRY_MS,seriesNextAt(due))};
      // Atomic SQLite reservation prevents the same slot from running in two processes.
      const claimed=await invoke<StoredRule|null>("claim_snapshot_schedule",{
        rule:reserved,expectedRevision:due.dbRevision,slotAtMs:due.nextAt,claimedAtMs:started,
      });
      if(!valid(rev,owner,auth))return;
      if(!claimed){
        await refreshRules();
        const existing=state.rules.find(r=>r.key===due.key);
        if(existing&&existing.nextAt===due.nextAt&&existing.enabled){
          const next=existing.mode==="interval"?nextIntervalSlot(due.nextAt,existing):
            existing.mode==="daily"?nextDaily(timestamp+60_000,existing.dailyTime,existing.daysOfWeek):0;
          if(existing.mode==="series")await change({...existing,enabled:false,status:"expired",lastError:"Совпадающий слот: серия не завершена."});
          else await change({...existing,nextAt:next,lastError:"Совпадающий слот другого расписания: повторный сбор пропущен."});
        }
        return;
      }
      state.rules=state.rules.map(r=>r.key===due.key?claimed:r);
      state.collectingKey=due.key;
      try{
        await invoke(SNAPSHOT_COMMANDS[due.kind],{connectionId:due.connectionId});
        if(!valid(rev,owner,auth))return;
        const ended=now();
        const completed:StoredRule={...claimed,lastSuccessAt:ended,
          lastDurationMs:Math.max(0,ended-started),lastError:null};
        if(due.mode==="series"){
          completed.seriesCompleted=due.seriesCompleted+1;
          if(completed.seriesCompleted>=due.seriesCount){completed.enabled=false;completed.status="complete";completed.nextAt=0;}
          else completed.nextAt=seriesNextAt(completed);
        }
        await change(completed);
      }catch(error){
        if(valid(rev,owner,auth)){
          await change({...claimed,lastError:String(error).slice(0,1000),
            lastDurationMs:Math.max(0,now()-started),
            nextAt:due.mode==="series"?now()+CONNECTION_RETRY_MS:claimed.nextAt});
        }
      }finally{if(valid(rev,owner,auth))state.collectingKey=null;}
    }catch(error){if(valid(rev,owner,auth))state.error=t('tsScheduler.collectionError', { error: String(error) });}
    finally{checking=false;}
  }
  function start():void{
    if(timer!==null)return;
    timer=setInterval(()=>{void check();},CHECK_EVERY_MS);
    maintenanceTimer=setInterval(()=>{if(active()&&invoke)void invoke("history_maintenance_tick").catch(()=>{});},60*60_000);
    void syncAuth().then(()=>{if(active()&&invoke)void invoke("history_maintenance_tick").catch(()=>{});});
  }
  function stop():void{
    if(timer!==null)clearInterval(timer);timer=null;
    if(maintenanceTimer!==null)clearInterval(maintenanceTimer);maintenanceTimer=null;
    revision++;
  }
  return{syncAuth,refreshConnections,refreshRules,saveRule,removeRule,setEnabled,check,start,stop};
}
