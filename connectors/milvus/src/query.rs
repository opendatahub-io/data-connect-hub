use commons::api::errors::ConnectorError;
use serde::Deserialize;

// Milvus REST request json format: https://github.com/milvus-io/web-content/tree/master/API_Reference/milvus-restful

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct MilvusRequestInput {
    pub collection_name: String,
    #[serde(default)]
    pub db_name: Option<String>,
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    pub output_fields: Option<Vec<String>>,
    #[serde(default)]
    pub partition_names: Option<Vec<String>>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
    #[serde(default)]
    pub data: Option<Vec<Vec<f32>>>,
    #[serde(default)]
    pub anns_field: Option<String>,
    #[serde(default)]
    pub search_params: Option<SearchParams>,
    #[serde(default)]
    pub grouping_field: Option<String>,
    #[serde(default)]
    pub consistency_level: Option<String>,
    #[serde(default)]
    pub id: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct SearchParams {
    pub metric_type: Option<String>,
    pub params: Option<serde_json::Value>,
}

pub enum MilvusOperation {
    Query,
    Search,
    Get,
}

impl MilvusRequestInput {
    pub fn parse(query: &str) -> Result<Self, ConnectorError> {
        serde_json::from_str(query)
            .map_err(|e| ConnectorError::InvalidRequest(format!("Invalid Milvus JSON query: {e}")))
    }

    pub fn operation(&self) -> MilvusOperation {
        if self.data.is_some() {
            MilvusOperation::Search
        } else if self.id.is_some() {
            MilvusOperation::Get
        } else {
            MilvusOperation::Query
        }
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
        assert_eq!(req.filter.as_deref(), Some("price > 50"));
        assert_eq!(req.output_fields.as_ref().unwrap(), &["id", "name"]);
        assert_eq!(req.limit, Some(100));
        assert!(matches!(req.operation(), MilvusOperation::Query));
    }

    #[test]
    fn test_parse_search() {
        let json = r#"{"collectionName":"products","data":[[0.1,0.2,0.3]],"annsField":"embedding","limit":10}"#;
        let req = MilvusRequestInput::parse(json).unwrap();
        assert_eq!(req.collection_name, "products");
        assert!(req.data.is_some());
        assert_eq!(req.anns_field.as_deref(), Some("embedding"));
        assert!(matches!(req.operation(), MilvusOperation::Search));
    }

    #[test]
    fn test_parse_get() {
        let json = r#"{"collectionName":"products","id":[1,2,3],"outputFields":["id","name"]}"#;
        let req = MilvusRequestInput::parse(json).unwrap();
        assert_eq!(req.collection_name, "products");
        assert!(req.id.is_some());
        assert!(matches!(req.operation(), MilvusOperation::Get));
    }

    #[test]
    fn test_parse_search_with_params() {
        let json = r#"{"collectionName":"col","data":[[0.1]],"annsField":"vec","searchParams":{"metricType":"L2"}}"#;
        let req = MilvusRequestInput::parse(json).unwrap();
        let params = req.search_params.unwrap();
        assert_eq!(params.metric_type.as_deref(), Some("L2"));
    }

    #[test]
    fn test_parse_invalid_json() {
        let result = MilvusRequestInput::parse("not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_missing_collection() {
        let result = MilvusRequestInput::parse(r#"{"filter":"id > 1"}"#);
        assert!(result.is_err());
    }
}
