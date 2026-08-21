use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use commons::api::errors::ConnectorError;
use oci_client::secrets::RegistryAuth;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
struct DockerConfig {
    auths: HashMap<String, AuthEntry>,
}

#[derive(Deserialize)]
struct AuthEntry {
    auth: Option<String>,
    username: Option<String>,
    password: Option<String>,
}

pub fn parse_docker_config(docker_config_json: &str, registry_host: &str) -> Result<RegistryAuth, ConnectorError> {
    let config: DockerConfig = serde_json::from_str(docker_config_json)
        .map_err(|e| ConnectorError::ConfigError(format!("Invalid .dockerconfigjson: {e}")))?;

    let host_normalized = normalize_host(registry_host);

    for (key, entry) in &config.auths {
        if normalize_host(key) == host_normalized {
            return auth_from_entry(entry);
        }
    }

    Err(ConnectorError::ConfigError(format!(
        "No credentials found for registry '{registry_host}' in .dockerconfigjson"
    )))
}

fn normalize_host(host: &str) -> String {
    let h = host
        .trim_end_matches('/')
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    // Strip path suffix (e.g. "index.docker.io/v1/" → "index.docker.io")
    match h.find('/') {
        Some(pos) => h[..pos].to_lowercase(),
        None => h.to_lowercase(),
    }
}

fn auth_from_entry(entry: &AuthEntry) -> Result<RegistryAuth, ConnectorError> {
    if let Some(auth) = &entry.auth {
        let decoded = BASE64
            .decode(auth)
            .map_err(|e| ConnectorError::ConfigError(format!("Invalid base64 in auth field: {e}")))?;
        let decoded_str = String::from_utf8(decoded)
            .map_err(|e| ConnectorError::ConfigError(format!("Invalid UTF-8 in auth field: {e}")))?;

        let (username, password) = decoded_str
            .split_once(':')
            .ok_or_else(|| ConnectorError::ConfigError("Auth field missing ':' separator".to_string()))?;

        return Ok(RegistryAuth::Basic(username.to_string(), password.to_string()));
    }

    if let (Some(username), Some(password)) = (&entry.username, &entry.password) {
        return Ok(RegistryAuth::Basic(username.clone(), password.clone()));
    }

    Err(ConnectorError::ConfigError(
        "Auth entry has neither 'auth' nor 'username'/'password' fields".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_base64_auth() {
        let json = r#"{"auths":{"registry.example.com":{"auth":"dXNlcjpwYXNz"}}}"#;
        let auth = parse_docker_config(json, "registry.example.com").unwrap();
        assert_eq!(auth, RegistryAuth::Basic("user".into(), "pass".into()));
    }

    #[test]
    fn test_parse_with_https_prefix() {
        let json = r#"{"auths":{"https://registry.example.com":{"auth":"dXNlcjpwYXNz"}}}"#;
        let auth = parse_docker_config(json, "registry.example.com").unwrap();
        assert_eq!(auth, RegistryAuth::Basic("user".into(), "pass".into()));
    }

    #[test]
    fn test_parse_username_password_fields() {
        let json = r#"{"auths":{"reg.io":{"username":"alice","password":"secret"}}}"#;
        let auth = parse_docker_config(json, "reg.io").unwrap();
        assert_eq!(auth, RegistryAuth::Basic("alice".into(), "secret".into()));
    }

    #[test]
    fn test_registry_not_found() {
        let json = r#"{"auths":{"other.io":{"auth":"dXNlcjpwYXNz"}}}"#;
        let result = parse_docker_config(json, "registry.example.com");
        assert!(result.is_err());
    }

    #[test]
    fn test_normalize_trailing_slash() {
        let json = r#"{"auths":{"registry.example.com/":{"auth":"dXNlcjpwYXNz"}}}"#;
        let auth = parse_docker_config(json, "registry.example.com").unwrap();
        assert_eq!(auth, RegistryAuth::Basic("user".into(), "pass".into()));
    }

    #[test]
    fn test_normalize_strips_path() {
        let json = r#"{"auths":{"https://index.docker.io/v1/":{"auth":"dXNlcjpwYXNz"}}}"#;
        let auth = parse_docker_config(json, "index.docker.io").unwrap();
        assert_eq!(auth, RegistryAuth::Basic("user".into(), "pass".into()));
    }

    #[test]
    fn test_normalize_strips_path_no_scheme() {
        let json = r#"{"auths":{"index.docker.io/v2/":{"auth":"dXNlcjpwYXNz"}}}"#;
        let auth = parse_docker_config(json, "index.docker.io").unwrap();
        assert_eq!(auth, RegistryAuth::Basic("user".into(), "pass".into()));
    }
}
