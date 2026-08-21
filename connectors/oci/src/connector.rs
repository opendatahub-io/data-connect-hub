use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::auth;
use crate::format::{self, Compression, FileFormat};
use commons::api::connection_types::Provider;
use commons::api::connections::{Admin, DataConnectionResource};
use commons::api::errors::ConnectorError;
use commons::api::tabular::{FlightConnector, QueryOptions, QueryOutput, TabularReader, TabularState};
use futures::TryStreamExt;
use moka::future::Cache;
use oci_client::client::ClientConfig;
use oci_client::manifest::{OciDescriptor, OciManifest};
use oci_client::secrets::RegistryAuth;
use oci_client::{Client, Reference};

const KEY_HOST: &str = "OCI_HOST";
const KEY_DOCKER_CONFIG: &str = ".dockerconfigjson";
const KEY_INSECURE: &str = "OCI_INSECURE";
const KEY_CA_CERT: &str = "OCI_CA_CERT";

const CONFIG_MEDIA_TYPE: &str = "application/vnd.oci.image.config.v1+json";

pub struct OciConnector {
    clients: Cache<String, Client>,
}

impl OciConnector {
    pub fn new(cache_ttl: Duration, cache_idle: Duration, cache_max_capacity: u64) -> Self {
        Self {
            clients: Cache::builder()
                .time_to_live(cache_ttl)
                .time_to_idle(cache_idle)
                .max_capacity(cache_max_capacity)
                .build(),
        }
    }
}

fn extract_credentials(
    data_connection: &DataConnectionResource,
) -> Result<Arc<HashMap<String, String>>, ConnectorError> {
    match &data_connection.resource.admin {
        Some(Admin::Secret { name: _, secret }) => Ok(secret.clone()),
        _ => Err(ConnectorError::ConnectionError(
            "OCI credentials are required".to_string(),
        )),
    }
}

fn get_registry_host(credentials: &HashMap<String, String>) -> Result<String, ConnectorError> {
    credentials
        .get(KEY_HOST)
        .filter(|h| !h.is_empty())
        .cloned()
        .ok_or_else(|| ConnectorError::ConfigError(format!("{KEY_HOST} is required")))
}

fn get_registry_auth(
    credentials: &HashMap<String, String>,
    registry_host: &str,
) -> Result<RegistryAuth, ConnectorError> {
    match credentials.get(KEY_DOCKER_CONFIG) {
        Some(docker_config) if !docker_config.is_empty() => auth::parse_docker_config(docker_config, registry_host),
        _ => Ok(RegistryAuth::Anonymous),
    }
}

fn parse_reference(registry_host: &str, query: &str) -> Result<Reference, ConnectorError> {
    let host_prefix = format!("{registry_host}/");
    let full_ref = if query.starts_with(&host_prefix) {
        query.to_string()
    } else {
        format!("{registry_host}/{query}")
    };

    full_ref
        .parse::<Reference>()
        .map_err(|e| ConnectorError::InvalidRequest(format!("Invalid OCI reference '{full_ref}': {e}")))
}

fn find_data_layer(layers: &[OciDescriptor]) -> Result<&OciDescriptor, ConnectorError> {
    layers
        .iter()
        .find(|l| l.media_type != CONFIG_MEDIA_TYPE)
        .ok_or(ConnectorError::NoDataError)
}

#[async_trait::async_trait]
impl FlightConnector for OciConnector {
    fn provider(&self) -> String {
        Provider::Oci.as_str().to_string()
    }

    fn description(&self) -> String {
        "OCI compliant registry connector".to_string()
    }

