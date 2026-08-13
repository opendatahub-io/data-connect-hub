use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct GlobalConnectionTypes {
    #[serde(rename = "tenant-id")]
    pub tenant_id: String,
}

impl GlobalConnectionTypes {
    pub fn new(tenant_id: String) -> Self {
        Self { tenant_id }
    }
}
