use crate::errors::metastore::MetaStoreError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DataLocation {
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DataConnection {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub provider: String,
    pub format: String,
    pub tenant_id: String,
    pub location: DataLocation,
    pub created_at: String,
    pub updated_at: String,
    pub properties: HashMap<String, String>,
}

#[async_trait::async_trait]
pub trait MetaStore {
    async fn get_connection(&self, uid: &str) -> Result<DataConnection, MetaStoreError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_connection() -> DataConnection {
        DataConnection {
            id: "123".to_string(),
            namespace: "test-ns".to_string(),
            name: "test-conn".to_string(),
            provider: "postgres".to_string(),
            format: "jdbc".to_string(),
            tenant_id: "tenant-1".to_string(),
            location: DataLocation {
                url: "postgresql://localhost:5432/db".to_string(),
            },
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            properties: HashMap::from([("key".to_string(), "value".to_string())]),
        }
    }

    #[test]
    fn test_data_location_serialize_deserialize() {
        let location = DataLocation {
            url: "postgresql://localhost:5432/db".to_string(),
        };
        let json = serde_json::to_string(&location).unwrap();
        let deserialized: DataLocation = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.url, location.url);
    }

    #[test]
    fn test_data_connection_serialize_deserialize() {
        let fixture = serde_json::json!({
            "id": "123",
            "namespace": "test-ns",
            "name": "test-conn",
            "provider": "postgres",
            "format": "jdbc",
            "tenant_id": "tenant-1",
            "location": { "url": "postgresql://localhost:5432/db" },
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "properties": { "key": "value" }
        });

        let conn: DataConnection = serde_json::from_value(fixture.clone()).unwrap();

        assert_eq!(conn.id, "123");
        assert_eq!(conn.namespace, "test-ns");
        assert_eq!(conn.name, "test-conn");
        assert_eq!(conn.provider, "postgres");
        assert_eq!(conn.format, "jdbc");
        assert_eq!(conn.tenant_id, "tenant-1");
        assert_eq!(conn.location.url, "postgresql://localhost:5432/db");
        assert_eq!(conn.created_at, "2026-01-01T00:00:00Z");
        assert_eq!(conn.updated_at, "2026-01-01T00:00:00Z");
        assert_eq!(conn.properties["key"], "value");

        let round_tripped = serde_json::to_value(&conn).unwrap();
        assert_eq!(round_tripped, fixture);
    }

    #[test]
    fn test_data_connection_clone() {
        let conn = sample_connection();
        let cloned = conn.clone();

        assert_eq!(cloned.id, conn.id);
        assert_eq!(cloned.namespace, conn.namespace);
        assert_eq!(cloned.location.url, conn.location.url);
        assert_eq!(cloned.properties, conn.properties);
    }
}
