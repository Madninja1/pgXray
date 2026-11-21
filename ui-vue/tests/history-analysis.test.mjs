import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import test from 'node:test';
import assert from 'node:assert/strict';
import {xidTrendPoints,xidRelations,xidAgeCohorts,xidTrendSegments,formatPgssDuration,formatPgssExecRate,rankPgss,pgssTrendSegments} from '../dist/features/history/analysis-model.js';
const row=(id,t,age)=>({id,collection_started_at_ms:t,database_xid_age:age});
test('XID chart respects unequal observation intervals and freeze max scale',()=>{
 const c=xidTrendPoints([row(1,1000,50),row(2,2000,60),row(3,9000,70)],100);
 assert.ok(c.points[1].x-c.points[0].x<c.points[2].x-c.points[1].x);
 assert.ok(c.thresholdY!==null && c.thresholdY<c.points[2].y);
});
test('OID reuse does not create false freeze event',()=>{
 const rel=(name,age)=>({relation_oid:1,schema_name:'public',relation_name:name,relation_type:'table',xid_age:age,pct_of_freeze_max:20,effective_freeze_max_age:200,mxid_age:null,pct_of_multixact_freeze_max:null,total_size_bytes:100,last_autovacuum:null,last_vacuum:null});
 const curr={overview:{},relations:{data:[rel('new',20)]}}, prev={overview:{},relations:{data:[rel('old',100)]}};
 assert.equal(xidRelations(curr,prev)[0].age_delta,null);
 assert.equal(xidRelations(curr,prev)[0].freeze_observed,false);
});
test('PGSS regression ranking excludes missing baseline',()=>{
 const m=(exec,mean)=>({calls_per_hour:1,exec_ms_per_hour:exec,mean_exec_ms:mean,shared_reads_per_hour:0,temp_blocks_per_hour:0,wal_bytes_per_hour:0});
 const q=(id,latest,previous)=>({queryid:id,userid:1,dbid:1,toplevel:true,role_name:null,database:null,latest,previous});
 assert.deepEqual(rankPgss([q('a',m(100,100),null),q('b',m(30,30),m(10,10))], 'latency').map(x=>x.queryid),['b']);
});
test('PGSS gaps must split trend instead of joining rates across reset',()=>{
 const c=(id,state,v)=>({to_snapshot_id:id,to_ms:id*1000,state,global_calls_per_hour:v});
 const seg=pgssTrendSegments([c(1,'comparable',10),c(2,'dealloc_changed',null),c(3,'comparable',15)],'global_calls_per_hour');
 assert.equal(seg.length,2);
});

test('XID trend never connects snapshots of different targets under reused connection ID',()=>{
  const same={connection_id:7,host:'host',port:5432,database_oid:9,username:'postgres',sslmode:'disable'};
  const c=xidTrendPoints([ {...row(1,1000,50),...same,database_oid:10}, {...row(2,2000,60),...same}, {...row(3,3000,70),...same} ],100);
  assert.deepEqual(c.points.map(x=>x.id),[2,3]);
});

test('XID relations with identical freeze horizon are presented as a cohort without losing members',()=>{
 const relation=(oid,age,mxid,scheme='pg_catalog')=>({relation_oid:oid,schema_name:scheme,relation_name:`r${oid}`,relation_type:'r',xid_age:age,
   pct_of_freeze_max:age/2000,effective_freeze_max_age:200000,mxid_age:mxid,pct_of_multixact_freeze_max:mxid/200,
   age_delta:50,freeze_observed:false,total_size_bytes:100,last_autovacuum:null,last_vacuum:null});
 const curr={overview:{},relations:{data:[relation(1,80000,100),relation(2,80000,100),relation(3,80000,150),relation(4,80000,100,'public')]}};
 const cohorts=xidAgeCohorts(xidRelations(curr));
 assert.equal(cohorts.length,3);
 assert.equal(cohorts.reduce((n,g)=>n+g.members.length,0),4);
 assert.equal(cohorts.find(g=>g.members.length===2).system,true);
});
test('XID time-series does not connect a two-day observation gap',()=>{
 const points=xidTrendPoints([row(1,0,50),row(2,600000,52),row(3,1200000,54),row(4,173000000,65)],100).points;
 const segments=xidTrendSegments(points);
 assert.equal(segments.length,2);
 assert.deepEqual(segments.map(s=>s.length),[3,1]);
});
test('PGSS execution time uses meaningful units without implying CPU time',()=>{
 assert.equal(formatPgssDuration(94396.32),'94,4 с');
 assert.equal(formatPgssExecRate(3644984.48),'60,75 мин/ч');
 assert.equal(formatPgssExecRate(null),'—');
});

test('PGSS history graph does not imply continuity across a two-day gap', () => {
 const row=(id,ms)=>({to_snapshot_id:id,to_ms:ms,state:'comparable',global_calls_per_hour:25});
 const segments=pgssTrendSegments([row(2,3600000),row(3,7200000),row(4,2*86400000)],'global_calls_per_hour');
 assert.deepEqual(segments.map(part=>part.map(point=>point.id)),[[2,3],[4]]);
});
test('PGSS ranking never treats an unsampled previous query as a regression', () => {
 const metrics=(execMs,latency)=>({calls_per_hour:2,exec_ms_per_hour:execMs,mean_exec_ms:latency,shared_reads_per_hour:0,temp_blocks_per_hour:0,wal_bytes_per_hour:0});
 const row=(id,latest,previous)=>({userid:1,dbid:1,queryid:id,toplevel:true,role_name:'app',database:'db',latest,previous});
 assert.deepEqual(rankPgss([row('no-baseline',metrics(900,200),null),row('regression',metrics(40,20),metrics(10,5))],'impact').map(item=>item.queryid),['regression']);
});
