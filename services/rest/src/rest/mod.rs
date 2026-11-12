pub mod access_log;
pub mod endpoints;
pub mod errors;
pub mod metrics;
pub mod middleware;

pub const API_VERSION: &str = "v1alpha1";

use commons::api::connections::CredentialsRef;
use commons::api::connections::DataConnection;
use commons::api::connections::DataFormat;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Deserialize, Serialize, Clone)]
pub struct InlineCreds {
    pub secret: String,
    pub properties: HashMap<String, String>,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct DataConnectionWithCreds {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub data_connection_type_id: String,
    pub format: DataFormat,
    pub credentials: InlineCreds,
    pub properties: HashMap<String, String>,
}

impl DataConnectionWithCreds {
    pub fn to_data_connection(&self) -> DataConnection {
        DataConnection {
            name: self.name.clone(),
            description: self.description.clone(),
            data_connection_type_id: self.data_connection_type_id.clone(),
            format: self.format.clone(),
            credentials_ref: CredentialsRef {
                secret: self.credentials.secret.clone(),
            },
            properties: self.properties.clone(),
        }
    }
}

#[derive(Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub(crate) enum CreateConnectionRequest {
    DataConnectionWithSecretRef(DataConnection),
    DataConnectionWithInlineCreds(DataConnectionWithCreds),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inline_creds_request_carries_description() {
        let req: CreateConnectionRequest = serde_json::from_value(serde_json::json!({
            "name": "my-pg",
            "description": "Sales warehouse",
            "data_connection_type_id": "ct-1",
            "format": "tabular",
            "credentials": {"secret": "my-pg-creds", "properties": {"password": "x"}},
            "properties": {}
        }))
        .unwrap();
        let CreateConnectionRequest::DataConnectionWithInlineCreds(dc_creds) = req else {
            panic!("expected inline credentials variant");
        };
        let conn = dc_creds.to_data_connection();
        assert_eq!(conn.description.as_deref(), Some("Sales warehouse"));
        assert_eq!(conn.credentials_ref.secret, "my-pg-creds");
    }

    #[test]
    fn test_secret_ref_request_without_description() {
        let req: CreateConnectionRequest = serde_json::from_value(serde_json::json!({
            "name": "my-pg",
            "data_connection_type_id": "ct-1",
            "format": "tabular",
            "credentials_ref": {"secret": "my-pg-creds"},
            "properties": {}
        }))
        .unwrap();
        let CreateConnectionRequest::DataConnectionWithSecretRef(conn) = req else {
            panic!("expected secret ref variant");
        };
        assert!(conn.description.is_none());
    }
}
