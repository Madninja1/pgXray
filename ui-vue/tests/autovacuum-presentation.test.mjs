import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from "node:assert/strict";
import test from "node:test";
import { avUniqueNotes, avAdviceNextStep, avAdvisorRequirements } from "../dist/features/autovacuum/presentation.js";

test("deduplicates only identical limitations and does not omit distinct issues",()=>{
  assert.deepEqual(avUniqueNotes(["  Ошибка   таблицы ","Ошибка таблицы","Ошибка VACUUM", "", "Ошибка VACUUM"]),["Ошибка таблицы","Ошибка VACUUM"]);
});

test("advice with insufficient history never presents a policy change as next action",()=>{
  const note=avAdviceNextStep({history_points:1,history_span_seconds:null,stats_period_continuous:false,status:'candidate'});
  assert.match(note,/минимум 3 снимка/);
  assert.doesNotMatch(note,/Примени|Измени/);
});

test("advisor prioritizes unavailable continuity before candidate suggestions",()=>{
  const note=avAdviceNextStep({history_points:3,history_span_seconds:3600,stats_period_continuous:false,status:'candidate'});
  assert.match(note,/Непрерывность/);
});

test("investigation does not tell user to reduce autovacuum threshold",()=>{
  const note=avAdviceNextStep({history_points:5,history_span_seconds:3600,stats_period_continuous:true,status:'investigate'});
  assert.match(note,/workers/);
  assert.match(note,/не устраняет/);
});

test("unknown stats_reset explains excluded observations instead of just requesting three more",()=>{
  const note=avAdviceNextStep(
    {history_points:1,history_span_seconds:null,stats_period_continuous:false,status:'insufficient_history'},
    {savedPoints:3,resetUnknown:true},
  );
  assert.match(note,/3 наблюдений/);
  assert.match(note,/только 1/);
  assert.match(note,/stats_reset/);
  assert.match(note,/новые снимки сами по себе не подтвердят/);
  assert.doesNotMatch(note,/Нужно минимум 3 снимка/);
});

test("known reset timestamp still recommends enough comparable samples",()=>{
  const note=avAdviceNextStep(
    {history_points:1,history_span_seconds:null,stats_period_continuous:false,status:'insufficient_history'},
    {savedPoints:3,resetUnknown:false},
  );
  assert.match(note,/минимум 3 снимка/);
});

test("advisor criteria explicitly explain why nine saved snapshots can produce one comparable point",()=>{
  const items=avAdvisorRequirements({history_points:1,history_span_seconds:null,stats_period_continuous:false},
    {savedPoints:9,trackCounts:true,statsReset:null});
  assert.equal(items.length,5);
  assert.equal(items[0].met,true);
  assert.equal(items[1].met,false);
  assert.match(items[1].explanation,/NULL/);
  assert.match(items[2].explanation,/9/);
  assert.match(items[2].explanation,/1/);
  assert.equal(items[3].met,false);
  assert.equal(items[4].met,false);
});

test("advisor criteria accept a three-point continuous half-hour segment",()=>{
  const items=avAdvisorRequirements({history_points:3,history_span_seconds:1800,stats_period_continuous:true},
    {savedPoints:5,trackCounts:true,statsReset:'2026-11-14'});
  assert.ok(items.every(x=>x.met));
});
