use crate::api::ResourceMetadata;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Admin {
    SecretRef {
        secret_ref: String,
    },

    Secret {
        name: String,
        secret: HashMap<String, String>,
    },
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

impl std::fmt::Display for DataFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataFormat::Tabular => write!(f, "tabular"),
            DataFormat::Binary => write!(f, "binary"),
        }
    }
}

impl DataFormat {
    pub fn from_string(s: &str) -> Result<DataFormat, String> {
        match s {
            "tabular" => Ok(DataFormat::Tabular),
            "binary" => Ok(DataFormat::Binary),
            _ => Err(format!("invalid data format: {}", s)),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum DataConnectionState {
    /// The data connection is ready to be used for ingestion or for secret consumption
    #[serde(rename = "ready")]
    Ready,
    /// The data connection is not ready to be used for ingestion but can be used for secret consumption
    #[serde(rename = "ingestion_not_ready")]
    IngestionNotReady,
    /// The data connection points to a secret that is not valid or missing.
    #[serde(rename = "not_ready")]
    NotReady,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum PhaseState {
    #[serde(rename = "secret_ready")]
    SecretReady,
    #[serde(rename = "secret_valid")]
    SecretInvalid(String),
    #[serde(rename = "secret_not_found")]
    SecretNotFound,
    #[serde(rename = "ingestion_ready")]
    IngestionReady,
    #[serde(rename = "ingestion_not_ready")]
    IngestionNotReady,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DataConnectionStatus {
    pub state: DataConnectionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

impl Default for DataConnectionStatus {
    fn default() -> Self {
        Self {
            state: DataConnectionState::NotReady,
            message: None,
            updated_at: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct DataConnection {
    pub name: String,
    pub data_connection_type_id: String,
    pub format: DataFormat,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin: Option<Admin>,
    pub properties: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DataConnectionResource {
    pub metadata: ResourceMetadata,
    pub resource: DataConnection,
    #[serde(default)]
    pub status: DataConnectionStatus,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_connection_resource() -> DataConnectionResource {
        DataConnectionResource {
            metadata: ResourceMetadata {
                id: "123".to_string(),
                tenant_id: Some("tenant-1".to_string()),
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
            status: DataConnectionStatus {
                state: DataConnectionState::NotReady,
                message: None,
                updated_at: None,
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
            },
            "status": {
                "state": "not_ready"
            }
        });

        let res: DataConnectionResource = serde_json::from_value(fixture.clone()).unwrap();

        assert_eq!(res.metadata.id, "123");
        assert_eq!(res.metadata.tenant_id, Some("tenant-1".to_string()));
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

    #[test]
    fn test_status_serialize_ingestion_not_ready() {
        let status = DataConnectionStatus {
            state: DataConnectionState::NotReady,
            message: None,
            updated_at: None,
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["state"], "not_ready");
        assert_eq!(json["message"], serde_json::Value::Null);
    }

    #[test]
    fn test_status_serialize_ingestion_ready_with_message() {
        let status = DataConnectionStatus {
            state: DataConnectionState::Ready,
            message: Some("All checks passed".to_string()),
            updated_at: None,
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["state"], "ready");
        assert_eq!(json["message"], "All checks passed");
    }

    #[test]
    fn test_status_roundtrip() {
        let status = DataConnectionStatus {
            state: DataConnectionState::NotReady,
            message: Some("ready".to_string()),
            updated_at: None,
        };
        let json = serde_json::to_string(&status).unwrap();
        let deserialized: DataConnectionStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, status);
    }

    #[test]
    fn test_status_deserialize_from_json() {
        let json = serde_json::json!({
            "state": "not_ready",
            "message": "Connection timeout"
        });
        let status: DataConnectionStatus = serde_json::from_value(json).unwrap();
        assert_eq!(status.state, DataConnectionState::NotReady);
        assert_eq!(status.message.as_deref(), Some("Connection timeout"));
    }

    #[test]
    fn test_status_deserialize_null_message() {
        let json = serde_json::json!({
            "state": "ready",
            "message": null
        });
        let status: DataConnectionStatus = serde_json::from_value(json).unwrap();
        assert_eq!(status.state, DataConnectionState::Ready);
        assert!(status.message.is_none());
    }

    #[test]
    fn test_status_equality() {
        let a = DataConnectionStatus {
            state: DataConnectionState::Ready,
            message: Some("ok".to_string()),
            updated_at: None,
        };
        let b = DataConnectionStatus {
            state: DataConnectionState::Ready,
            message: Some("ok".to_string()),
            updated_at: None,
        };
        let c = DataConnectionStatus {
            state: DataConnectionState::NotReady,
            message: Some("ok".to_string()),
            updated_at: None,
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_connection_resource_includes_status() {
        let res = sample_connection_resource();
        let json = serde_json::to_value(&res).unwrap();
        assert_eq!(json["status"]["state"], "not_ready");
        assert_eq!(json["status"]["message"], serde_json::Value::Null);
    }

    #[test]
    fn test_deserialize_legacy_resource_without_status() {
        let json = serde_json::json!({
            "metadata": {
                "id": "legacy-1",
                "tenant_id": "tenant-1",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z"
            },
            "resource": {
                "name": "old-conn",
                "data_connection_type_id": "postgres",
                "format": "tabular",
                "properties": {}
            }
        });

        let res: DataConnectionResource = serde_json::from_value(json).unwrap();
        assert_eq!(res.status.state, DataConnectionState::NotReady);
        assert!(res.status.message.is_none());
    }
}
