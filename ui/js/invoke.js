"use strict";

(() => {
  const rawInvoke = window.__TAURI__?.core?.invoke;

  if (!rawInvoke) {
    window.pgxrayInvoke = null;
    return;
  }

  const loggedCommands = new Set([
    "register_initial_user",
    "login",
    "logout",
    "create_user",
    "rename_user",
    "change_user_password",
    "delete_user",
    "probe",
    "probe_saved_connection",
    "probe_connection_draft",
    "save_connection",
    "update_saved_connection",
    "delete_saved_connection",
    "start_psql_session",
    "close_psql_session",
    "set_export_directory",
    "set_application_log_directory",
    "migrate_workspace_root",
    "runtime_readiness",
    "set_history_retention",
    "delete_connection_history",
    "collect_xid_snapshot",
    "collect_pgss_snapshot",
    "pgss_query_detail",
    "collect_autovacuum_snapshot",
    "collect_pg_settings_snapshot",
    "autovacuum_table_detail",
    "simulate_autovacuum_policy",
    "create_privacy_scope",
    "create_investigation_case",
    "update_investigation_case_description",
    "create_case_intervention",
    "delete_case_intervention",
    "record_case_change_impact",
    "delete_case_change_impact",
    "import_case_file",
    "create_case_file",
    "update_case_file",
    "delete_investigation_case",
    "delete_case_artifact",
    "delete_case_comparison",
    "delete_privacy_scope",
    "reset_privacy_mapping",
    "delete_privacy_mapping_entries",
    "privacy_transform",
    "anonymize_case_artifact",
    "set_case_artifact_role",
    "export_case_artifact",
    "reveal_case_artifact",
    "open_plan_workspace",
    "prepare_plan_share",
    "compare_plan_artifacts",
    "compare_case_query_results",
    "compare_case_query_plans",
    "signal_postgres_backend",
  ]);

  const safeContextKeys = new Set([
    "id",
    "connectionId",
    "caseId",
    "artifactId",
    "baselineArtifactId",
    "candidateArtifactId",
    "comparisonId",
    "interventionId",
    "impactId",
    "scopeId",
    "sessionId",
    "relationOid",
    "pid",
    "backendStartMs",
    "firstId",
    "secondId",
    "analyzeQueries",
    "timeoutMs",
    "scanInbox",
    "deleteComparisons",
    "forceNew",
    "force",
    "useSavedPassword",
    "keepExistingPassword",
    "toplevel",
    "retentionDays",
  ]);

  let requestCounter = 0;
  let logQueue = Promise.resolve();

  function nextRequestId() {
    requestCounter = (requestCounter + 1) % 1_000_000;
    return `${Date.now().toString(36)}:${requestCounter.toString(36)}`;
  }

  function safeContext(args) {
    if (!args || typeof args !== "object") {
      return null;
    }

    const parts = [];

    for (const [key, value] of Object.entries(args)) {
      if (!safeContextKeys.has(key)) {
        continue;
      }

      if (
        typeof value !== "number"
        && typeof value !== "boolean"
        && value !== null
      ) {
        continue;
      }

      parts.push(`${key}=${String(value)}`);
    }

    return parts.length > 0 ? parts.join(" ") : null;
  }

  function queueLog(operation, phase, requestId, durationMs, context, error) {
    logQueue = logQueue
        .then(() => rawInvoke("application_log_event", {
          operation,
          phase,
          requestId,
          durationMs,
          context,
          error,
        }))
        .catch(() => {
          // Application logging is best-effort and never gates the user operation.
        });
  }

  window.pgxrayInvoke = async (command, args) => {
    if (!loggedCommands.has(command)) {
      return rawInvoke(command, args);
    }

    const requestId = nextRequestId();
    const context = safeContext(args);

    queueLog(command, "started", requestId, null, context, null);
    const startedAt = performance.now();

    try {
      const result = await rawInvoke(command, args);
      const durationMs = Math.max(0, Math.round(performance.now() - startedAt));
      queueLog(command, "succeeded", requestId, durationMs, context, null);
      return result;
    } catch (error) {
      const durationMs = Math.max(0, Math.round(performance.now() - startedAt));
      queueLog(
          command,
          "failed",
          requestId,
          durationMs,
          context,
          String(error).slice(0, 4096),
      );
      throw error;
    }
  };
})();
