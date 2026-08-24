use commons::api::errors::ConnectorError;

#[derive(Debug, Clone)]
pub struct UriRequest {
    pub path: String,
    pub data_path: Option<String>,
}

impl UriRequest {
    pub fn parse(query: &str) -> Result<Self, ConnectorError> {
        let value: serde_json::Value =
            serde_json::from_str(query).map_err(|e| ConnectorError::InvalidRequest(format!("Invalid JSON: {e}")))?;

        let obj = value
            .as_object()
            .ok_or_else(|| ConnectorError::InvalidRequest("Query must be a JSON object".to_string()))?;

        let raw_path = obj
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ConnectorError::InvalidRequest("'path' is required".to_string()))?;

        let path = raw_path.trim_start_matches('/').to_string();

        if let Some(method) = obj.get("method").and_then(|v| v.as_str())
            && !method.eq_ignore_ascii_case("GET")
        {
            return Err(ConnectorError::InvalidRequest(format!(
                "Only GET is supported, got '{method}'"
            )));
        }

        let data_path = obj.get("data_path").and_then(|v| v.as_str()).map(String::from);

        Ok(Self { path, data_path })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal() {
        let req = UriRequest::parse(r#"{"path": "/api/data"}"#).unwrap();
        assert_eq!(req.path, "api/data");
        assert!(req.data_path.is_none());
    }

    #[test]
    fn test_parse_strips_leading_slash() {
        let req = UriRequest::parse(r#"{"path": "/api/data"}"#).unwrap();
        assert_eq!(req.path, "api/data");
    }

    #[test]
    fn test_parse_relative_path() {
        let req = UriRequest::parse(r#"{"path": "api/data"}"#).unwrap();
        assert_eq!(req.path, "api/data");
    }

    #[test]
    fn test_parse_with_data_path() {
        let req = UriRequest::parse(r#"{"path": "/api/search", "data_path": "results.items"}"#).unwrap();
        assert_eq!(req.path, "api/search");
        assert_eq!(req.data_path.as_deref(), Some("results.items"));
    }

    #[test]
    fn test_parse_explicit_get() {
        let req = UriRequest::parse(r#"{"path": "/api/data", "method": "GET"}"#).unwrap();
        assert_eq!(req.path, "api/data");
    }

    #[test]
    fn test_parse_rejects_post() {
        let err = UriRequest::parse(r#"{"path": "/api", "method": "POST"}"#).unwrap_err();
        assert!(err.to_string().contains("Only GET is supported"));
    }

    #[test]
    fn test_parse_rejects_put() {
        let err = UriRequest::parse(r#"{"path": "/api", "method": "PUT"}"#).unwrap_err();
        assert!(err.to_string().contains("Only GET is supported"));
    }

    #[test]
    fn test_parse_accepts_any_path_string() {
        let req = UriRequest::parse(r#"{"path": "some/path"}"#).unwrap();
        assert_eq!(req.path, "some/path");
    }

    #[test]
    fn test_parse_missing_path() {
        let err = UriRequest::parse(r#"{"data_path": "x"}"#).unwrap_err();
        assert!(err.to_string().contains("path"));
    }

    #[test]
    fn test_parse_invalid_json() {
        let err = UriRequest::parse("not json").unwrap_err();
        assert!(err.to_string().contains("Invalid JSON"));
    }

    #[test]
    fn test_parse_not_object() {
        let err = UriRequest::parse("[1,2,3]").unwrap_err();
        assert!(err.to_string().contains("JSON object"));
    }
}
