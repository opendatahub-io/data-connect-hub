use std::time::Duration;

use serde::{Deserialize, Deserializer};

/// GlobalConnectionTypes lists the tenants whose connection types are visible
/// to every tenant: the DCH service namespace, which holds the built-in
/// connection types, and the designated global namespaces.
#[derive(Debug, Deserialize, Clone)]
pub struct GlobalConnectionTypes {
    #[serde(rename = "tenant-ids", deserialize_with = "non_empty_tenant_ids")]
    pub tenant_ids: Vec<String>,
}

impl GlobalConnectionTypes {
    pub fn new(tenant_ids: Vec<String>) -> Self {
        Self { tenant_ids }
    }
}

fn non_empty_tenant_ids<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    let tenant_ids = Vec::<String>::deserialize(deserializer)?;
    if tenant_ids.is_empty() || tenant_ids.iter().any(|id| id.is_empty() || id.trim() != id) {
        return Err(serde::de::Error::custom(
            "tenant-ids must list at least one tenant, without empty entries or surrounding whitespace",
        ));
    }
    Ok(tenant_ids)
}

fn default_connection_timeout_secs() -> u64 {
    10
}

fn default_request_timeout_secs() -> u64 {
    30
}

fn default_read_timeout_secs() -> u64 {
    30
}

fn default_chunk_size() -> usize {
    4 * 1024 * 1024
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct ConnectorConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_connection_timeout_secs")]
    pub connection_timeout_secs: u64,
    #[serde(default = "default_request_timeout_secs")]
    pub request_timeout_secs: u64,
    #[serde(default = "default_read_timeout_secs")]
    pub read_timeout_secs: u64,
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
}

fn default_enabled() -> bool {
    true
}

impl Default for ConnectorConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            connection_timeout_secs: default_connection_timeout_secs(),
            request_timeout_secs: default_request_timeout_secs(),
            read_timeout_secs: default_read_timeout_secs(),
            chunk_size: default_chunk_size(),
        }
    }
}

impl ConnectorConfig {
    pub fn connection_timeout(&self) -> Duration {
        Duration::from_secs(self.connection_timeout_secs)
    }

    pub fn request_timeout(&self) -> Duration {
        Duration::from_secs(self.request_timeout_secs)
    }

    pub fn read_timeout(&self) -> Duration {
        Duration::from_secs(self.read_timeout_secs)
    }

    pub fn merge(self, overrides: ConnectorConfigOverride) -> Self {
        Self {
            enabled: overrides.enabled.unwrap_or(self.enabled),
            connection_timeout_secs: overrides
                .connection_timeout_secs
                .unwrap_or(self.connection_timeout_secs),
            request_timeout_secs: overrides.request_timeout_secs.unwrap_or(self.request_timeout_secs),
            read_timeout_secs: overrides.read_timeout_secs.unwrap_or(self.read_timeout_secs),
            chunk_size: overrides.chunk_size.unwrap_or(self.chunk_size),
        }
    }
}

#[derive(Debug, Deserialize, Clone, Copy, Default)]
pub struct ConnectorConfigOverride {
    pub enabled: Option<bool>,
    pub connection_timeout_secs: Option<u64>,
    pub request_timeout_secs: Option<u64>,
    pub read_timeout_secs: Option<u64>,
    pub chunk_size: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::GlobalConnectionTypes;

    #[test]
    fn test_global_connection_types_lists_tenants() {
        let config: GlobalConnectionTypes = serde_json::from_str(r#"{"tenant-ids": ["dch", "global-a"]}"#).unwrap();
        assert_eq!(config.tenant_ids, vec!["dch", "global-a"]);
    }

    #[test]
    fn test_global_connection_types_rejects_empty_tenants() {
        for json in [
            r#"{"tenant-ids": []}"#,
            r#"{"tenant-ids": [""]}"#,
            r#"{"tenant-ids": [" "]}"#,
            r#"{"tenant-ids": [" opendatahub"]}"#,
        ] {
            assert!(
                serde_json::from_str::<GlobalConnectionTypes>(json).is_err(),
                "{json} should be rejected"
            );
        }
    }
}
