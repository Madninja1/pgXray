import { t } from "../i18n/locale-service.js";
/** UI grouping only. Each source event and all its Evidence refs remain intact. */
export interface TimelineGroupable {
  occurred_at_ms: number;
  phase: string;
  source: string;
  kind: string;
  evidence_level: string;
}

export interface TimelineGroup<T extends TimelineGroupable> {
  key: string;
  occurred_at_ms: number;
  phase: string;
  source: string;
  events: T[];
}

/** Group only adjacent events from the same observation and source, never across time. */
export function groupTimeline<T extends TimelineGroupable>(events: readonly T[]): TimelineGroup<T>[] {
  const groups: TimelineGroup<T>[] = [];
  let previousSignature = "";
  for (const event of events) {
    const signature = JSON.stringify([
      event.occurred_at_ms, event.phase, event.source, event.kind, event.evidence_level,
    ]);
    const last = groups[groups.length - 1];
    if (last && previousSignature === signature) {
      last.events.push(event);
    } else {
      groups.push({
        key: `${groups.length}:${signature}`,
        occurred_at_ms: event.occurred_at_ms,
        phase: event.phase,
        source: event.source,
        events: [event],
      });
    }
    previousSignature = signature;
  }
  return groups;
}

export function timelineSourceLabel(source: string): string {
  switch (source) {
    case "pg_stat_statements": return t('tsFinish.timelinePgss');
    case "xid": return "XID / Freeze";
    case "pg_settings": return "Settings";
    case "autovacuum": return "Autovacuum";
    case "io_wal": return "I/O & WAL";
    case "index_health": return "Index Health";
    case "manual": return t('tsFinish.timelineManual');
    default: return source;
  }
}
