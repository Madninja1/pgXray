import { locale } from "../dist/features/i18n/locale-service.js";
locale.value = "ru";
import assert from 'node:assert/strict';
import test from 'node:test';
import {
  comparisonSources, comparisonRequest, isPlanComparison, isQueryResultComparison,
} from '../dist/features/tools/case-comparisons.js';

const caseId = 19;
const artifacts = [
  { artifact: { id: 1, case_id: caseId, kind: 'sql', original_name: 'baseline.sql', visibility: 'sensitive' } },
  { artifact: { id: 2, case_id: caseId, kind: 'sql', original_name: 'candidate.sql', visibility: 'sensitive' } },
  { artifact: { id: 3, case_id: caseId, kind: 'explain', original_name: 'baseline.json', visibility: 'sensitive' } },
  { artifact: { id: 4, case_id: caseId, kind: 'explain', original_name: 'candidate.json', visibility: 'sensitive' } },
  { artifact: { id: 5, case_id: caseId + 1, kind: 'sql', original_name: 'foreign.sql', visibility: 'sensitive' } },
  { artifact: { id: 6, case_id: caseId, kind: 'sql', original_name: 'public.sql', visibility: 'shareable' } },
];

test('only sensitive source artifacts belonging to selected case are selectable', () => {
  assert.deepEqual(comparisonSources(artifacts, caseId, 'query_explain').map(a => a.artifact.id), [1, 2]);
  assert.deepEqual(comparisonSources(artifacts, caseId, 'saved_plan').map(a => a.artifact.id), [3, 4]);
});

test('rejects same-side artifacts, wrong case and missing sources', () => {
  assert.throws(() => comparisonRequest(artifacts, caseId, 'query_explain', 1, 1, 300_000, false), /разными/);
  assert.throws(() => comparisonRequest(artifacts, caseId, 'query_explain', 1, 5, 300_000, false), /текущего расследования/);
  assert.throws(() => comparisonRequest(artifacts, caseId, 'saved_plan', null, 4, 300_000, false), /baseline/);
});

test('routes safe EXPLAIN and saved plans to existing Tauri commands', () => {
  assert.deepEqual(comparisonRequest(artifacts, caseId, 'saved_plan', 3, 4, 300_000, false), {
    command: 'compare_plan_artifacts', args: { baselineArtifactId: 3, candidateArtifactId: 4 },
  });
  assert.deepEqual(comparisonRequest(artifacts, caseId, 'query_explain', 1, 2, 30_000, false), {
    command: 'compare_case_query_plans', args: {
      caseId, baselineArtifactId: 1, candidateArtifactId: 2, analyzeQueries: false, timeoutMs: 30_000,
    },
  });
});

test('requires explicit confirmation for SQL execution', () => {
  assert.throws(() => comparisonRequest(artifacts, caseId, 'query_analyze', 1, 2, 300_000, false), /Подтверди/);
  assert.throws(() => comparisonRequest(artifacts, caseId, 'query_result', 1, 2, 300_000, false), /Подтверди/);
  assert.deepEqual(comparisonRequest(artifacts, caseId, 'query_analyze', 1, 2, 300_000, true), {
    command: 'compare_case_query_plans', args: {
      caseId, baselineArtifactId: 1, candidateArtifactId: 2, analyzeQueries: true, timeoutMs: 300_000,
    },
  });
  assert.deepEqual(comparisonRequest(artifacts, caseId, 'query_result', 1, 2, 300_000, true), {
    command: 'compare_case_query_results', args: { caseId, baselineArtifactId: 1, candidateArtifactId: 2, timeoutMs: 300_000 },
  });
});

test('rejects unapproved timeout values', () => {
  assert.throws(() => comparisonRequest(artifacts, caseId, 'query_result', 1, 2, 0, true), /timeout/);
  assert.throws(() => comparisonRequest(artifacts, caseId, 'query_explain', 1, 2, 999_999, false), /timeout/);
});

test('stored result shape detection does not confuse plan and row comparisons', () => {
  const plan = { summary: ['x'], metrics: [] };
  const query = { equivalent: false, baseline_rows: 2, candidate_rows: 1 };
  assert.equal(isPlanComparison(plan), true);
  assert.equal(isPlanComparison(query), false);
  assert.equal(isQueryResultComparison(query), true);
  assert.equal(isQueryResultComparison(plan), false);
  assert.equal(isQueryResultComparison(null), false);
});