    async fn get_reader(
        &self,
        data_connection: &DataConnectionResource,
    ) -> Result<Arc<dyn TabularReader>, ConnectorError> {
        let credentials = extract_credentials(data_connection)?;
        let registry_host = get_registry_host(&credentials)?;
        let registry_auth = get_registry_auth(&credentials, &registry_host)?;

        let insecure = credentials
            .get(KEY_INSECURE)
            .is_some_and(|v| v.eq_ignore_ascii_case("true") || v == "1");

        let protocol = if insecure {
            oci_client::client::ClientProtocol::Http
        } else {
            oci_client::client::ClientProtocol::Https
        };

        let mut extra_root_certificates = Vec::new();
        if let Some(ca_pem) = credentials.get(KEY_CA_CERT).filter(|v| !v.is_empty()) {
            extra_root_certificates.push(oci_client::client::Certificate {
                encoding: oci_client::client::CertificateEncoding::Pem,
                data: ca_pem.as_bytes().to_vec(),
            });
        }

        let client = self
            .clients
            .try_get_with_by_ref(&data_connection.metadata.id, async {
                Ok::<_, ConnectorError>(Client::new(ClientConfig {
                    protocol,
                    extra_root_certificates,
                    ..Default::default()
                }))
            })
            .await
            .map_err(|e| ConnectorError::ConnectionError(format!("Failed to create OCI client: {e}")))?;

        let format_hint = data_connection.resource.properties.get("format").cloned();

        Ok(Arc::new(OciReader {
            client,
            registry_host,
            registry_auth,
            format_hint,
        }))
    }
}

pub struct OciReader {
    client: Client,
    registry_host: String,
    registry_auth: RegistryAuth,
    format_hint: Option<String>,
}

impl OciReader {
    async fn pull_manifest(
        &self,
        reference: &Reference,
    ) -> Result<(Vec<OciDescriptor>, Option<OciDescriptor>), ConnectorError> {
        let (manifest, _digest) = self
            .client
            .pull_manifest(reference, &self.registry_auth)
            .await
            .map_err(|e| ConnectorError::ConnectionError(format!("Failed to pull manifest: {e}")))?;

        match manifest {
            OciManifest::Image(image) => Ok((image.layers, Some(image.config))),
            OciManifest::ImageIndex(_) => Err(ConnectorError::InvalidRequest(
                "Image index manifests are not supported; specify a platform-specific reference".to_string(),
            )),
        }
    }

    fn detect_format(&self, reference: &Reference, layer: &OciDescriptor) -> FileFormat {
        FileFormat::detect(
            reference.repository(),
            self.format_hint.as_deref(),
            Some(&layer.media_type),
        )
    }

    fn detect_compression(&self, layer: &OciDescriptor) -> Compression {
        Compression::detect(Some(&layer.media_type), self.format_hint.as_deref())
    }

    async fn pull_blob_stream(
        &self,
        reference: &Reference,
        layer: &OciDescriptor,
    ) -> Result<format::ByteStream, ConnectorError> {
        let sized_stream = self
            .client
            .pull_blob_stream(reference, layer)
            .await
            .map_err(|e| ConnectorError::IOError(format!("Failed to pull blob stream: {e}")))?;

        let stream = sized_stream.stream.map_ok(|b| b);
        Ok(Box::pin(stream))
    }
}

#[async_trait::async_trait]
impl TabularReader for OciReader {
    fn provider(&self) -> String {
        "oci".to_string()
    }

    async fn schema(&self, query: &str) -> Result<Arc<TabularState>, ConnectorError> {
        let reference = parse_reference(&self.registry_host, query)?;
        let (layers, _config) = self.pull_manifest(&reference).await?;
        let layer = find_data_layer(&layers)?;
        let format = self.detect_format(&reference, layer);
        let compression = self.detect_compression(layer);

        let schema = match format {
            FileFormat::Parquet if compression != Compression::None => {
                return Err(ConnectorError::InvalidRequest(
                    "Externally compressed parquet is not supported; parquet has built-in compression".to_string(),
                ));
            },
            FileFormat::Parquet => {
                let oci_reader = format::parquet::OciAsyncReader::new(self.client.clone(), reference, layer.clone());
                format::read_parquet_schema(oci_reader).await?
            },
            FileFormat::Csv => {
                let stream = self.pull_blob_stream(&reference, layer).await?;
                let stream = format::decompress_stream(stream, compression);
                format::read_csv_schema(stream).await?
            },
            FileFormat::JsonLines => {
                let stream = self.pull_blob_stream(&reference, layer).await?;
                let stream = format::decompress_stream(stream, compression);
                format::read_jsonl_schema(stream).await?
            },
            FileFormat::Binary => format::binary_schema(),
        };

        Ok(Arc::new(TabularState::new(query.to_owned(), Arc::new(schema))))
    }

