use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::api::ResourceMetadata;
use crate::api::errors::MetaStoreError;

const MAX_NAME_LENGTH: usize = 253;
const MAX_LABEL_LENGTH: usize = 256;
const MAX_DESCRIPTION_LENGTH: usize = 1024;
const MAX_DEFAULT_VALUE_LENGTH: usize = 1024;
const VALID_FIELD_TYPES: &[&str] = &["string", "enum"];

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EnumValue {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Field {
    pub name: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub required: bool,
    #[serde(rename = "type")]
    pub d_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<EnumValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
}

impl Field {
    pub fn validate(&self) -> Result<(), MetaStoreError> {
        if self.name.is_empty() {
            return Err(MetaStoreError::Validation("field name must not be empty".to_string()));
        }
        if self.name.len() > MAX_NAME_LENGTH {
            return Err(MetaStoreError::Validation(format!(
                "field name '{}' exceeds maximum length of {MAX_NAME_LENGTH}",
                self.name
            )));
        }
        if self.label.is_empty() {
            return Err(MetaStoreError::Validation(format!(
                "field '{}': label must not be empty",
                self.name
            )));
        }
        if self.label.len() > MAX_LABEL_LENGTH {
            return Err(MetaStoreError::Validation(format!(
                "field '{}': label exceeds maximum length of {MAX_LABEL_LENGTH}",
                self.name
            )));
        }
        if let Some(desc) = &self.description
            && desc.len() > MAX_DESCRIPTION_LENGTH
        {
            return Err(MetaStoreError::Validation(format!(
                "field '{}': description exceeds maximum length of {MAX_DESCRIPTION_LENGTH}",
                self.name
            )));
        }
        if !VALID_FIELD_TYPES.contains(&self.d_type.as_str()) {
            return Err(MetaStoreError::Validation(format!(
                "field '{}': type '{}' is not valid; valid types are: {}",
                self.name,
                self.d_type,
                VALID_FIELD_TYPES.join(", ")
            )));
        }
        if self.d_type == "enum" {
            match &self.enum_values {
                None => {
                    return Err(MetaStoreError::Validation(format!(
                        "field '{}': enum_values must be provided when type is 'enum'",
                        self.name
                    )));
                },
                Some(values) if values.is_empty() => {
                    return Err(MetaStoreError::Validation(format!(
                        "field '{}': enum_values must not be empty when type is 'enum'",
                        self.name
                    )));
                },
                Some(values) => {
                    for v in values {
                        if v.value.is_empty() {
                            return Err(MetaStoreError::Validation(format!(
                                "field '{}': enum value must not be empty",
                                self.name
                            )));
                        }
                        if v.value.len() > MAX_LABEL_LENGTH {
                            return Err(MetaStoreError::Validation(format!(
                                "field '{}': enum value '{}' exceeds maximum length of {MAX_LABEL_LENGTH}",
                                self.name, v.value
                            )));
                        }
                        if v.label.is_empty() {
                            return Err(MetaStoreError::Validation(format!(
                                "field '{}': enum label must not be empty",
                                self.name
                            )));
                        }
                        if v.label.len() > MAX_LABEL_LENGTH {
                            return Err(MetaStoreError::Validation(format!(
                                "field '{}': enum label '{}' exceeds maximum length of {MAX_LABEL_LENGTH}",
                                self.name, v.label
                            )));
                        }
                    }
                },
            }
        }
        if let Some(default) = &self.default_value
            && default.len() > MAX_DEFAULT_VALUE_LENGTH
        {
            return Err(MetaStoreError::Validation(format!(
                "field '{}': default_value exceeds maximum length of {MAX_DEFAULT_VALUE_LENGTH}",
                self.name
            )));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Secret {
    pub name: String,
    pub namespace: String,
    pub properties: Arc<HashMap<String, String>>,
    pub labels: Arc<HashMap<String, String>>,
    pub annotations: Arc<HashMap<String, String>>,
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Secret")
            .field("name", &self.name)
            .field("namespace", &self.namespace)
            .field("properties", &"[REDACTED]")
            .field("labels", &self.labels)
            .finish()
    }
}

/// Provider is the single source of truth for the data connector providers
/// recognized by Data Connect Hub. Both the connector crates (via their
/// `provider()` methods) and the services (for validation) reference this enum,
/// so the set of known providers cannot drift between them. Adding a connector
/// means adding a variant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Postgres,
    Sqlite,
    S3,
    Milvus,
    Elasticsearch,
    Neo4j,
}

impl Provider {
    /// ALL lists every known provider, in declaration order.
    pub const ALL: &'static [Provider] = &[
        Provider::Postgres,
        Provider::Sqlite,
        Provider::S3,
        Provider::Milvus,
        Provider::Elasticsearch,
        Provider::Neo4j,
    ];

    /// as_str returns the canonical wire identifier for the provider.
    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::Postgres => "postgres",
            Provider::Sqlite => "sqlite",
            Provider::S3 => "s3",
            Provider::Milvus => "milvus",
            Provider::Elasticsearch => "elasticsearch",
            Provider::Neo4j => "neo4j",
        }
    }

    /// from_id parses a provider identifier, returning `None` when it does not
    /// match a known provider.
    pub fn from_id(id: &str) -> Option<Provider> {
        Provider::ALL.iter().copied().find(|p| p.as_str() == id)
    }
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DataConnectionType {
    pub name: String,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub credentials_fields: Vec<Field>,
}

