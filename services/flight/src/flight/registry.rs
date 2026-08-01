use commons::api::tabular::FlightConnector;
use commons::errors::ApiError;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
pub struct ConnectorsRegistry {
    pub connectors: HashMap<String, Arc<dyn FlightConnector>>,
}

impl ConnectorsRegistry {
    pub fn new() -> Self {
        Self {
            connectors: HashMap::new(),
        }
    }
    pub fn with_connector(mut self, connector: Arc<dyn FlightConnector>) -> Self {
        self.connectors.insert(connector.provider(), connector);
        self
    }

    pub fn get_connector(&self, provider: &str) -> Result<&Arc<dyn FlightConnector>, ApiError> {
        self.connectors
            .get(provider)
            .ok_or(ApiError::InvalidRequest(format!("Connector not found: {}", provider)))
    }
}
