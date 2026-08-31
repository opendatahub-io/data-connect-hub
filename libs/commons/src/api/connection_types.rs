use crate::api::ResourceMetadata;
use crate::api::errors::DataConnectionTypeError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DataConnectionType {
    pub name: String,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub credentials_fields: Vec<Field>,
}

impl DataConnectionType {
    pub fn check_credentials_schema(&self, secret: &HashMap<String, String>) -> Result<(), DataConnectionTypeError> {
        for field in &self.credentials_fields {
            if field.required && !secret.contains_key(&field.name) {
                return Err(DataConnectionTypeError::MissingRequiredField(field.name.clone()));
            }
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
