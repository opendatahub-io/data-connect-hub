use commons::api::errors::ConnectorError;

#[derive(Debug)]
pub enum MilvusOperation {
    Query,
    Search,
    Get,
}

pub struct MilvusRequestInput {
    pub collection_name: String,
    pub operation: MilvusOperation,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub body: serde_json::Value,
}

impl MilvusRequestInput {
    pub fn parse(query: &str) -> Result<Self, ConnectorError> {
        let value: serde_json::Value = serde_json::from_str(query)
            .map_err(|e| ConnectorError::InvalidRequest(format!("Invalid Milvus JSON query: {e}")))?;

        let obj = value
            .as_object()
            .ok_or_else(|| ConnectorError::InvalidRequest("Query must be a JSON object".to_string()))?;

        let collection_name = obj
            .get("collectionName")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ConnectorError::InvalidRequest("'collectionName' is required".to_string()))?
            .to_string();

        let operation = if obj.contains_key("data") {
            MilvusOperation::Search
        } else if obj.contains_key("id") {
            MilvusOperation::Get
        } else {
            MilvusOperation::Query
        };

        let limit = obj.get("limit").and_then(|v| v.as_i64());
        let offset = obj.get("offset").and_then(|v| v.as_i64());

        Ok(Self {
            collection_name,
            operation,
            limit,
            offset,
            body: value,
        })
    }

    pub fn output_fields(&self) -> Option<Vec<String>> {
        self.body
            .get("outputFields")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_query() {
        let json = r#"{"collectionName":"products","filter":"price > 50","outputFields":["id","name"],"limit":100}"#;
        let req = MilvusRequestInput::parse(json).unwrap();
        assert_eq!(req.collection_name, "products");
        assert_eq!(req.limit, Some(100));
        assert!(matches!(req.operation, MilvusOperation::Query));
        assert_eq!(req.output_fields().unwrap(), vec!["id", "name"]);
    }

    #[test]
    fn test_parse_search() {
        let json = r#"{"collectionName":"products","data":[[0.1,0.2,0.3]],"annsField":"embedding","limit":10}"#;
        let req = MilvusRequestInput::parse(json).unwrap();
        assert_eq!(req.collection_name, "products");
        assert!(matches!(req.operation, MilvusOperation::Search));
        assert_eq!(req.limit, Some(10));
    }

    #[test]
    fn test_parse_get() {
        let json = r#"{"collectionName":"products","id":[1,2,3],"outputFields":["id","name"]}"#;
        let req = MilvusRequestInput::parse(json).unwrap();
        assert_eq!(req.collection_name, "products");
        assert!(matches!(req.operation, MilvusOperation::Get));
    }

    #[test]
    fn test_parse_with_offset() {
        let json = r#"{"collectionName":"products","limit":50,"offset":100}"#;
        let req = MilvusRequestInput::parse(json).unwrap();
        assert_eq!(req.limit, Some(50));
        assert_eq!(req.offset, Some(100));
    }

    #[test]
    fn test_parse_invalid_json() {
        assert!(MilvusRequestInput::parse("not json").is_err());
    }

    #[test]
    fn test_parse_missing_collection() {
        assert!(MilvusRequestInput::parse(r#"{"filter":"id > 1"}"#).is_err());
    }

    #[test]
    fn test_parse_not_object() {
        assert!(MilvusRequestInput::parse(r#"[1,2,3]"#).is_err());
    }

    #[test]
    fn test_output_fields_none() {
        let json = r#"{"collectionName":"products"}"#;
        let req = MilvusRequestInput::parse(json).unwrap();
        assert!(req.output_fields().is_none());
    }

    #[test]
    fn test_body_preserved() {
        let json = r#"{"collectionName":"products","filter":"price > 50","customParam":"value"}"#;
        let req = MilvusRequestInput::parse(json).unwrap();
        assert_eq!(req.body["customParam"], "value");
        assert_eq!(req.body["filter"], "price > 50");
    }
}
