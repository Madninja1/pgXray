import test from 'node:test';
import assert from 'node:assert/strict';
import {xidHistoryTimeline,xidHistoryChart,xidChartSegments,xidRelationTransitions} from '../dist/features/history/xid-insights.js';
const target={connection_id:2,host:'db1',port:5432,database:'postgres',database_oid:5,username:'postgres',sslmode:'disable'};
const point=(id,t,xid,mxid=25,options={})=>{const {summary:summaryOverrides={},...rest}=options;return {summary:{...target,id,collection_started_at_ms:t,current_xid:String(xid),database_xid_age:100,...summaryOverrides},
  database_mxid_age:mxid,autovacuum_freeze_max_age:200,autovacuum_multixact_freeze_max_age:400,
  postmaster_start_time:'2026-11-01T10:00:00Z',in_recovery:false,...rest};};

test('XID/MXID history uses comparable snapshots and shows both horizon movements',()=>{
 const a=point(1,0,100,50,{summary:{database_xid_age:70}});
 const b=point(2,60_000,180,49,{summary:{database_xid_age:65}});
 const timeline=xidHistoryTimeline([b,a]);
 assert.equal(timeline.intervals.length,1);
 assert.equal(timeline.intervals[0].state,'observed');
 assert.equal(timeline.intervals[0].xid_age_delta,-5);
 assert.equal(timeline.intervals[0].mxid_age_delta,-1);
 assert.equal(timeline.intervals[0].horizon_advanced,true);
 assert.equal(timeline.intervals[0].mxid_horizon_advanced,true);
 assert.equal(timeline.intervals[0].consumed_xids,80n);
 assert.equal(timeline.intervals[0].xid_per_hour,4800);
 const chart=xidHistoryChart(timeline,'mxid');
 assert.equal(chart.points.length,2);
 assert.equal(xidChartSegments(chart).length,1);
});

test('XID does not interpolate across restart, rollback, missing metadata or long gap',()=>{
 const pts=[point(1,0,100),point(2,60_000,110,25,{postmaster_start_time:'new'}),point(3,120_000,95,25,{postmaster_start_time:'new'}),
  point(4,180_000,100,25,{postmaster_start_time:null}),point(5,7*3_600_000,110,25,{postmaster_start_time:null})];
 const timeline=xidHistoryTimeline(pts);
 assert.deepEqual(timeline.intervals.map(i=>i.state),['restart','xid_rewind','unknown_continuity','gap']);
 assert.ok(timeline.intervals.every(i=>i.horizon_advanced===false));
 assert.equal(xidChartSegments(xidHistoryChart(timeline,'xid')).length,5);
});

test('Reusing a connection ID for a different target excludes old target history',()=>{
 const timeline=xidHistoryTimeline([point(1,0,100),point(2,1000,200,25,{summary:{host:'another'}}),point(3,2000,300,25,{summary:{host:'another'}})]);
 assert.deepEqual(timeline.points.map(p=>p.summary.id),[2,3]);
 assert.equal(timeline.ignoredTargets,1);
});

test('Charts do not connect over missing MXID samples or fabricate zeros',()=>{
 const timeline=xidHistoryTimeline([point(1,0,100,25),point(2,1000,150,null),point(3,2000,160,30)]);
 const mxid=xidHistoryChart(timeline,'mxid');
 assert.equal(mxid.points.length,2);
 assert.equal(mxid.points[1].connectToPrevious,false);
 assert.equal(xidChartSegments(mxid).length,2);
});

test('Relation history will not claim freeze across restart or OID identity change',()=>{
 const rel=(name,age,mxid=20)=>({schema_name:'public',relation_name:name,relation_type:'r',xid_age:age,pct_of_freeze_max:40,mxid_age:mxid,
   pct_of_multixact_freeze_max:5,last_vacuum:null,last_autovacuum:null});
 const pts=[{snapshot_id:1,collection_started_at_ms:1000,current_xid:'100',postmaster_start_time:'one',in_recovery:false,relation:rel('same',100)},
 {snapshot_id:2,collection_started_at_ms:2000,current_xid:'110',postmaster_start_time:'one',in_recovery:false,relation:rel('same',50,15)},
 {snapshot_id:3,collection_started_at_ms:3000,current_xid:'115',postmaster_start_time:'one',in_recovery:false,relation:rel('other',10,5)}];
 const data=xidRelationTransitions(pts);
 assert.equal(data[1].xid_horizon_moved,true);
 assert.equal(data[1].mxid_horizon_moved,true);
 assert.equal(data[2].xid_delta,null);
 assert.equal(data[2].state,'unverified');
});

test('current_xid larger than JS safe integer is compared exactly',()=>{
 const a=point(1,0,'9007199254740993'); const b=point(2,3_600_000,'9007199254740999');
 const data=xidHistoryTimeline([a,b]).intervals[0];
 assert.equal(data.consumed_xids,6n);
 assert.equal(data.xid_per_hour,6);
});
test('Two observations two days apart never imply a continuous history',()=>{
 const a=point(1,0,100); const b=point(2,2*24*3_600_000,500);
 const timeline=xidHistoryTimeline([a,b]);
 assert.equal(timeline.intervals[0].state,'gap');
 assert.equal(xidChartSegments(xidHistoryChart(timeline,'xid')).length,2);
});
