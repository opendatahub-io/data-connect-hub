/// AuditConfig controls whether audit records are emitted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuditConfig {
    /// enabled indicates whether audit events should be emitted.
    pub enabled: bool,
    /// log_queries indicates whether the `audit.query` field should be
    /// included in audit records. Queries can contain sensitive data, so
    /// this is off by default.
    pub log_queries: bool,
}

impl AuditConfig {
    /// from_env reads the controller-provided `AUDIT_ENABLED` and
    /// `AUDIT_LOG_QUERIES` settings.
    pub fn from_env() -> Self {
        Self::from_values(
            std::env::var("AUDIT_ENABLED").ok().as_deref(),
            std::env::var("AUDIT_LOG_QUERIES").ok().as_deref(),
        )
    }

    fn from_values(enabled: Option<&str>, log_queries: Option<&str>) -> Self {
        Self {
            enabled: env_flag(enabled),
            log_queries: env_flag(log_queries),
        }
    }
}

/// env_flag parses a controller-provided environment variable, enabling only
/// on an explicit case-insensitive "true".
fn env_flag(value: Option<&str>) -> bool {
    value.is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_config_is_disabled_by_default_and_requires_true() {
        assert!(!AuditConfig::from_values(None, None).enabled);
        assert!(!AuditConfig::from_values(Some("false"), None).enabled);
        assert!(!AuditConfig::from_values(Some("1"), None).enabled);
        assert!(AuditConfig::from_values(Some("TRUE"), None).enabled);
    }

    #[test]
    fn audit_config_log_queries_is_disabled_by_default_and_requires_true() {
        assert!(!AuditConfig::from_values(None, None).log_queries);
        assert!(!AuditConfig::from_values(None, Some("false")).log_queries);
        assert!(!AuditConfig::from_values(None, Some("1")).log_queries);
        assert!(AuditConfig::from_values(None, Some("TRUE")).log_queries);
    }
}
