use pg_meta_store::store::DatabaseConfig;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Server {
    pub address: String,
    pub port: u16,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub server: Server,
    #[serde(rename = "database")]
    pub _database: DatabaseConfig,
}

#[cfg(test)]
mod tests {
    use config::Config;

    use super::*;

    #[test]
    fn test_server_config_deserialize() {
        let toml_str = r#"
            [server]
            address = "127.0.0.1"
            port = 8080

            [database]
            url = "postgresql://user:pass@localhost:5432/testdb"
        "#;

        let config = Config::builder()
            .add_source(config::File::from_str(toml_str, config::FileFormat::Toml))
            .build()
            .unwrap();

        let server_config: ServerConfig = config.try_deserialize().unwrap();
        assert_eq!(server_config.server.address, "127.0.0.1");
        assert_eq!(server_config.server.port, 8080);
        assert_eq!(
            server_config._database.url,
            "postgresql://user:pass@localhost:5432/testdb"
        );
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
