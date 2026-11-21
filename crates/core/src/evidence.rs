use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EvidenceServerContext {
    pub connection_name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub database_oid: Option<u32>,
    pub server_version: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EvidenceQuality {
    pub level: String,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ObservationEnvelope<T> {
    pub schema_version: u32,
    pub connection_id: i64,
    pub captured_at_ms: i64,
    pub source: String,
    pub source_version: String,
    pub server_context: EvidenceServerContext,
    pub quality: EvidenceQuality,
    pub payload: T,
}

impl<T> ObservationEnvelope<T> {
    pub fn observed(
        connection_id: i64,
        captured_at_ms: i64,
        source: &str,
        source_version: &str,
        server_context: EvidenceServerContext,
        limitations: Vec<String>,
        payload: T,
    ) -> Self {
        Self {
            schema_version: 1,
            connection_id,
            captured_at_ms,
            source: source.to_owned(),
            source_version: source_version.to_owned(),
            server_context,
            quality: EvidenceQuality {
                level: if limitations.is_empty() {
                    "observed".into()
                } else {
                    "observed_with_limitations".into()
                },
                limitations,
            },
            payload,
        }
    }
}
