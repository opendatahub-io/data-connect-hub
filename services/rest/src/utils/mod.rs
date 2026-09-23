use commons::utils::config::GlobalConnectionTypes;
use pg_meta_store::store::DatabaseConfig;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize, Clone)]
pub struct Server {
    pub address: String,
    pub port: u16,
    #[serde(rename = "sa-token-file")]
    pub sa_token_file: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FlightService {
    #[serde(rename = "ca-cert")]
    pub ca_cert: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MetricsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_metrics_address")]
    pub address: String,
    #[serde(default = "default_metrics_port")]
    pub port: u16,
}

fn default_metrics_address() -> String {
    "0.0.0.0".to_string()
}

fn default_metrics_port() -> u16 {
    9090
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            address: default_metrics_address(),
            port: default_metrics_port(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub server: Server,
    pub database: DatabaseConfig,
    #[serde(rename = "global-connection-types")]
    pub global_connection_types: GlobalConnectionTypes,
    #[serde(rename = "flight-service")]
    pub flight_service: FlightService,
    #[serde(default)]
    pub metrics: MetricsConfig,
}

pub fn default_secret_labels() -> HashMap<String, String> {
    HashMap::from([("dataconnecthub.opendatahub.io/attached".to_string(), "true".to_string())])
}

#[cfg(test)]
mod tests {
    use config::Config;

    use super::*;

    #[test]
    fn test_server_config_deserialize() {
        let toml_str = r#"
            [database]
            url = "postgresql://user-a@localhost:5432/db-a"
            
            [server]
            address = "127.0.0.1"
            port = 8080

            [global-connection-types]
            tenant-id = "opendatahub"

            [flight-service]
            ca-cert = "/etc/tls/flight/ca.crt"

            [metrics]
            enabled = true
            address = "127.0.0.2"
            port = 9191
        "#;

        let config = Config::builder()
            .add_source(config::File::from_str(toml_str, config::FileFormat::Toml))
            .build()
            .unwrap();

        let server_config: ServerConfig = config.try_deserialize().unwrap();
        assert_eq!(server_config.server.address, "127.0.0.1");
        assert_eq!(server_config.server.port, 8080);
        assert_eq!(
            server_config.flight_service.ca_cert,
            Some("/etc/tls/flight/ca.crt".to_string())
        );
        assert!(server_config.metrics.enabled);
        assert_eq!(server_config.metrics.address, "127.0.0.2");
        assert_eq!(server_config.metrics.port, 9191);
    }

    #[test]
    fn test_metrics_config_defaults() {
        let metrics = MetricsConfig::default();
        assert!(!metrics.enabled);
        assert_eq!(metrics.address, "0.0.0.0");
        assert_eq!(metrics.port, 9090);
    }

    #[test]
    fn test_server_config_missing_port() {
        let toml_str = r#"
            [server]
            address = "127.0.0.1"

            [database]
            url = "postgresql://user:pass@localhost:5432/testdb"
        "#;

        let config = Config::builder()
            .add_source(config::File::from_str(toml_str, config::FileFormat::Toml))
            .build()
            .unwrap();

        let err = config.try_deserialize::<ServerConfig>().unwrap_err();
        assert!(
            err.to_string().contains("port"),
            "expected error about 'port', got: {err}"
        );
    }

    #[test]
    fn test_server_config_missing_database() {
        let toml_str = r#"
            [server]
            address = "127.0.0.1"
            port = 8080
        "#;

        let config = Config::builder()
            .add_source(config::File::from_str(toml_str, config::FileFormat::Toml))
            .build()
            .unwrap();

        let err = config.try_deserialize::<ServerConfig>().unwrap_err();
        assert!(
            err.to_string().contains("database"),
            "expected error about 'database', got: {err}"
        );
    }

    #[test]
    fn test_server_config_missing_address() {
        let toml_str = r#"
            [server]
            port = 8080

            [database]
            url = "postgresql://user:pass@localhost:5432/testdb"
        "#;

        let config = Config::builder()
            .add_source(config::File::from_str(toml_str, config::FileFormat::Toml))
            .build()
            .unwrap();

        let err = config.try_deserialize::<ServerConfig>().unwrap_err();
        assert!(
            err.to_string().contains("address"),
            "expected error about 'address', got: {err}"
        );
    }
}
