import { t } from "../i18n/locale-service.js";
import type { CaseNavigationTarget } from "./investigation.js";

export type EvidenceRoute =
  | { view: "pgss-history"; connectionId: number; queryid: string; notice: string }
  | { view: "pgss-live"; connectionId: number; queryid: string; notice: string }
  | { view: "autovacuum"; connectionId: number; relationOid: number; notice: string }
  | { view: "locks"; connectionId: number; pid: number; notice: string }
  | { view: "artifact"; artifactId: number }
  | { view: "unavailable"; notice: string };

/** Route evidence intentionally: historical evidence is never silently replaced by live data. */
export function resolveEvidenceRoute(connectionId: number, target: CaseNavigationTarget): EvidenceRoute {
  if (target.kind === "query" && target.queryid) {
    if (target.snapshot_id != null) return {
      view: "pgss-history", connectionId, queryid: target.queryid,
      notice: t('tsFinish.evidenceHistoricalPgss', { id: String(target.snapshot_id) }),
    };
    return {
      view: "pgss-live", connectionId, queryid: target.queryid,
      notice: t('tsFinish.evidenceCurrentPgss'),
    };
  }
  if (target.kind === "relation" && Number.isSafeInteger(target.relation_oid) && (target.relation_oid ?? 0) > 0) return {
    view: "autovacuum", connectionId, relationOid: target.relation_oid!,
    notice: t('tsFinish.evidenceRelationOid', { oid: String(target.relation_oid) }),
  };
  if (target.kind === "pid" && Number.isSafeInteger(target.pid) && (target.pid ?? 0) > 0) return {
    view: "locks", connectionId, pid: target.pid!,
    notice: t('tsFinish.evidenceLocksPid', { pid: String(target.pid) }),
  };
  if (Number.isSafeInteger(target.artifact_id) && (target.artifact_id ?? 0) > 0) return {
    view: "artifact", artifactId: target.artifact_id!,
  };
  return { view: "unavailable", notice: t('tsFinish.evidenceUnavailable') };
}
