use commons::api::connection_types::Secret;
use commons::api::connections::Admin;
use commons::api::connections::DataConnection;
use commons::utils::config::GlobalConnectionTypes;
use pg_meta_store::store::DatabaseConfig;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Deserialize, Clone)]
pub struct Server {
    pub address: String,
    pub port: u16,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FlightService {
    pub address: String,
    pub port: u16,
}

impl FlightService {
    pub fn endpoint(&self) -> String {
        format!("http://{}:{}", self.address, self.port)
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
}

pub async fn transform_data_connection(
    tenant_id: &str,
    data_connection: &DataConnection,
) -> (DataConnection, Option<Secret>) {
    let mut data_connection = data_connection.clone();

    match &data_connection.admin {
        Some(Admin::Secret { name, secret }) => {
            let properties = secret.clone();

            let secret_obj = Secret {
                name: name.to_string(),
                namespace: tenant_id.to_string(),
                properties: properties.clone(),
                labels: Arc::new(HashMap::new()),
                annotations: Arc::new(HashMap::new()),
            };
            data_connection.admin = Some(Admin::SecretRef {
                secret_ref: name.to_string(),
            });
            (data_connection, Some(secret_obj))
        },
        _ => (data_connection, None),
    }
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
            address = "127.0.0.1"
            port = 50051
        "#;

        let config = Config::builder()
            .add_source(config::File::from_str(toml_str, config::FileFormat::Toml))
            .build()
            .unwrap();

        let server_config: ServerConfig = config.try_deserialize().unwrap();
        assert_eq!(server_config.server.address, "127.0.0.1");
        assert_eq!(server_config.server.port, 8080);
        assert_eq!(server_config.flight_service.address, "127.0.0.1");
        assert_eq!(server_config.flight_service.port, 50051);
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
