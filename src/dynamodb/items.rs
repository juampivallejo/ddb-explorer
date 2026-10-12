use std::{collections::HashMap, str::FromStr};

use aws_sdk_dynamodb::types::{AttributeValue, KeySchemaElement, KeyType, ScalarAttributeType};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Number, Value};

use super::{Connection, LoadError, run_request, sdk_result};

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

pub fn describe_table(connection: Connection, table: String) -> Result<TableSchema, LoadError> {
    run_request(connection, |client| async move {
        let response = sdk_result(client.describe_table().table_name(table).send().await)?;
        let table = response.table().ok_or(LoadError::Unexpected)?;
        let types: HashMap<_, _> = table
            .attribute_definitions()
            .iter()
            .map(|definition| {
                (
                    definition.attribute_name().to_owned(),
                    definition.attribute_type().clone(),
                )
            })
            .collect();
        let target = |index_name, keys: &[KeySchemaElement]| -> Result<KeyTarget, LoadError> {
            let key = |kind: KeyType| -> Result<Option<KeyDefinition>, LoadError> {
                keys.iter()
                    .find(|key| *key.key_type() == kind)
                    .map(|key| {
                        Ok(KeyDefinition {
                            name: key.attribute_name().to_owned(),
                            kind: types
                                .get(key.attribute_name())
                                .ok_or(LoadError::Unexpected)?
                                .clone(),
                        })
                    })
                    .transpose()
            };
            Ok(KeyTarget {
                index_name,
                partition: key(KeyType::Hash)?.ok_or(LoadError::Unexpected)?,
                sort: key(KeyType::Range)?,
            })
        };
        let mut targets = vec![target(None, table.key_schema())?];
        for index in table.global_secondary_indexes() {
            targets.push(target(
                index.index_name().map(str::to_owned),
                index.key_schema(),
            )?);
        }
        Ok(TableSchema { targets })
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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

type Expression = (
    String,
    HashMap<String, String>,
    HashMap<String, AttributeValue>,
);

fn query_expression(
    target: &KeyTarget,
    partition: AttributeValue,
    sort: Option<(SortOperator, AttributeValue, Option<AttributeValue>)>,
) -> Result<Expression, LoadError> {
    let mut expression = "#pk = :pk".to_owned();
    let mut names = HashMap::from([("#pk".into(), target.partition.name.clone())]);
    let mut values = HashMap::from([(":pk".into(), partition)]);
    if let Some((operator, value, end)) = sort {
        let key = target.sort.as_ref().ok_or(LoadError::InvalidRequest)?;
        names.insert("#sk".into(), key.name.clone());
        values.insert(":sk".into(), value);
        expression.push_str(" AND ");
        expression.push_str(match operator {
            SortOperator::Equal => "#sk = :sk",
            SortOperator::Less => "#sk < :sk",
            SortOperator::LessEqual => "#sk <= :sk",
            SortOperator::Greater => "#sk > :sk",
            SortOperator::GreaterEqual => "#sk >= :sk",
            SortOperator::BeginsWith => "begins_with(#sk, :sk)",
            SortOperator::Between => {
                values.insert(":end".into(), end.ok_or(LoadError::InvalidRequest)?);
                "#sk BETWEEN :sk AND :end"
            }
            SortOperator::Any => return Err(LoadError::InvalidRequest),
        });
    }
    Ok((expression, names, values))
}

/// Exactly one page per call. Never auto-paginate item reads.
pub fn read_items(
    connection: Connection,
    request: ReadRequest,
    start: Option<Item>,
) -> Result<ItemPage, LoadError> {
    if !(1..=100).contains(&request.limit) {
        return Err(LoadError::InvalidRequest);
    }
    run_request(connection, |client| async move {
        let (items, next_key) = match request.operation {
            ReadOperation::Scan => {
                let page = sdk_result(
                    client
                        .scan()
                        .table_name(request.table)
                        .set_index_name(request.target.index_name)
                        .limit(request.limit)
                        .set_exclusive_start_key(start)
                        .send()
                        .await,
                )?;
                (page.items.unwrap_or_default(), page.last_evaluated_key)
            }
            ReadOperation::Query { partition, sort } => {
                let (expression, names, values) =
                    query_expression(&request.target, partition, sort)?;
                let page = sdk_result(
                    client
                        .query()
                        .table_name(request.table)
                        .set_index_name(request.target.index_name)
                        .limit(request.limit)
                        .key_condition_expression(expression)
                        .set_expression_attribute_names(Some(names))
                        .set_expression_attribute_values(Some(values))
                        .set_exclusive_start_key(start)
                        .send()
                        .await,
                )?;
                (page.items.unwrap_or_default(), page.last_evaluated_key)
            }
        };
        let items = items.into_iter().map(item_json).collect::<Result<_, _>>()?;
        Ok(ItemPage {
            items,
            next_key: next_key.filter(|key| !key.is_empty()),
        })
    })
}

fn item_json(item: Item) -> Result<Value, LoadError> {
    item.into_iter()
        .map(|(name, value)| Ok((name, attribute_json(value)?)))
        .collect::<Result<serde_json::Map<_, _>, LoadError>>()
        .map(Value::Object)
}

fn attribute_json(value: AttributeValue) -> Result<Value, LoadError> {
    Ok(match value {
        AttributeValue::S(value) => Value::String(value),
        AttributeValue::N(value) => {
            Value::Number(Number::from_str(&value).map_err(|_| LoadError::Unexpected)?)
        }
        AttributeValue::Bool(value) => Value::Bool(value),
        AttributeValue::Null(_) => Value::Null,
        AttributeValue::B(value) => Value::String(STANDARD.encode(value.as_ref())),
        AttributeValue::L(values) => Value::Array(
            values
                .into_iter()
                .map(attribute_json)
                .collect::<Result<_, _>>()?,
        ),
        AttributeValue::M(item) => item_json(item)?,
        AttributeValue::Ss(values) => Value::Array(values.into_iter().map(Value::String).collect()),
        AttributeValue::Ns(values) => Value::Array(
            values
                .into_iter()
                .map(|value| attribute_json(AttributeValue::N(value)))
                .collect::<Result<_, _>>()?,
        ),
        AttributeValue::Bs(values) => Value::Array(
            values
                .into_iter()
                .map(|value| Value::String(STANDARD.encode(value.as_ref())))
                .collect(),
        ),
        _ => return Err(LoadError::Unexpected),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_json_preserves_numbers_and_nested_types() {
        let value = attribute_json(AttributeValue::M(HashMap::from([
            (
                "n".into(),
                AttributeValue::N("123456789012345678901234567890.12345678".into()),
            ),
            (
                "nested".into(),
                AttributeValue::L(vec![AttributeValue::Bool(true), AttributeValue::Null(true)]),
            ),
            ("binary".into(), AttributeValue::B(vec![72, 105].into())),
            (
                "set".into(),
                AttributeValue::Ns(vec!["1".into(), "2".into()]),
            ),
        ])))
        .unwrap();
        assert_eq!(
            value["n"].to_string(),
            "123456789012345678901234567890.12345678"
        );
        assert_eq!(value["nested"], serde_json::json!([true, null]));
        assert_eq!(value["binary"], "SGk=");
        assert_eq!(value["set"], serde_json::json!([1, 2]));
    }

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

    #[test]
    #[ignore = "requires seeded DynamoDB Local"]
    fn scans_queries_and_pagination_work_locally() {
        let schema = describe_table(Connection::Local, "Events".into()).unwrap();
        let request = ReadRequest {
            table: "Events".into(),
            target: schema.targets[0].clone(),
            operation: ReadOperation::Scan,
            limit: 5,
        };
        let first = read_items(Connection::Local, request.clone(), None).unwrap();
        assert_eq!(first.items.len(), 5);
        let second = read_items(Connection::Local, request, first.next_key).unwrap();
        assert_eq!(second.items.len(), 5);
        assert!(first.items.iter().all(|item| !second.items.contains(item)));
        for (target, pk) in [(0, "USER#1"), (1, "LOGIN")] {
            let request = ReadRequest {
                table: "Events".into(),
                target: schema.targets[target].clone(),
                operation: ReadOperation::Query {
                    partition: AttributeValue::S(pk.into()),
                    sort: None,
                },
                limit: 50,
            };
            assert_eq!(
                read_items(Connection::Local, request, None)
                    .unwrap()
                    .items
                    .len(),
                20
            );
        }
        let request = ReadRequest {
            table: "Events".into(),
            target: schema.targets[0].clone(),
            operation: ReadOperation::Query {
                partition: AttributeValue::S("USER#1".into()),
                sort: Some((
                    SortOperator::Between,
                    AttributeValue::S("2026-01-01T00:02:00Z".into()),
                    Some(AttributeValue::S("2026-01-01T00:04:00Z".into())),
                )),
            },
            limit: 50,
        };
        assert_eq!(
            read_items(Connection::Local, request, None)
                .unwrap()
                .items
                .len(),
            3
        );
        let empty = describe_table(Connection::Local, "EmptyTable".into()).unwrap();
        assert!(
            read_items(
                Connection::Local,
                ReadRequest {
                    table: "EmptyTable".into(),
                    target: empty.targets[0].clone(),
                    operation: ReadOperation::Scan,
                    limit: 50
                },
                None
            )
            .unwrap()
            .items
            .is_empty()
        );
    }
}