impl DataConnectionType {
    pub fn validate(&self) -> Result<(), MetaStoreError> {
        if self.name.is_empty() {
            return Err(MetaStoreError::Validation("name must not be empty".to_string()));
        }
        if self.name.len() > MAX_NAME_LENGTH {
            return Err(MetaStoreError::Validation(format!(
                "name '{}' exceeds maximum length of {MAX_NAME_LENGTH}",
                self.name
            )));
        }
        if Provider::from_id(&self.provider).is_none() {
            let supported = Provider::ALL.iter().map(|p| p.as_str()).collect::<Vec<_>>().join(", ");
            return Err(MetaStoreError::UnsupportedProvider(format!(
                "unsupported provider '{}'; supported providers are: {supported}",
                self.provider
            )));
        }
        if let Some(desc) = &self.description
            && desc.len() > MAX_DESCRIPTION_LENGTH
        {
            return Err(MetaStoreError::Validation(format!(
                "description exceeds maximum length of {MAX_DESCRIPTION_LENGTH}"
            )));
        }
        for field in &self.credentials_fields {
            field.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct Capabilities {
    pub flight: bool,
    pub rest: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct DataConnectionTypeStatus {
    pub capabilities: Capabilities,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DataConnectionTypeResource {
    pub metadata: ResourceMetadata,
    pub resource: DataConnectionType,
    #[serde(default)]
    pub status: DataConnectionTypeStatus,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_data_connection_type_resource() -> DataConnectionTypeResource {
        DataConnectionTypeResource {
            metadata: ResourceMetadata {
                id: "dct-001".to_string(),
                tenant_id: Some("tenant-1".to_string()),
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
            status: DataConnectionTypeStatus::default(),
        }
    }

    #[test]
    fn test_provider_from_id_roundtrip() {
        for provider in Provider::ALL {
            assert_eq!(Provider::from_id(provider.as_str()), Some(*provider));
        }
        assert_eq!(Provider::from_id("mysql"), None);
        assert_eq!(Provider::from_id(""), None);
    }

    #[test]
    fn test_provider_serde_lowercase() {
        assert_eq!(serde_json::to_value(Provider::Postgres).unwrap(), "postgres");
        assert_eq!(
            serde_json::from_value::<Provider>(serde_json::json!("elasticsearch")).unwrap(),
            Provider::Elasticsearch
        );
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

    fn valid_connection_type() -> DataConnectionType {
        DataConnectionType {
            name: "PostgreSQL".to_string(),
            provider: "postgres".to_string(),
            description: Some("A connection".to_string()),
            credentials_fields: vec![Field {
                name: "url".to_string(),
                label: "URL".to_string(),
                description: None,
                required: true,
                d_type: "string".to_string(),
                enum_values: None,
                default_value: None,
            }],
        }
    }

    #[test]
    fn test_validate_valid_connection_type() {
        assert!(valid_connection_type().validate().is_ok());
    }

    #[test]
    fn test_validate_empty_name() {
        let mut ct = valid_connection_type();
        ct.name = "".to_string();
        let err = ct.validate().unwrap_err().to_string();
        assert!(err.contains("name must not be empty"));
    }

    #[test]
    fn test_validate_name_too_long() {
        let mut ct = valid_connection_type();
        ct.name = "a".repeat(254);
        let err = ct.validate().unwrap_err().to_string();
        assert!(err.contains("exceeds maximum length"));
    }

    #[test]
    fn test_validate_unsupported_provider() {
        let mut ct = valid_connection_type();
        ct.provider = "postgresql".to_string();
        let err = ct.validate().unwrap_err().to_string();
        assert!(err.contains("unsupported provider 'postgresql'"));
        assert!(err.contains("postgres"));
    }

    #[test]
    fn test_validate_description_too_long() {
        let mut ct = valid_connection_type();
        ct.description = Some("a".repeat(1025));
        let err = ct.validate().unwrap_err().to_string();
        assert!(err.contains("description exceeds maximum length"));
    }

    #[test]
    fn test_validate_field_empty_name() {
        let mut ct = valid_connection_type();
        ct.credentials_fields[0].name = "".to_string();
        let err = ct.validate().unwrap_err().to_string();
        assert!(err.contains("field name must not be empty"));
    }

    #[test]
    fn test_validate_field_invalid_type() {
        let mut ct = valid_connection_type();
        ct.credentials_fields[0].d_type = "integer".to_string();
        let err = ct.validate().unwrap_err().to_string();
        assert!(err.contains("type 'integer' is not valid"));
    }

    #[test]
    fn test_validate_enum_field_missing_values() {
        let mut ct = valid_connection_type();
        ct.credentials_fields[0].d_type = "enum".to_string();
        ct.credentials_fields[0].enum_values = None;
        let err = ct.validate().unwrap_err().to_string();
        assert!(err.contains("enum_values must be provided"));
    }

    #[test]
    fn test_validate_enum_field_empty_values() {
        let mut ct = valid_connection_type();
        ct.credentials_fields[0].d_type = "enum".to_string();
        ct.credentials_fields[0].enum_values = Some(vec![]);
        let err = ct.validate().unwrap_err().to_string();
        assert!(err.contains("enum_values must not be empty"));
    }

    #[test]
    fn test_validate_enum_field_valid() {
        let mut ct = valid_connection_type();
        ct.credentials_fields[0].d_type = "enum".to_string();
        ct.credentials_fields[0].enum_values = Some(vec![EnumValue {
            value: "a".to_string(),
            label: "A".to_string(),
        }]);
        assert!(ct.validate().is_ok());
    }

    #[test]
    fn test_validate_all_providers() {
        for p in Provider::ALL {
            let mut ct = valid_connection_type();
            ct.provider = p.as_str().to_string();
            assert!(ct.validate().is_ok(), "provider '{}' should be valid", p.as_str());
        }
    }
}
