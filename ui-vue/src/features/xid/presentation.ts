import { t } from "../i18n/locale-service.js";
import type { XidRelation, XidSection, XidSnapshot } from "./controller.js";

/** Presentation-only measurements. These are not risk/severity assessments. */
export function xidShare(age: number | null | undefined, max: number | null | undefined): number | null {
  return typeof age === "number" && Number.isFinite(age) && age >= 0 &&
    typeof max === "number" && Number.isFinite(max) && max > 0 ? age / max * 100 : null;
}

export function xidKnownCount<T>(section: XidSection<T[]> | null | undefined): number | null {
  return section && section.error === null && Array.isArray(section.data) ? section.data.length : null;
}

export type XidHorizon = "xid" | "mxid";
export function xidRelationShare(relation: XidRelation, horizon: XidHorizon): number | null {
  return horizon === "xid"
    ? xidShare(relation.xid_age, relation.effective_freeze_max_age)
    : xidShare(relation.mxid_age, relation.effective_multixact_freeze_max_age);
}

/** Top-N relative to *configured* thresholds. No implicit severity label. */
export function xidTopRelations(rows: readonly XidRelation[], horizon: XidHorizon, maxRows = 5): XidRelation[] {
  return rows.filter(row => xidRelationShare(row, horizon) !== null)
    .sort((a, b) => (xidRelationShare(b, horizon) ?? 0) - (xidRelationShare(a, horizon) ?? 0) ||
      a.schema_name.localeCompare(b.schema_name) || a.relation_name.localeCompare(b.relation_name))
    .slice(0, Math.max(0, maxRows));
}

export function xidSnapshotFacts(snapshot: XidSnapshot) {
  const sections: { label: string; section: XidSection<unknown> }[] = [
    { label: t('tsFinish.xidDatabases'), section: snapshot.databases },
    { label: t('tsFinish.xidRelations'), section: snapshot.relations },
    { label: t('tsFinish.xidThresholds'), section: snapshot.autovacuum_triggers },
    { label: "VACUUM", section: snapshot.vacuums },
    { label: "Backend XMIN", section: snapshot.backends },
    { label: "Replication slots", section: snapshot.slots },
    { label: "Prepared transactions", section: snapshot.prepared },
  ];
  return {
    databaseXidPct: xidShare(snapshot.overview.database_xid_age, snapshot.overview.autovacuum_freeze_max_age),
    databaseMxidPct: xidShare(snapshot.overview.database_mxid_age, snapshot.overview.autovacuum_multixact_freeze_max_age),
    relationCount: xidKnownCount(snapshot.relations),
    activeVacuumCount: xidKnownCount(snapshot.vacuums),
    backendCount: xidKnownCount(snapshot.backends),
    slotCount: xidKnownCount(snapshot.slots),
    preparedCount: snapshot.prepared.error === null ? snapshot.prepared.data?.count ?? null : null,
    missingSections: sections.filter(item => item.section.error !== null || item.section.data === null)
      .map(item => item.label),
  };
}
