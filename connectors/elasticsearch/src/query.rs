use commons::api::errors::ConnectorError;

#[derive(Debug, Clone)]
pub struct EsRequestInput {
    pub index: Option<String>,
    pub body: serde_json::Value,
    pub size: Option<u64>,
    pub has_user_sort: bool,
}

impl EsRequestInput {
    pub fn parse(query: &str) -> Result<Self, ConnectorError> {
        let mut value: serde_json::Value =
            serde_json::from_str(query).map_err(|e| ConnectorError::InvalidRequest(format!("Invalid JSON: {e}")))?;

        let obj = value
            .as_object_mut()
            .ok_or_else(|| ConnectorError::InvalidRequest("Query must be a JSON object".to_string()))?;

        let index = obj.remove("index").and_then(|v| v.as_str().map(String::from));

        if obj.get("_source") == Some(&serde_json::Value::Bool(false)) {
            return Err(ConnectorError::InvalidRequest(
                "\"_source\": false is not supported; this connector reads data from _source".to_string(),
            ));
        }

        let size = obj.get("size").and_then(|v| v.as_u64());
        let has_user_sort = obj.contains_key("sort");

        Ok(Self {
            index,
            body: value,
            size,
            has_user_sort,
        })
    }

    pub fn resolve_index(&self, default_index: Option<&str>) -> Result<String, ConnectorError> {
        self.index
            .clone()
            .or_else(|| default_index.map(String::from))
            .ok_or_else(|| {
                ConnectorError::InvalidRequest(
                    "No index specified: set 'index' in the query or configure a default index in connection properties"
                        .to_string(),
                )
            })
    }

    pub fn build_pit_search_body(
        &self,
        pit_id: &str,
        page_size: u64,
        search_after: Option<&serde_json::Value>,
    ) -> serde_json::Value {
        let mut body = self.body.clone();
        let obj = body.as_object_mut().unwrap();

        obj.insert(
            "pit".to_string(),
            serde_json::json!({ "id": pit_id, "keep_alive": "5m" }),
        );
        obj.insert("size".to_string(), serde_json::json!(page_size));

        if !self.has_user_sort {
            obj.insert("sort".to_string(), serde_json::json!([{"_shard_doc": "asc"}]));
        } else if let Some(serde_json::Value::Array(sort_arr)) = obj.get_mut("sort") {
            let has_shard_doc = sort_arr.iter().any(|s| {
                s.as_object().is_some_and(|o| o.contains_key("_shard_doc"))
                    || s.as_str().is_some_and(|s| s == "_shard_doc")
            });
            if !has_shard_doc {
                sort_arr.push(serde_json::json!({"_shard_doc": "asc"}));
            }
        }

        if let Some(sa) = search_after {
            obj.insert("search_after".to_string(), sa.clone());
        }

        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_with_index() {
        let input = r#"{"index": "my-index", "query": {"match_all": {}}}"#;
        let req = EsRequestInput::parse(input).unwrap();
        assert_eq!(req.index, Some("my-index".to_string()));
        assert!(req.size.is_none());
        assert!(!req.has_user_sort);
        assert!(req.body.get("index").is_none());
        assert!(req.body.get("query").is_some());
    }

    #[test]
    fn test_parse_without_index() {
        let input = r#"{"query": {"match_all": {}}}"#;
        let req = EsRequestInput::parse(input).unwrap();
        assert!(req.index.is_none());
        assert!(req.body.get("query").is_some());
    }

    #[test]
    fn test_parse_with_size_and_sort() {
        let input = r#"{"index": "idx", "size": 500, "sort": [{"date": "desc"}]}"#;
        let req = EsRequestInput::parse(input).unwrap();
        assert_eq!(req.size, Some(500));
        assert!(req.has_user_sort);
    }

    #[test]
    fn test_parse_source_false_rejected() {
        let input = r#"{"index": "idx", "_source": false}"#;
        let err = EsRequestInput::parse(input).unwrap_err();
        assert!(err.to_string().contains("_source"));
    }

    #[test]
    fn test_resolve_index_from_query() {
        let input = r#"{"index": "my-index", "query": {"match_all": {}}}"#;
        let req = EsRequestInput::parse(input).unwrap();
        assert_eq!(req.resolve_index(Some("default")).unwrap(), "my-index");
    }

    #[test]
    fn test_resolve_index_from_default() {
        let input = r#"{"query": {"match_all": {}}}"#;
        let req = EsRequestInput::parse(input).unwrap();
        assert_eq!(req.resolve_index(Some("default-index")).unwrap(), "default-index");
    }

    #[test]
    fn test_resolve_index_missing() {
        let input = r#"{"query": {"match_all": {}}}"#;
        let req = EsRequestInput::parse(input).unwrap();
        assert!(req.resolve_index(None).is_err());
    }

    #[test]
    fn test_parse_invalid_json() {
        let err = EsRequestInput::parse("not json").unwrap_err();
        assert!(err.to_string().contains("Invalid JSON"));
    }

    #[test]
    fn test_build_pit_search_body_no_user_sort() {
        let input = r#"{"index": "idx", "query": {"match_all": {}}}"#;
        let req = EsRequestInput::parse(input).unwrap();
        let body = req.build_pit_search_body("pit-123", 100, None);

        assert_eq!(body["pit"]["id"], "pit-123");
        assert_eq!(body["size"], 100);
        assert_eq!(body["sort"], serde_json::json!([{"_shard_doc": "asc"}]));
        assert!(body.get("search_after").is_none());
    }

    #[test]
    fn test_build_pit_search_body_with_user_sort() {
        let input = r#"{"index": "idx", "sort": [{"date": "desc"}]}"#;
        let req = EsRequestInput::parse(input).unwrap();
        let body = req.build_pit_search_body("pit-123", 50, None);

        let sort = body["sort"].as_array().unwrap();
        assert_eq!(sort.len(), 2);
        assert_eq!(sort[0], serde_json::json!({"date": "desc"}));
        assert_eq!(sort[1], serde_json::json!({"_shard_doc": "asc"}));
    }

    #[test]
    fn test_build_pit_search_body_with_search_after() {
        let input = r#"{"index": "idx"}"#;
        let req = EsRequestInput::parse(input).unwrap();
        let sa = serde_json::json!([1234, 5]);
        let body = req.build_pit_search_body("pit-123", 100, Some(&sa));

        assert_eq!(body["search_after"], serde_json::json!([1234, 5]));
    }

    #[test]
    fn test_build_pit_search_body_shard_doc_not_duplicated() {
        let input = r#"{"index": "idx", "sort": [{"_shard_doc": "asc"}]}"#;
        let req = EsRequestInput::parse(input).unwrap();
        let body = req.build_pit_search_body("pit-123", 100, None);

        let sort = body["sort"].as_array().unwrap();
        assert_eq!(sort.len(), 1);
    }
}
