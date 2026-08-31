use std::collections::HashMap;
use std::sync::Arc;

use crate::api::ResourceList;
use crate::api::connection_types::{DataConnectionType, DataConnectionTypeResource, DataConnectionTypeStatus};
use crate::api::connections::DataConnectionStatus;
use crate::api::connections::{DataConnection, DataConnectionResource};
use crate::api::errors::{MetaStoreError, SecretStoreError};
use crate::api::secret::Secret;

/// Persistent store for data connection and data connection type metadata.
#[async_trait::async_trait]
pub trait MetaStore {
    /// Retrieves all data connections for the given tenant.
    async fn get_data_connections(
        &self,
        tenant_id: &str,
    ) -> Result<ResourceList<DataConnectionResource>, MetaStoreError>;

    /// Retrieves a data connection by tenant and unique identifier.
    async fn get_data_connection(&self, tenant_id: &str, uid: &str) -> Result<DataConnectionResource, MetaStoreError>;

    /// Creates a new data connection for the given tenant.
    async fn create_data_connection(
        &self,
        tenant_id: &str,
        data_connection: &DataConnection,
    ) -> Result<DataConnectionResource, MetaStoreError>;

    /// Replaces the data connection identified by `uid` with the provided value.
    async fn update_data_connection(
        &self,
        tenant_id: &str,
        uid: &str,
        update_fn: Arc<dyn Fn(DataConnection) -> Result<DataConnection, MetaStoreError> + Send + Sync>,
    ) -> Result<DataConnectionResource, MetaStoreError>;

    /// Updates the status of the data connection identified by `uid`.
    async fn update_data_connection_status(
        &self,
        tenant_id: &str,
        uid: &str,
        update_fn: Arc<dyn Fn(DataConnectionStatus) -> Result<DataConnectionStatus, MetaStoreError> + Send + Sync>,
    ) -> Result<DataConnectionResource, MetaStoreError>;

    /// Deletes the data connection identified by `uid`.
    async fn delete_data_connection(&self, tenant_id: &str, uid: &str) -> Result<(), MetaStoreError>;

    /// Retrieves all data connection types for the given tenant.
    async fn get_data_connection_types(
        &self,
        tenant_id: &str,
    ) -> Result<ResourceList<DataConnectionTypeResource>, MetaStoreError>;

    /// Retrieves all data connection types. Used internally to audit data connection types.
    async fn get_all_data_connection_types(&self) -> Result<ResourceList<DataConnectionTypeResource>, MetaStoreError>;

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
        data_connection_type: &DataConnectionType,
    ) -> Result<DataConnectionTypeResource, MetaStoreError>;

    /// Replaces the data connection type identified by `uid` with the provided value.
    async fn update_data_connection_type(
        &self,
        tenant_id: &str,
        uid: &str,
        update_fn: Arc<dyn Fn(DataConnectionType) -> Result<DataConnectionType, MetaStoreError> + Send + Sync>,
    ) -> Result<DataConnectionTypeResource, MetaStoreError>;

    /// Updates the status of the data connection type identified by `uid`.
    async fn update_data_connection_type_status(
        &self,
        uid: &str,
        update_fn: Arc<
            dyn Fn(DataConnectionTypeStatus) -> Result<DataConnectionTypeStatus, MetaStoreError> + Send + Sync,
        >,
    ) -> Result<DataConnectionTypeResource, MetaStoreError>;

    /// Deletes the data connection type identified by `uid`.
    async fn delete_data_connection_type(&self, tenant_id: &str, uid: &str) -> Result<(), MetaStoreError>;
}

#[async_trait::async_trait]
pub trait SecretStore {
    async fn get_secret(&self, namespace: &str, name: &str) -> Result<Secret, SecretStoreError>;
    async fn create_secret(&self, secret: &Secret, overwrite: bool) -> Result<(), SecretStoreError>;
    async fn delete_secret(&self, namespace: &str, name: &str) -> Result<(), SecretStoreError>;
    async fn set_secret_labels(
        &self,
        namespace: &str,
        name: &str,
        labels: HashMap<String, String>,
    ) -> Result<(), SecretStoreError>;
}
