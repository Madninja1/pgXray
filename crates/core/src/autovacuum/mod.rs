mod advisor;
mod collector;
mod interpretation;
mod model;
mod policy;
mod queries;
mod query_relation;
mod reference;

pub(crate) use advisor::advise_maintenance;
pub use collector::{collect, table_detail};
pub(crate) use model::{
    latest_continuous_maintenance_samples, maintenance_stats_pair_continuous,
    AutovacuumMaintenanceHistorySample, AutovacuumMaintenancePolicySignature,
};
pub use model::{
    AutovacuumAnalyzeProgress, AutovacuumIndex, AutovacuumInterpretation,
    AutovacuumInterpretationItem, AutovacuumInterpretationSeverity, AutovacuumMaintenanceAdvice,
    AutovacuumMaintenanceFinding, AutovacuumMethodStep, AutovacuumOverview,
    AutovacuumPolicyApplyObservedState, AutovacuumPolicyApplyVerification,
    AutovacuumPolicyCandidate, AutovacuumPolicyCandidates, AutovacuumPolicySimulation,
    AutovacuumQueryReference, AutovacuumReference, AutovacuumScaleCandidate, AutovacuumSection,
    AutovacuumSnapshot, AutovacuumSourceReference, AutovacuumTable, AutovacuumTableDetail,
    AutovacuumToast, AutovacuumUpdateStatement, AutovacuumVacuumProgress,
};
pub(crate) use policy::build_policy_candidates;
pub use policy::{apply_policy_candidate, simulate_policy};
pub use reference::reference;

#[cfg(test)]
mod tests;
