use std::time::Duration;

use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct GlobalConnectionTypes {
    #[serde(rename = "tenant-id")]
    pub tenant_id: String,
}

impl GlobalConnectionTypes {
    pub fn new(tenant_id: String) -> Self {
        Self { tenant_id }
    }
}

fn default_connection_timeout_secs() -> u64 {
    10
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct ConnectorConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_connection_timeout_secs")]
    pub connection_timeout_secs: u64,
}

fn default_enabled() -> bool {
    true
}

impl Default for ConnectorConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            connection_timeout_secs: default_connection_timeout_secs(),
        }
    }
}

impl ConnectorConfig {
    pub fn connection_timeout(&self) -> Duration {
        Duration::from_secs(self.connection_timeout_secs)
    }

    pub fn merge(self, overrides: ConnectorConfigOverride) -> Self {
        Self {
            enabled: overrides.enabled.unwrap_or(self.enabled),
            connection_timeout_secs: overrides
                .connection_timeout_secs
                .unwrap_or(self.connection_timeout_secs),
        }
    }
}

#[derive(Debug, Deserialize, Clone, Copy, Default)]
pub struct ConnectorConfigOverride {
    pub enabled: Option<bool>,
    pub connection_timeout_secs: Option<u64>,
}