    async fn read(&self, state: Arc<TabularState>, options: &QueryOptions) -> QueryOutput {
        let reference = parse_reference(&self.registry_host, &state.query)?;
        let (layers, _config) = self.pull_manifest(&reference).await?;
        let layer = find_data_layer(&layers)?;
        let format = self.detect_format(&reference, layer);
        let compression = self.detect_compression(layer);
        let batch_size = options.batch_size;

        match format {
            FileFormat::Parquet if compression != Compression::None => Err(ConnectorError::InvalidRequest(
                "Externally compressed parquet is not supported; parquet has built-in compression".to_string(),
            )),
            FileFormat::Parquet => {
                let oci_reader = format::parquet::OciAsyncReader::new(self.client.clone(), reference, layer.clone());
                format::read_parquet_batches(oci_reader, batch_size).await
            },
            FileFormat::Csv => {
                let stream = self.pull_blob_stream(&reference, layer).await?;
                let stream = format::decompress_stream(stream, compression);
                format::read_csv_batches(stream, &state.schema, batch_size).await
            },
            FileFormat::JsonLines => {
                let stream = self.pull_blob_stream(&reference, layer).await?;
                let stream = format::decompress_stream(stream, compression);
                format::read_jsonl_batches(stream, &state.schema, batch_size).await
            },
            FileFormat::Binary => {
                let stream = self.pull_blob_stream(&reference, layer).await?;
                let stream = format::decompress_stream(stream, compression);
                format::read_binary_batches(stream, batch_size).await
            },
        }
    }

    async fn test_connection(&self) -> Result<(), ConnectorError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commons::api::ResourceMetadata;
    use commons::api::connections::{DataConnection, DataFormat};

    fn make_credentials() -> Arc<HashMap<String, String>> {
        Arc::new(HashMap::from([
            (KEY_HOST.to_string(), "registry.example.com".to_string()),
            (
                KEY_DOCKER_CONFIG.to_string(),
                r#"{"auths":{"registry.example.com":{"auth":"dXNlcjpwYXNz"}}}"#.to_string(),
            ),
        ]))
    }

    fn make_connection(credentials: Arc<HashMap<String, String>>) -> DataConnectionResource {
        DataConnectionResource {
            metadata: ResourceMetadata {
                id: "conn-oci-1".to_string(),
                tenant_id: Some("tenant-1".to_string()),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
            resource: DataConnection {
                name: "test-oci".to_string(),
                data_connection_type_id: "oci-type".to_string(),
                format: DataFormat::Tabular,
                admin: Some(Admin::Secret {
                    name: "test-oci".to_string(),
                    secret: credentials,
                }),
                properties: HashMap::new(),
            },
            status: Default::default(),
        }
    }

    #[test]
    fn test_oci_connector_provider() {
        let connector = OciConnector::new(Duration::from_secs(300), Duration::from_secs(60), 100);
        assert_eq!(connector.provider(), "oci");
    }

    #[test]
    fn test_extract_credentials_success() {
        let creds = make_credentials();
        let conn = make_connection(creds.clone());
        let result = extract_credentials(&conn);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().get(KEY_HOST).unwrap(), "registry.example.com");
    }

