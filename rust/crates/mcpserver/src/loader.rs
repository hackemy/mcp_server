use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use crate::types::{McpError, Resource, SchemaMeta, SchemaRequirementSet, Tool};

/// Load tool definitions from a JSON file on disk.
pub fn load_tools(path: impl AsRef<Path>) -> Result<Vec<Tool>, McpError> {
    let data = std::fs::read(path)?;
    parse_tools(&data)
}

/// Parse tool definitions from raw JSON bytes.
pub fn parse_tools(data: &[u8]) -> Result<Vec<Tool>, McpError> {
    let raw: Vec<Value> = serde_json::from_slice(data)?;
    let mut tools = Vec::with_capacity(raw.len());

    for val in raw {
        let name = val["name"].as_str().unwrap_or_default().to_string();
        let description = val["description"].as_str().unwrap_or_default().to_string();
        let input_schema = val["inputSchema"].clone();

        // Parse schema metadata for validation.
        let schema_meta = parse_schema_meta(&input_schema);

        tools.push(Tool {
            name,
            description,
            input_schema,
            schema_meta,
        });
    }

    Ok(tools)
}

/// Load resource definitions from a JSON file on disk.
pub fn load_resources(path: impl AsRef<Path>) -> Result<Vec<Resource>, McpError> {
    let data = std::fs::read(path)?;
    parse_resources(&data)
}

/// Parse resource definitions from raw JSON bytes.
pub fn parse_resources(data: &[u8]) -> Result<Vec<Resource>, McpError> {
    let resources: Vec<Resource> = serde_json::from_slice(data)?;
    Ok(resources)
}

/// Extract validation metadata from a JSON Schema object.
fn parse_schema_meta(schema: &Value) -> SchemaMeta {
    let mut meta = SchemaMeta::default();

    if let Some(arr) = schema.get("required").and_then(|v| v.as_array()) {
        meta.required = arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
    }

    if let Some(arr) = schema.get("oneOf").and_then(|v| v.as_array()) {
        meta.one_of = arr
            .iter()
            .filter_map(|v| {
                v.get("required").and_then(|r| r.as_array()).map(|reqs| {
                    SchemaRequirementSet {
                        required: reqs
                            .iter()
                            .filter_map(|r| r.as_str().map(String::from))
                            .collect(),
                    }
                })
            })
            .collect();
    }

    if let Some(obj) = schema.get("dependencies").and_then(|v| v.as_object()) {
        let mut deps = HashMap::new();
        for (key, val) in obj {
            if let Some(arr) = val.as_array() {
                deps.insert(
                    key.clone(),
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect(),
                );
            }
        }
        meta.dependencies = deps;
    }

    meta
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_parse_tools() {
        let json = r#"[{"name":"echo","description":"echoes","inputSchema":{"type":"object","properties":{"msg":{"type":"string"}},"required":["msg"]}}]"#;
        let tools = parse_tools(json.as_bytes()).unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");
        assert_eq!(tools[0].schema_meta.required, vec!["msg"]);
    }

    #[test]
    fn test_parse_resources() {
        let json = r#"[{"name":"forecast","description":"monthly","uri":"s3://bucket/file.csv","mimeType":"text/csv"}]"#;
        let resources = parse_resources(json.as_bytes()).unwrap();
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].name, "forecast");
        assert_eq!(resources[0].uri, "s3://bucket/file.csv");
    }

    #[test]
    fn test_load_tools_missing_file() {
        let result = load_tools("/nonexistent/path.json");
        assert!(result.is_err());
    }

    #[test]
    fn test_load_tools_malformed() {
        let result = parse_tools(b"{not valid json");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_tools_with_one_of() {
        let json = r#"[{"name":"otp","description":"otp","inputSchema":{"type":"object","properties":{},"oneOf":[{"required":["phone"]},{"required":["email"]}]}}]"#;
        let tools = parse_tools(json.as_bytes()).unwrap();
        assert_eq!(tools[0].schema_meta.one_of.len(), 2);
    }

    #[test]
    fn test_parse_tools_with_dependencies() {
        let json = r#"[{"name":"ch","description":"ch","inputSchema":{"type":"object","properties":{},"dependencies":{"geo_lat":["geo_lon"]}}}]"#;
        let tools = parse_tools(json.as_bytes()).unwrap();
        assert!(tools[0].schema_meta.dependencies.contains_key("geo_lat"));
    }
}
