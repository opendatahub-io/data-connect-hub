use anyhow::Result;
use commons::api::connections::{DataConnectionType, MetaStore};
use std::fs;
use std::sync::Arc;

fn load_default_connection_types(folder: &str) -> Result<Vec<DataConnectionType>> {
    let mut types = Vec::new();
    for entry in fs::read_dir(folder)? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "yaml" || ext == "yml") {
            let content = fs::read_to_string(&path)?;
            let ct: DataConnectionType = serde_yaml::from_str(&content)?;
            types.push(ct);
        }
    }
    Ok(types)
}

pub async fn sync_default_connection_types(meta_store: &Arc<dyn MetaStore + Send + Sync>, folder: &str) -> Result<()> {
    let defaults = load_default_connection_types(folder)?;
    tracing::info!("Loaded {} default connection types", defaults.len());

    let existing = meta_store.get_data_connection_types("").await?;
    let existing_by_name: std::collections::HashMap<&str, &str> = existing
        .items
        .iter()
        .map(|r| (r.resource.name.as_str(), r.metadata.id.as_str()))
        .collect();

    for ct in &defaults {
        match existing_by_name.get(ct.name.as_str()) {
            Some(&uid) => {
                tracing::info!("Updating default connection type: {}", ct.name);
                let ct = ct.clone();
                meta_store
                    .update_data_connection_type("", uid, Arc::new(move |_| Ok(ct.clone())))
                    .await?;
            },
            None => {
                tracing::info!("Creating default connection type: {}", ct.name);
                meta_store.create_data_connection_type("", ct).await?;
            },
        }
    }
    Ok(())
}
