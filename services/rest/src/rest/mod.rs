pub mod endpoints;
pub mod errors;
pub mod middleware;

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
    pub data_connection_type_id: String,
    pub format: DataFormat,
    pub credentials: InlineCreds,
    pub properties: HashMap<String, String>,
}

impl DataConnectionWithCreds {
    pub fn to_data_connection(&self) -> DataConnection {
        DataConnection {
            name: self.name.clone(),
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
