use jsonschema::validator_for;
use serde_json::Value;

use crate::error::{BackmaticError, Result};

pub fn validate_yaml_against_schema(yaml_content: &str) -> Result<()> {
    let schema_str = include_str!("../../schema/backmatic.schema.json");
    let schema: Value = serde_json::from_str(schema_str)
        .map_err(|e| BackmaticError::Config(format!("invalid embedded schema: {e}")))?;
    let compiled = validator_for(&schema)
        .map_err(|e| BackmaticError::Config(format!("schema compile error: {e}")))?;

    let yaml_value: serde_yaml::Value = serde_yaml::from_str(yaml_content)
        .map_err(|e| BackmaticError::Config(format!("yaml parse error: {e}")))?;
    let json_value = serde_json::to_value(&yaml_value)
        .map_err(|e| BackmaticError::Config(format!("yaml to json error: {e}")))?;

    if !compiled.is_valid(&json_value) {
        let msgs: Vec<String> = compiled
            .iter_errors(&json_value)
            .map(|e| e.to_string())
            .collect();
        return Err(BackmaticError::Validation(msgs.join("; ")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_config_passes_schema() {
        let yaml = r#"
version: 1
rsync:
  - comment: test
    src: [/tmp/a]
    dest: [/tmp/b]
"#;
        validate_yaml_against_schema(yaml).unwrap();
    }

    #[test]
    fn destmount_only_passes_schema() {
        let yaml = r#"
version: 1
borg:
  - comment: luks only
    srcmount:
      - host: h1
        user: u
        path: /
    destmount:
      - uuid: "12345678-1234-1234-1234-123456789abc"
"#;
        validate_yaml_against_schema(yaml).unwrap();
    }
}
