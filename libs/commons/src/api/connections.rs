use crate::errors::{MetaStoreError, SecretStoreError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Admin {
    pub secret_ref: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct DataConnection {
    pub id: String,
    pub name: String,
    pub data_connection_type_id: String,
    pub format: String,
    pub tenant_id: String,
    pub admin: Admin,
    pub created_at: String,
    pub updated_at: String,
    pub properties: HashMap<String, String>,
    #[serde(skip)]
    pub credentials: HashMap<String, String>,
}

impl std::fmt::Debug for DataConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataConnection")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("data_connection_type_id", &self.data_connection_type_id)
            .field("format", &self.format)
            .field("tenant_id", &self.tenant_id)
            .field("admin", &self.admin)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .field("properties", &self.properties)
            .field("credentials", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EnumValue {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Field {
    pub name: String,
    pub label: String,
    pub description: Option<String>,
    pub required: bool,
    #[serde(rename = "type")]
    pub d_type: String,
    pub enum_values: Option<Vec<EnumValue>>,
    pub default_value: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DataConnectionType {
    pub id: String,
    pub tenant_id: Option<String>,
    pub name: String,
    pub provider: String,
    pub description: Option<String>,
    pub credentials_fields: Vec<Field>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Secret {
    pub name: String,
    pub namespace: String,
    pub properties: HashMap<String, String>,
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Secret")
            .field("name", &self.name)
            .field("namespace", &self.namespace)
            .field("properties", &"[REDACTED]")
            .finish()
    }
}

#[async_trait::async_trait]
pub trait MetaStore {
    async fn get_connection(&self, tenant_id: &str, uid: &str) -> Result<DataConnection, MetaStoreError>;
    async fn get_data_connection_type(&self, tenant_id: &str, id: &str) -> Result<DataConnectionType, MetaStoreError>;
}

#[async_trait::async_trait]
pub trait SecretStore {
    async fn get_secret(&self, namespace: &str, name: &str) -> Result<Secret, SecretStoreError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_connection() -> DataConnection {
        DataConnection {
            id: "123".to_string(),
            name: "test-conn".to_string(),
            data_connection_type_id: "postgres".to_string(),
            format: "jdbc".to_string(),
            tenant_id: "tenant-1".to_string(),
            admin: Admin {
                secret_ref: "secret/test-conn".to_string(),
            },
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            properties: HashMap::from([("key".to_string(), "value".to_string())]),
            credentials: HashMap::new(),
        }
    }

    #[test]
    fn test_admin_serialize_deserialize() {
        let admin = Admin {
            secret_ref: "secret/test".to_string(),
        };
        let json = serde_json::to_string(&admin).unwrap();
        let deserialized: Admin = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.secret_ref, admin.secret_ref);
    }

    #[test]
    fn test_data_connection_serialize_deserialize() {
        let fixture = serde_json::json!({
            "id": "123",
            "name": "test-conn",
            "data_connection_type_id": "postgres",
            "format": "jdbc",
            "tenant_id": "tenant-1",
            "admin": { "secret_ref": "secret/test-conn" },
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "properties": { "key": "value" }
        });

        let conn: DataConnection = serde_json::from_value(fixture.clone()).unwrap();

        assert_eq!(conn.id, "123");
        assert_eq!(conn.name, "test-conn");
        assert_eq!(conn.data_connection_type_id, "postgres");
        assert_eq!(conn.format, "jdbc");
        assert_eq!(conn.tenant_id, "tenant-1");
        assert_eq!(conn.admin.secret_ref, "secret/test-conn");
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
        assert_eq!(cloned.admin.secret_ref, conn.admin.secret_ref);
        assert_eq!(cloned.properties, conn.properties);
    }

    fn sample_data_connection_type() -> DataConnectionType {
        DataConnectionType {
            id: "postgres".to_string(),
            tenant_id: Some("tenant-1".to_string()),
            name: "PostgreSQL".to_string(),
            provider: "postgres".to_string(),
            description: Some("PostgreSQL database connection".to_string()),
            credentials_fields: vec![Field {
                name: "url".to_string(),
                label: "URL".to_string(),
                description: Some("PostgreSQL connection URL".to_string()),
                required: true,
                d_type: "string".to_string(),
                enum_values: None,
                default_value: None,
            }],
        }
    }

    #[test]
    fn test_data_connection_type_serialize_deserialize() {
        let dct = sample_data_connection_type();
        let json = serde_json::to_value(&dct).unwrap();

        assert_eq!(json["id"], "postgres");
        assert_eq!(json["tenant_id"], "tenant-1");
        assert_eq!(json["name"], "PostgreSQL");
        assert_eq!(json["provider"], "postgres");
        assert_eq!(json["description"], "PostgreSQL database connection");
        assert_eq!(json["credentials_fields"][0]["name"], "url");
        assert_eq!(json["credentials_fields"][0]["type"], "string");
        assert_eq!(json["credentials_fields"][0]["required"], true);

        let deserialized: DataConnectionType = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.id, dct.id);
        assert_eq!(deserialized.provider, dct.provider);
        assert_eq!(deserialized.credentials_fields.len(), 1);
        assert_eq!(deserialized.credentials_fields[0].d_type, "string");
    }

    #[test]
    fn test_data_connection_type_optional_fields() {
        let json = serde_json::json!({
            "id": "mysql",
            "name": "MySQL",
            "provider": "mysql",
            "description": null,
            "tenant_id": null,
            "credentials_fields": []
        });

        let dct: DataConnectionType = serde_json::from_value(json).unwrap();
        assert!(dct.description.is_none());
        assert!(dct.tenant_id.is_none());
        assert!(dct.credentials_fields.is_empty());
    }

    #[test]
    fn test_data_connection_type_clone() {
        let dct = sample_data_connection_type();
        let cloned = dct.clone();

        assert_eq!(cloned.id, dct.id);
        assert_eq!(cloned.name, dct.name);
        assert_eq!(cloned.provider, dct.provider);
        assert_eq!(cloned.description, dct.description);
        assert_eq!(cloned.credentials_fields.len(), dct.credentials_fields.len());
        assert_eq!(cloned.credentials_fields[0].name, dct.credentials_fields[0].name);
    }

    #[test]
    fn test_data_connection_type_with_enum_field() {
        let json = serde_json::json!({
            "id": "s3",
            "name": "S3",
            "provider": "s3",
            "credentials_fields": [
                {
                    "name": "region",
                    "label": "Region",
                    "required": true,
                    "type": "enum",
                    "enum_values": [
                        { "value": "us-east-1", "label": "US East" },
                        { "value": "eu-west-1", "label": "EU West" }
                    ]
                }
            ]
        });

        let dct: DataConnectionType = serde_json::from_value(json).unwrap();
        let field = &dct.credentials_fields[0];
        assert_eq!(field.d_type, "enum");
        let enums = field.enum_values.as_ref().unwrap();
        assert_eq!(enums.len(), 2);
        assert_eq!(enums[0].value, "us-east-1");
        assert_eq!(enums[1].label, "EU West");
    }
}
