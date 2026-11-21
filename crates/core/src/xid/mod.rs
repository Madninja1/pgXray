mod collector;
mod interpretation;
mod model;
mod policy;
mod queries;
mod reference;

pub use collector::collect;
pub use model::{
    XidAutovacuumTrigger, XidBackend, XidDatabaseAge, XidFreezePolicyView, XidInterpretation,
    XidInterpretationItem, XidMethodStep, XidOverview, XidPolicyApplyVerification,
    XidPolicyCandidate, XidPolicyLiveState, XidPreparedSummary, XidQueryReference, XidReference,
    XidRelationAge, XidRelationPolicyAdvice, XidSection, XidSlot, XidSnapshot, XidSourceReference,
    XidVacuum,
};
pub use policy::apply_policy_candidate;
pub use reference::reference;

pub(crate) use model::XidPolicyHistorySample;
pub(crate) use policy::advise_relation_policy;

#[cfg(test)]
mod tests;
