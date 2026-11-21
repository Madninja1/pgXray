import { t } from "../i18n/locale-service.js";
import { xidIsSystemSchema, type XidRelation, type XidTrigger } from "./controller.js";
import { xidRelationShare, type XidHorizon } from "./presentation.js";

export type XidRelationKind = "all" | "toast" | "non-toast";
export type XidRelationHorizon = "either" | XidHorizon;
export interface XidRelationFilters {
  query: string; schemaScope: "all" | "user" | "system";
  kind: XidRelationKind; horizon: XidRelationHorizon; minPct: number;
}
export function xidFilterRelations(rows: readonly XidRelation[], filters: XidRelationFilters): XidRelation[] {
  const term = filters.query.trim().toLocaleLowerCase("ru");
  const minimum = Number.isFinite(filters.minPct) ? Math.max(0, filters.minPct) : 0;
  return rows.filter(row => {
    if (filters.schemaScope !== "all" &&
        (filters.schemaScope === "system") !== xidIsSystemSchema(row.schema_name)) return false;
    if (filters.kind === "toast" && row.relation_type !== "t") return false;
    if (filters.kind === "non-toast" && row.relation_type === "t") return false;
    if (term && !`${row.schema_name} ${row.relation_name} ${row.relation_oid} ${row.parent_schema ?? ""} ${row.parent_relation ?? ""}`
      .toLocaleLowerCase("ru").includes(term)) return false;
    if (minimum === 0) return true;
    const xid = xidRelationShare(row, "xid"), mxid = xidRelationShare(row, "mxid");
    return filters.horizon === "xid" ? xid !== null && xid >= minimum :
      filters.horizon === "mxid" ? mxid !== null && mxid >= minimum :
      (xid !== null && xid >= minimum) || (mxid !== null && mxid >= minimum);
  });
}

export type XidTriggerState = "disabled" | "reached" | "unknown" | "below";
export function xidTriggerState(trigger: XidTrigger): XidTriggerState {
  if (!trigger.autovacuum_enabled) return "disabled";
  if (trigger.dead_trigger_reached || trigger.insert_trigger_reached) return "reached";
  if (trigger.thresholds_reliable === false || trigger.estimated_dead_tuple_trigger === null ||
      trigger.estimated_insert_trigger === null) return "unknown";
  return "below";
}
export function xidTriggerStateLabel(trigger: XidTrigger): string {
  switch (xidTriggerState(trigger)) {
    case "disabled": return t('tsFinish.xidDisabled');
    case "reached": return t('tsFinish.xidReached');
    case "unknown": return t('tsFinish.xidUnknown');
    case "below": return t('tsFinish.xidBelow');
  }
}

/** Select options for the relation investigator. Materialized views live in "other". */
export type XidPickerScope = "all" | "user" | "system" | "toast" | "other";
export function xidRelationPicker(rows: readonly XidRelation[], scope: XidPickerScope, query: string): XidRelation[] {
  const text = query.trim().toLocaleLowerCase("ru-RU");
  return rows.filter(row => {
    if (scope === "user" && xidIsSystemSchema(row.schema_name)) return false;
    if (scope === "system" && !xidIsSystemSchema(row.schema_name)) return false;
    if (scope === "toast" && row.relation_type !== "t") return false;
    if (scope === "other" && (row.relation_type === "r" || row.relation_type === "t")) return false;
    return !text || `${row.schema_name}.${row.relation_name} ${row.relation_oid} ${row.parent_schema ?? ""}.${row.parent_relation ?? ""}`
      .toLocaleLowerCase("ru-RU").includes(text);
  });
}
