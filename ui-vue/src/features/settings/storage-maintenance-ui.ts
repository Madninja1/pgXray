import { t, locale } from "../i18n/locale-service.js";

/** Local-only presentation of SQLite maintenance operations. The backend remains authoritative. */
export const maintenanceActions = {
  check: {get title() { return t('settingsTs.maintenance_check_title'); }, get description() { return t('settingsTs.maintenance_check_description'); }, destructive: false},
  analyze: {get title() { return t('settingsTs.maintenance_analyze_title'); }, get description() { return t('settingsTs.maintenance_analyze_description'); }, destructive: false},
  checkpoint: {get title() { return t('settingsTs.maintenance_checkpoint_title'); }, get description() { return t('settingsTs.maintenance_checkpoint_description'); }, destructive: false},
  vacuum: {get title() { return t('settingsTs.maintenance_vacuum_title'); }, get description() { return t('settingsTs.maintenance_vacuum_description'); }, destructive: true},
  retention: {get title() { return t('settingsTs.maintenance_retention_title'); }, get description() { return t('settingsTs.maintenance_retention_description'); }, destructive: true},
  prune: {get title() { return t('settingsTs.maintenance_prune_title'); }, get description() { return t('settingsTs.maintenance_prune_description'); }, destructive: true},
} as const;

export type MaintenanceAction = keyof typeof maintenanceActions;

export function maintenanceConfirmation(action: MaintenanceAction): string {
  switch (action) {
    case "vacuum":
      return t('settingsTs.maintenance_vacuum_confirm');
    case "retention":
      return t('settingsTs.maintenance_retention_confirm');
    case "prune":
      return t('settingsTs.maintenance_prune_confirm');
    default:
      return t('settingsTs.maintenance_safe_confirm');
  }
}

export function maintenanceResultLabel(action: MaintenanceAction, detail: string, deleted: number): string {
  const title = maintenanceActions[action].title;
  if (action === "retention" || action === "prune") {
    return t('settingsTs.maintenanceDeleted', { title, detail, deleted: deleted.toLocaleString(locale.value === 'ru' ? 'ru-RU' : 'en-US') });
  }
  return t('settingsTs.maintenanceResult', { title, detail });
}
