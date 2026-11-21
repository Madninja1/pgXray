import assert from "node:assert/strict";
import test from "node:test";
import { suggestComparisonPeriods } from "../dist/features/what-changed/period-suggestions.js";
const hour = 3_600_000;
function sample(ms, kind, host = "prod-db") {
  return { id: ms, collection_started_at_ms: ms, host, port: 5432, database: "postgres", database_oid: 5,
    username: "reader", connection_id: 12, connection_name: "prod", kind };
}
const base = Date.UTC(2026, 10, 18, 8);
test("four spaced sessions produce two valid periods using real snapshot timestamps", () => {
  const kinds = ["xid", "pgss", "autovacuum", "pg_settings", "io_wal", "index_health"];
  const source = Object.fromEntries(kinds.map(kind => [kind, [0,1,2,3].map(i => sample(base + i*hour + kinds.indexOf(kind)*1000,kind))]));
  const original = source.pgss.map(item => item.collection_started_at_ms);
  const selected = suggestComparisonPeriods(source);
  assert.ok(selected);
  assert.equal(selected.usedSessions, 4);
  assert.equal(selected.readySources, 6);
  assert.ok(selected.baselineFromMs < selected.baselineToMs);
  assert.ok(selected.baselineToMs < selected.incidentFromMs);
  assert.ok(selected.incidentFromMs < selected.incidentToMs);
  assert.deepEqual(source.pgss.map(item => item.collection_started_at_ms), original);
});
test("one snapshot per period is useful for index/state but not PGSS rates", () => {
  const selected = suggestComparisonPeriods({ index_health: [sample(base, "index_health"),sample(base+hour,"index_health")],
    pgss: [sample(base,"pgss"), sample(base+hour,"pgss")] });
  assert.equal(selected?.readySources, 1);
  assert.equal(selected?.usedSessions, 2);
});
test("different PostgreSQL targets under the same connection id must not be mixed", () => {
  const result = suggestComparisonPeriods({ pg_settings: [sample(base,"pg_settings","old-db"),
    sample(base+hour,"pg_settings","old-db"), sample(base+2*hour,"pg_settings","new-db"),
    sample(base+3*hour,"pg_settings","new-db")] });
  assert.equal(result?.usedSessions, 2);
  assert.equal(result?.readySources, 1);
  assert.ok(result.baselineFromMs > base + hour);
});
test("missing history or one collection session produces no false suggestion", () => {
  assert.equal(suggestComparisonPeriods({}), null);
  assert.equal(suggestComparisonPeriods({ pgss: [sample(base,"pgss")] }), null);
  assert.equal(suggestComparisonPeriods({ xid: [sample(base,"xid"),sample(base+60_000,"xid")] }), null);
});
test("partial three-session history keeps both periods separate", () => {
  const data = { pgss: [0,1,2].map(i => sample(base+i*hour,"pgss")) };
  const result = suggestComparisonPeriods(data);
  assert.equal(result?.usedSessions, 3);
  assert.equal(result?.readySources, 0);
  assert.ok(result.baselineToMs < result.incidentFromMs);
});
