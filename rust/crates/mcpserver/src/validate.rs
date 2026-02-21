use serde_json::Value;
use crate::types::Tool;

impl Tool {
    /// Validate arguments against the tool's input schema metadata.
    pub fn validate_arguments(&self, args: &Value) -> Result<(), String> {
        let empty = serde_json::Map::new();
        let obj = args.as_object().unwrap_or(&empty);
        let meta = &self.schema_meta;

        // Check required fields.
        for field in &meta.required {
            if !obj.contains_key(field) {
                return Err(format!("missing required field \"{}\"", field));
            }
        }

        // Check oneOf — at least one set of required fields must be satisfied.
        if !meta.one_of.is_empty() {
            let satisfied = meta.one_of.iter().any(|set| {
                set.required.iter().all(|f| obj.contains_key(f))
            });
            if !satisfied {
                return Err("arguments must satisfy oneOf requirements".into());
            }
        }

        // Check dependencies — if field A is present, fields B must also be present.
        for (field, deps) in &meta.dependencies {
            if obj.contains_key(field) {
                for dep in deps {
                    if !obj.contains_key(dep) {
                        return Err(format!(
                            "field \"{}\" requires \"{}\" to also be present",
                            field, dep
                        ));
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::parse_tools;

    fn make_tool(schema_json: &str) -> Tool {
        let json = format!(
            r#"[{{"name":"test","description":"test","inputSchema":{}}}]"#,
            schema_json
        );
        let tools = parse_tools(json.as_bytes()).unwrap();
        tools.into_iter().next().unwrap()
    }

    #[test]
    fn test_validate_required_present() {
        let tool = make_tool(r#"{"type":"object","properties":{},"required":["name"]}"#);
        let args = serde_json::json!({"name": "hello"});
        assert!(tool.validate_arguments(&args).is_ok());
    }

    #[test]
    fn test_validate_required_missing() {
        let tool = make_tool(r#"{"type":"object","properties":{},"required":["name"]}"#);
        let args = serde_json::json!({});
        let err = tool.validate_arguments(&args).unwrap_err();
        assert!(err.contains("missing required field"));
    }

    #[test]
    fn test_validate_one_of_match() {
        let tool = make_tool(
            r#"{"type":"object","properties":{},"oneOf":[{"required":["phone"]},{"required":["email"]}]}"#,
        );
        let args = serde_json::json!({"phone": "+1555"});
        assert!(tool.validate_arguments(&args).is_ok());
    }

    #[test]
    fn test_validate_one_of_none_match() {
        let tool = make_tool(
            r#"{"type":"object","properties":{},"oneOf":[{"required":["phone"]},{"required":["email"]}]}"#,
        );
        let args = serde_json::json!({});
        let err = tool.validate_arguments(&args).unwrap_err();
        assert!(err.contains("oneOf"));
    }

    #[test]
    fn test_validate_dependencies_satisfied() {
        let tool = make_tool(
            r#"{"type":"object","properties":{},"dependencies":{"geo_lat":["geo_lon"]}}"#,
        );
        let args = serde_json::json!({"geo_lat": 1.0, "geo_lon": 2.0});
        assert!(tool.validate_arguments(&args).is_ok());
    }

    #[test]
    fn test_validate_dependencies_missing() {
        let tool = make_tool(
            r#"{"type":"object","properties":{},"dependencies":{"geo_lat":["geo_lon"]}}"#,
        );
        let args = serde_json::json!({"geo_lat": 1.0});
        let err = tool.validate_arguments(&args).unwrap_err();
        assert!(err.contains("requires"));
    }

    #[test]
    fn test_validate_combined_required_and_one_of() {
        let tool = make_tool(
            r#"{"type":"object","properties":{},"required":["code"],"oneOf":[{"required":["phone","code"]},{"required":["email","code"]}]}"#,
        );
        let args = serde_json::json!({"code": "123456", "phone": "+1555"});
        assert!(tool.validate_arguments(&args).is_ok());

        let args2 = serde_json::json!({"phone": "+1555"});
        assert!(tool.validate_arguments(&args2).is_err());
    }
}
