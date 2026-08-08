use crate::api::ResourceMetadata;
use crate::api::errors::{MetaStoreError, SecretStoreError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Admin {
    SecretRef { secret_ref: String },

    Secret { secret: Arc<HashMap<String, String>> },
}

impl std::fmt::Debug for Admin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Admin::SecretRef { secret_ref } => f.debug_struct("SecretRef").field("secret_ref", &secret_ref).finish(),
            Admin::Secret { .. } => f.debug_struct("Secret").field("secret", &"[REDACTED]").finish(),
        }
    }
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum DataFormat {
    #[serde(rename = "tabular")]
    Tabular,
    #[serde(rename = "binary")]
    Binary,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct DataConnection {
    pub name: String,
    pub data_connection_type_id: String,
    pub format: DataFormat,
    pub admin: Option<Admin>,
    pub properties: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DataConnectionResource {
    pub metadata: ResourceMetadata,
    pub resource: DataConnection,
}

impl std::fmt::Debug for DataConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataConnection")
            .field("name", &self.name)
            .field("data_connection_type_id", &self.data_connection_type_id)
            .field("format", &self.format)
            .field("admin", &self.admin)
            .field("properties", &self.properties)
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DataConnectionType {
    pub name: String,
    pub provider: String,
    pub description: Option<String>,
    pub credentials_fields: Vec<Field>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DataConnectionTypeResource {
    pub metadata: ResourceMetadata,
    pub resource: DataConnectionType,
}

/// Persistent store for data connection and data connection type metadata.
#[async_trait::async_trait]
pub trait MetaStore {
    /// Retrieves a data connection by tenant and unique identifier.
    async fn get_data_connection(&self, tenant_id: &str, uid: &str) -> Result<DataConnectionResource, MetaStoreError>;

    /// Creates a new data connection for the given tenant.
    async fn create_data_connection(
        &self,
        tenant_id: &str,
        data_connection: DataConnection,
    ) -> Result<DataConnectionResource, MetaStoreError>;

    /// Replaces the data connection identified by `uid` with the provided value.
    async fn update_data_connection(
        &self,
        tenant_id: &str,
        uid: &str,
        update_fn: Arc<dyn Fn(DataConnection) -> Result<DataConnection, MetaStoreError> + Send + Sync>,
    ) -> Result<DataConnectionResource, MetaStoreError>;

    /// Deletes the data connection identified by `uid`.
    async fn delete_data_connection(&self, tenant_id: &str, uid: &str) -> Result<(), MetaStoreError>;

    /// Retrieves a data connection type by tenant and unique identifier.
    async fn get_data_connection_type(
        &self,
        tenant_id: &str,
        id: &str,
    ) -> Result<DataConnectionTypeResource, MetaStoreError>;

    /// Creates a new data connection type for the given tenant.
    async fn create_data_connection_type(
        &self,
        tenant_id: &str,
        data_connection_type: DataConnectionType,
    ) -> Result<DataConnectionTypeResource, MetaStoreError>;

    /// Replaces the data connection type identified by `uid` with the provided value.
    async fn update_data_connection_type(
        &self,
        tenant_id: &str,
        uid: &str,
        update_fn: Arc<dyn Fn(DataConnectionType) -> Result<DataConnectionType, MetaStoreError> + Send + Sync>,
    ) -> Result<DataConnectionTypeResource, MetaStoreError>;

    /// Deletes the data connection type identified by `uid`.
    async fn delete_data_connection_type(&self, tenant_id: &str, uid: &str) -> Result<(), MetaStoreError>;
}

#[async_trait::async_trait]
pub trait SecretStore {
    async fn get_secret(&self, namespace: &str, name: &str) -> Result<Secret, SecretStoreError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_connection_resource() -> DataConnectionResource {
        DataConnectionResource {
            metadata: ResourceMetadata {
                id: "123".to_string(),
                tenant_id: "tenant-1".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
            resource: DataConnection {
                name: "test-conn".to_string(),
                data_connection_type_id: "postgres".to_string(),
                format: DataFormat::Tabular,
                admin: Some(Admin::SecretRef {
                    secret_ref: "secret/test-conn".to_string(),
                }),
                properties: HashMap::from([("key".to_string(), "value".to_string())]),
            },
        }
    }

    #[test]
    fn test_admin_serialize_deserialize() {
        let admin = Admin::SecretRef {
            secret_ref: "secret/test".to_string(),
        };
        let json = serde_json::to_string(&admin).unwrap();
        let deserialized: Admin = serde_json::from_str(&json).unwrap();
        match (&deserialized, &admin) {
            (Admin::SecretRef { secret_ref: a }, Admin::SecretRef { secret_ref: b }) => {
                assert_eq!(a, b);
            },
            _ => panic!("expected SecretRef variant"),
        }
    }

    #[test]
    fn test_data_connection_resource_serialize_deserialize() {
        let fixture = serde_json::json!({
            "metadata": {
                "id": "123",
                "tenant_id": "tenant-1",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z"
            },
            "resource": {
                "name": "test-conn",
                "data_connection_type_id": "postgres",
                "format": "tabular",
                "admin": { "secret_ref": "secret/test-conn" },
                "properties": { "key": "value" }
            }
        });

        let res: DataConnectionResource = serde_json::from_value(fixture.clone()).unwrap();

        assert_eq!(res.metadata.id, "123");
        assert_eq!(res.metadata.tenant_id, "tenant-1");
        assert_eq!(res.resource.name, "test-conn");
        assert_eq!(res.resource.data_connection_type_id, "postgres");
        assert_eq!(res.resource.format, DataFormat::Tabular);
        match &res.resource.admin {
            Some(Admin::SecretRef { secret_ref }) => assert_eq!(secret_ref, &"secret/test-conn".to_string()),
            _ => panic!("expected SecretRef variant"),
        }
        assert_eq!(res.resource.properties["key"], "value");

        let round_tripped = serde_json::to_value(&res).unwrap();
        assert_eq!(round_tripped, fixture);
    }

    #[test]
    fn test_data_connection_resource_clone() {
        let res = sample_connection_resource();
        let cloned = res.clone();

        assert_eq!(cloned.metadata.id, res.metadata.id);
        match (&cloned.resource.admin, &res.resource.admin) {
            (Some(Admin::SecretRef { secret_ref: a }), Some(Admin::SecretRef { secret_ref: b })) => {
                assert_eq!(a, b);
            },
            _ => panic!("expected SecretRef variant"),
        }
        assert_eq!(cloned.resource.properties, res.resource.properties);
    }

    fn sample_data_connection_type_resource() -> DataConnectionTypeResource {
        DataConnectionTypeResource {
            metadata: ResourceMetadata {
                id: "dct-001".to_string(),
                tenant_id: "tenant-1".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
            resource: DataConnectionType {
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
            },
        }
    }

    #[test]
    fn test_data_connection_type_resource_serialize_deserialize() {
        let res = sample_data_connection_type_resource();
        let json = serde_json::to_value(&res).unwrap();

        assert_eq!(json["metadata"]["id"], "dct-001");
        assert_eq!(json["metadata"]["tenant_id"], "tenant-1");
        assert_eq!(json["resource"]["name"], "PostgreSQL");
        assert_eq!(json["resource"]["provider"], "postgres");
        assert_eq!(json["resource"]["description"], "PostgreSQL database connection");
        assert_eq!(json["resource"]["credentials_fields"][0]["name"], "url");
        assert_eq!(json["resource"]["credentials_fields"][0]["type"], "string");
        assert_eq!(json["resource"]["credentials_fields"][0]["required"], true);

        let deserialized: DataConnectionTypeResource = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.metadata.id, res.metadata.id);
        assert_eq!(deserialized.resource.provider, res.resource.provider);
        assert_eq!(deserialized.resource.credentials_fields.len(), 1);
        assert_eq!(deserialized.resource.credentials_fields[0].d_type, "string");
    }

    #[test]
    fn test_data_connection_type_optional_fields() {
        let json = serde_json::json!({
            "metadata": {
                "id": "dct-002",
                "tenant_id": "",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z"
            },
            "resource": {
                "id": "mysql",
                "name": "MySQL",
                "provider": "mysql",
                "description": null,
                "tenant_id": null,
                "credentials_fields": []
            }
        });

        let res: DataConnectionTypeResource = serde_json::from_value(json).unwrap();
        assert!(res.resource.description.is_none());
        assert!(res.resource.credentials_fields.is_empty());
    }

    #[test]
    fn test_data_connection_type_resource_clone() {
        let res = sample_data_connection_type_resource();
        let cloned = res.clone();

        assert_eq!(cloned.metadata.id, res.metadata.id);
        assert_eq!(cloned.resource.name, res.resource.name);
        assert_eq!(cloned.resource.provider, res.resource.provider);
        assert_eq!(cloned.resource.description, res.resource.description);
        assert_eq!(
            cloned.resource.credentials_fields.len(),
            res.resource.credentials_fields.len()
        );
    }

    #[test]
    fn test_data_connection_type_with_enum_field() {
        let json = serde_json::json!({
            "metadata": {
                "id": "dct-003",
                "tenant_id": "",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z"
            },
            "resource": {
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
            }
        });

        let res: DataConnectionTypeResource = serde_json::from_value(json).unwrap();
        let field = &res.resource.credentials_fields[0];
        assert_eq!(field.d_type, "enum");
        let enums = field.enum_values.as_ref().unwrap();
        assert_eq!(enums.len(), 2);
        assert_eq!(enums[0].value, "us-east-1");
        assert_eq!(enums[1].label, "EU West");
    }
}
