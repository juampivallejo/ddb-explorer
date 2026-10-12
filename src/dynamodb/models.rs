use super::json::attribute_json;
use aws_sdk_dynamodb::types::{AttributeValue, ScalarAttributeType};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Number, Value};
use std::{collections::HashMap, str::FromStr};

pub type Item = HashMap<String, AttributeValue>;

#[derive(Clone, Debug)]
pub struct KeyDefinition {
    pub name: String,
    pub kind: ScalarAttributeType,
}

impl KeyDefinition {
    pub fn parse(&self, value: &str) -> Result<AttributeValue, String> {
        if value.is_empty() {
            return Err(format!("Enter a value for {}.", self.name));
        }
        match self.kind {
            ScalarAttributeType::S => Ok(AttributeValue::S(value.to_owned())),
            ScalarAttributeType::N => {
                Number::from_str(value).map_err(|_| format!("{} must be a number.", self.name))?;
                Ok(AttributeValue::N(value.to_owned()))
            }
            ScalarAttributeType::B => {
                let bytes = STANDARD
                    .decode(value)
                    .map_err(|_| format!("{} must be base64-encoded binary.", self.name))?;
                if bytes.is_empty() {
                    return Err(format!("{} must not be empty.", self.name));
                }
                Ok(AttributeValue::B(bytes.into()))
            }
            _ => Err("This key type is not supported.".into()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct KeyTarget {
    pub index_name: Option<String>,
    pub partition: KeyDefinition,
    pub sort: Option<KeyDefinition>,
}

impl KeyTarget {
    pub fn label(&self) -> &str {
        self.index_name.as_deref().unwrap_or("Primary key")
    }
}

#[derive(Clone, Debug)]
pub struct TableSchema {
    pub targets: Vec<KeyTarget>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SortOperator {
    #[default]
    Any,
    Equal,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    BeginsWith,
    Between,
}

impl SortOperator {
    pub const ALL: [Self; 8] = [
        Self::Any,
        Self::Equal,
        Self::Less,
        Self::LessEqual,
        Self::Greater,
        Self::GreaterEqual,
        Self::BeginsWith,
        Self::Between,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Any => "Any",
            Self::Equal => "=",
            Self::Less => "<",
            Self::LessEqual => "≤",
            Self::Greater => ">",
            Self::GreaterEqual => "≥",
            Self::BeginsWith => "Begins with",
            Self::Between => "Between",
        }
    }
}

#[derive(Clone, Debug)]
pub enum ReadOperation {
    Scan,
    Query {
        partition: AttributeValue,
        sort: Option<(SortOperator, AttributeValue, Option<AttributeValue>)>,
    },
}

#[derive(Clone, Debug)]
pub struct ReadRequest {
    pub table: String,
    pub target: KeyTarget,
    pub operation: ReadOperation,
    pub limit: i32,
}

impl ReadRequest {
    pub fn summary(&self) -> String {
        let display = |value: &AttributeValue| {
            attribute_json(value.clone())
                .map(|value| value.to_string())
                .unwrap_or_else(|_| "?".into())
        };
        match &self.operation {
            ReadOperation::Scan => format!("Scan · {}", self.target.label()),
            ReadOperation::Query { partition, sort } => {
                let mut summary = format!(
                    "Query · {} · {} = {}",
                    self.target.label(),
                    self.target.partition.name,
                    display(partition)
                );
                if let (Some(key), Some((operator, value, end))) = (&self.target.sort, sort) {
                    summary.push_str(&format!(
                        " · {} {} {}",
                        key.name,
                        operator.label(),
                        display(value)
                    ));
                    if let Some(end) = end {
                        summary.push_str(&format!(" and {}", display(end)));
                    }
                }
                summary
            }
        }
    }
}

#[derive(Debug)]
pub struct ItemPage {
    pub items: Vec<Value>,
    pub next_key: Option<Item>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn key_values_validate_types_without_trimming_strings() {
        let string = KeyDefinition {
            name: "pk".into(),
            kind: ScalarAttributeType::S,
        };
        assert_eq!(
            string.parse(" leading ").unwrap(),
            AttributeValue::S(" leading ".into())
        );
        assert!(string.parse("").is_err());
        let numeric = KeyDefinition {
            name: "sk".into(),
            kind: ScalarAttributeType::N,
        };
        assert!(numeric.parse("abc").is_err());
        assert!(numeric.parse("42.5").is_ok());
    }
}
