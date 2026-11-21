import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from 'node:assert/strict';
import test from 'node:test';
import { xidShare, xidTopRelations, xidSnapshotFacts, xidKnownCount } from '../dist/features/xid/presentation.js';
const section = (data,error=null) => ({data,error});
const relation=(oid,xidAge,mxidAge,threshold=200)=>({relation_oid:oid,schema_name:'public',relation_name:`table_${oid}`,
  xid_age:xidAge,effective_freeze_max_age:threshold,mxid_age:mxidAge,effective_multixact_freeze_max_age:400});

test('DBA quicklook uses measured shares, not subjective risk classification',()=>{
  assert.equal(xidShare(80,200),40);
  assert.equal(xidShare(0,200),0);
  assert.equal(xidShare(3,0),null);
  assert.equal(xidShare(null,200),null);
  assert.equal(xidShare(-1,200),null);
});
test('DBA top relations ranks on independent XID / MXID freeze horizons',()=>{
  const relations=[relation(1,80,2),relation(2,20,390),relation(3,110,1)];
  assert.deepEqual(xidTopRelations(relations,'xid',2).map(r=>r.relation_oid),[3,1]);
  assert.deepEqual(xidTopRelations(relations,'mxid',2).map(r=>r.relation_oid),[2,1]);
  assert.deepEqual(relations.map(r=>r.relation_oid),[1,2,3]);
});
test('DBA quicklook preserves unavailable vs truly empty sections',()=>{
  const snapshot={overview:{database_xid_age:80,autovacuum_freeze_max_age:200,database_mxid_age:1,
    autovacuum_multixact_freeze_max_age:400},databases:section([]),relations:section([relation(1,80,1)]),
    autovacuum_triggers:section([]),vacuums:section(null,'permission denied'),backends:section([]),
    slots:section([{slot_name:'test'}]),prepared:section({count:0})};
  const facts=xidSnapshotFacts(snapshot);
  assert.equal(facts.activeVacuumCount,null);
  assert.equal(facts.backendCount,0);
  assert.equal(facts.slotCount,1);
  assert.equal(facts.preparedCount,0);
  assert.deepEqual(facts.missingSections,['VACUUM']);
  assert.equal(xidKnownCount(section(null,'permission denied')),null);
});
import { xidFilterRelations, xidTriggerState, xidTriggerStateLabel } from '../dist/features/xid/investigation.js';
test('Investigation filters respect XID/MXID independently and preserve original rows',()=>{
  const rows=[relation(1,180,1),relation(2,5,360),{...relation(3,190,370),schema_name:'pg_toast',relation_type:'t'}];
  const base={query:'',schemaScope:'all',kind:'all',horizon:'xid',minPct:75};
  assert.deepEqual(xidFilterRelations(rows,base).map(r=>r.relation_oid),[1,3]);
  assert.deepEqual(xidFilterRelations(rows,{...base,horizon:'mxid'}).map(r=>r.relation_oid),[2,3]);
  assert.deepEqual(xidFilterRelations(rows,{...base,horizon:'either',schemaScope:'user'}).map(r=>r.relation_oid),[1,2]);
  assert.deepEqual(xidFilterRelations(rows,{...base,kind:'toast'}).map(r=>r.relation_oid),[3]);
  assert.deepEqual(rows.map(r=>r.relation_oid),[1,2,3]);
});
test('Trigger status never equates unknown and disabled with threshold not reached',()=>{
  const trigger={autovacuum_enabled:true,dead_trigger_reached:false,insert_trigger_reached:false,
    estimated_dead_tuple_trigger:100,estimated_insert_trigger:200,thresholds_reliable:true};
  assert.equal(xidTriggerState(trigger),'below');
  assert.equal(xidTriggerState({...trigger,estimated_dead_tuple_trigger:null}),'unknown');
  assert.equal(xidTriggerState({...trigger,thresholds_reliable:false}),'unknown');
  assert.equal(xidTriggerState({...trigger,dead_trigger_reached:true}),'reached');
  assert.equal(xidTriggerState({...trigger,autovacuum_enabled:false}),'disabled');
  assert.match(xidTriggerStateLabel({...trigger,estimated_dead_tuple_trigger:null}),/не определён/);
});
import { xidHistoryScatter, xidFilterChanges, xidCsv, xidCsvCell } from '../dist/features/xid/history-tools.js';
test('XID history plot draws independent observations, even across possible restart',()=>{
  const history=[{id:3,collection_started_at_ms:3000,database_xid_age:90,oldest_relation:{xid_age:80}},
    {id:2,collection_started_at_ms:2000,database_xid_age:10,oldest_relation:null},
    {id:1,collection_started_at_ms:1000,database_xid_age:80,oldest_relation:{xid_age:65}}];
  const scatter=xidHistoryScatter(history);
  assert.deepEqual(scatter.points.map(point=>point.id),[1,2,3]);
  assert.equal(scatter.points[1].oldestY,null);
  assert.equal(scatter.minAge,10);assert.equal(scatter.maxAge,90);
  assert.equal(scatter.firstAt,1000);assert.equal(scatter.lastAt,3000);
});
test('XID comparison filters distinguish changed, increased and entering sample',()=>{
  const changes=[
    {relation_oid:1,sample_state:'present_in_both',xid_age_delta:0},
    {relation_oid:2,sample_state:'present_in_both',xid_age_delta:25},
    {relation_oid:3,sample_state:'present_in_both',xid_age_delta:-3},
    {relation_oid:4,sample_state:'entered_new_sample',xid_age_delta:null},
    {relation_oid:5,sample_state:'present_in_both',xid_age_delta:null},
  ];
  assert.deepEqual(xidFilterChanges(changes,'increased').map(r=>r.relation_oid),[2]);
  assert.deepEqual(xidFilterChanges(changes,'decreased').map(r=>r.relation_oid),[3]);
  assert.deepEqual(xidFilterChanges(changes,'sample').map(r=>r.relation_oid),[4]);
  assert.deepEqual(xidFilterChanges(changes,'changed').map(r=>r.relation_oid),[2,3,4]);
});
test('CSV exporter preserves columns and defangs spreadsheet formulas in relation names',()=>{
  assert.equal(xidCsvCell('=HYPERLINK("evil")'),`"'=HYPERLINK(""evil"")"`);
  assert.equal(xidCsvCell('   +SUM(1;2)'),`"'   +SUM(1;2)"`);
  assert.equal(xidCsvCell(-2),'-2');
  assert.equal(xidCsv(['name','age'],[['pg_catalog.tbl',40],['a;b',null]]),
    '"name";"age"\r\n"pg_catalog.tbl";40\r\n"a;b";""\r\n');
});