    #[test]
    fn test_extract_credentials_missing() {
        let mut conn = make_connection(make_credentials());
        conn.resource.admin = None;
        let result = extract_credentials(&conn);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_registry_host() {
        let creds = make_credentials();
        let host = get_registry_host(&creds).unwrap();
        assert_eq!(host, "registry.example.com");
    }

    #[test]
    fn test_get_registry_host_missing() {
        let creds = HashMap::new();
        let result = get_registry_host(&creds);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_registry_auth() {
        let creds = make_credentials();
        let auth = get_registry_auth(&creds, "registry.example.com").unwrap();
        assert_eq!(auth, RegistryAuth::Basic("user".into(), "pass".into()));
    }

    #[test]
    fn test_get_registry_auth_anonymous() {
        let creds = HashMap::from([(KEY_HOST.to_string(), "reg.io".to_string())]);
        let auth = get_registry_auth(&creds, "reg.io").unwrap();
        assert_eq!(auth, RegistryAuth::Anonymous);
    }

    #[test]
    fn test_parse_reference() {
        let reference = parse_reference("registry.example.com", "myrepo:v1").unwrap();
        assert_eq!(reference.registry(), "registry.example.com");
        assert_eq!(reference.repository(), "myrepo");
        assert_eq!(reference.tag(), Some("v1"));
    }

    #[test]
    fn test_parse_reference_with_namespace() {
        let reference = parse_reference("registry.example.com", "org/myrepo:latest").unwrap();
        assert_eq!(reference.registry(), "registry.example.com");
        assert_eq!(reference.repository(), "org/myrepo");
        assert_eq!(reference.tag(), Some("latest"));
    }

    #[test]
    fn test_parse_reference_full() {
        let reference = parse_reference("registry.example.com", "registry.example.com/myrepo:v2").unwrap();
        assert_eq!(reference.registry(), "registry.example.com");
        assert_eq!(reference.repository(), "myrepo");
    }

    #[test]
    fn test_parse_reference_rejects_host_prefix_attack() {
        let reference = parse_reference("registry.example.com", "registry.example.com.evil.tld/repo:v1").unwrap();
        // Should prepend the real host, not treat the malicious host as a match
        assert_eq!(reference.registry(), "registry.example.com");
        assert!(reference.repository().contains("registry.example.com.evil.tld"));
    }

    #[test]
    fn test_find_data_layer() {
        let layers = vec![
            OciDescriptor {
                media_type: CONFIG_MEDIA_TYPE.to_string(),
                digest: "sha256:config".to_string(),
                size: 100,
                ..Default::default()
            },
            OciDescriptor {
                media_type: "application/vnd.apache.parquet".to_string(),
                digest: "sha256:data".to_string(),
                size: 50000,
                ..Default::default()
            },
        ];
        let layer = find_data_layer(&layers).unwrap();
        assert_eq!(layer.digest, "sha256:data");
    }

    #[test]
    fn test_find_data_layer_empty() {
        let layers: Vec<OciDescriptor> = vec![];
        let result = find_data_layer(&layers);
        assert!(result.is_err());
    }

    #[test]
    fn test_detect_format() {
        let reader = OciReader {
            client: Client::new(ClientConfig::default()),
            registry_host: "reg.io".to_string(),
            registry_auth: RegistryAuth::Anonymous,
            format_hint: None,
        };

        let layer = OciDescriptor {
            media_type: "application/vnd.apache.parquet".to_string(),
            ..Default::default()
        };
        let reference: Reference = "reg.io/data:v1".parse().unwrap();
        assert_eq!(reader.detect_format(&reference, &layer), FileFormat::Parquet);

        let layer = OciDescriptor {
            media_type: "application/octet-stream".to_string(),
            ..Default::default()
        };
        let ref_csv: Reference = "reg.io/data.csv:latest".parse().unwrap();
        assert_eq!(reader.detect_format(&ref_csv, &layer), FileFormat::Csv);
        let ref_no_ext: Reference = "reg.io/data:v1".parse().unwrap();
        assert_eq!(reader.detect_format(&ref_no_ext, &layer), FileFormat::Binary);
    }

    #[test]
    fn test_detect_format_with_hint() {
        let reader = OciReader {
            client: Client::new(ClientConfig::default()),
            registry_host: "reg.io".to_string(),
            registry_auth: RegistryAuth::Anonymous,
            format_hint: Some("csv".to_string()),
        };

        let layer = OciDescriptor {
            media_type: "application/octet-stream".to_string(),
            ..Default::default()
        };
        let reference: Reference = "reg.io/data:v1".parse().unwrap();
        assert_eq!(reader.detect_format(&reference, &layer), FileFormat::Csv);
    }
}
