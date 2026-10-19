//! Form validation does not depend on GPUI or the AWS runtime.
use crate::dynamodb::{ReadOperation, ReadRequest, SortOperator, TableSchema};
use aws_sdk_dynamodb::types::ScalarAttributeType;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum ReadMode {
    Scan,
    #[default]
    Query,
}

impl ReadMode {
    pub const ALL: [Self; 2] = [Self::Scan, Self::Query];
    pub fn label(self) -> &'static str {
        match self {
            Self::Scan => "Scan",
            Self::Query => "Query",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            Self::Scan => "Browse items in a table or index.",
            Self::Query => "Find items using a partition key and optional sort key condition.",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum QueryField {
    Partition,
    Sort,
    End,
}

#[derive(Debug)]
pub(super) struct QueryValidation {
    pub field: Option<QueryField>,
    pub message: String,
}

impl QueryValidation {
    pub fn general(message: impl Into<String>) -> Self {
        Self {
            field: None,
            message: message.into(),
        }
    }
    fn at(field: QueryField, message: impl Into<String>) -> Self {
        Self {
            field: Some(field),
            message: message.into(),
        }
    }
}

pub(super) struct QueryDraft {
    pub target: usize,
    pub partition: String,
    pub sort_operator: SortOperator,
    pub sort: String,
    pub end: String,
}

impl QueryDraft {
    pub fn validate(
        &self,
        table: &str,
        schema: &TableSchema,
        limit: i32,
    ) -> Result<ReadRequest, QueryValidation> {
        let target = schema
            .targets
            .get(self.target)
            .ok_or_else(|| QueryValidation::general("Select a key or index first."))?;
        let partition = target
            .partition
            .parse(&self.partition)
            .map_err(|message| QueryValidation::at(QueryField::Partition, message))?;
        let sort = if self.sort_operator == SortOperator::Any {
            None
        } else {
            let key = target.sort.as_ref().ok_or_else(|| {
                QueryValidation::at(QueryField::Sort, "This key has no sort key.")
            })?;
            if self.sort_operator == SortOperator::BeginsWith && key.kind == ScalarAttributeType::N
            {
                return Err(QueryValidation::at(
                    QueryField::Sort,
                    "Begins with is available for string and binary sort keys only.",
                ));
            }
            let value = key
                .parse(&self.sort)
                .map_err(|message| QueryValidation::at(QueryField::Sort, message))?;
            let end = if self.sort_operator == SortOperator::Between {
                let end = key
                    .parse(&self.end)
                    .map_err(|message| QueryValidation::at(QueryField::End, message))?;
                Some(end)
            } else {
                None
            };
            Some((self.sort_operator, value, end))
        };
        Ok(ReadRequest {
            table: table.to_owned(),
            target: target.clone(),
            operation: ReadOperation::Query { partition, sort },
            limit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dynamodb::{KeyDefinition, KeyTarget};
    use aws_sdk_dynamodb::types::AttributeValue;

    fn schema() -> TableSchema {
        let primary = KeyTarget {
            index_name: None,
            partition: KeyDefinition {
                name: "pk".into(),
                kind: ScalarAttributeType::S,
            },
            sort: Some(KeyDefinition {
                name: "sk".into(),
                kind: ScalarAttributeType::N,
            }),
        };
        let index = KeyTarget {
            index_name: Some("ByType".into()),
            partition: KeyDefinition {
                name: "type".into(),
                kind: ScalarAttributeType::S,
            },
            sort: None,
        };
        TableSchema {
            targets: vec![primary, index],
        }
    }

    fn draft() -> QueryDraft {
        QueryDraft {
            target: 0,
            partition: " USER#1 ".into(),
            sort_operator: SortOperator::Any,
            sort: String::new(),
            end: String::new(),
        }
    }

    #[test]
    fn validates_primary_and_index_queries_without_changing_executed_requests() {
        let mut draft = draft();
        let primary = draft.validate("Events", &schema(), 50).unwrap();
        draft.target = 1;
        draft.partition = "LOGIN".into();
        let index = draft.validate("Events", &schema(), 50).unwrap();
        assert_eq!(primary.target.index_name, None);
        assert_eq!(index.target.index_name.as_deref(), Some("ByType"));
        let ReadOperation::Query { partition, .. } = primary.operation else {
            panic!("expected query")
        };
        assert_eq!(partition, AttributeValue::S(" USER#1 ".into()));
        draft.target = 100;
        assert!(draft.validate("Events", &schema(), 50).is_err());
    }

    #[test]
    fn validates_sort_types_and_required_between_endpoint() {
        let mut draft = draft();
        draft.sort_operator = SortOperator::Between;
        draft.sort = "10".into();
        assert!(draft.validate("Events", &schema(), 50).is_err());
        draft.end = "20.5".into();
        let request = draft.validate("Events", &schema(), 50).unwrap();
        let ReadOperation::Query {
            sort: Some((SortOperator::Between, start, Some(end))),
            ..
        } = request.operation
        else {
            panic!("expected between condition")
        };
        assert_eq!(start, AttributeValue::N("10".into()));
        assert_eq!(end, AttributeValue::N("20.5".into()));
        // Pass the entered bounds through; DynamoDB owns range semantics.
        draft.sort = "20.5".into();
        draft.end = "10".into();
        let reversed = draft.validate("Events", &schema(), 50).unwrap();
        let ReadOperation::Query {
            sort: Some((SortOperator::Between, start, Some(end))),
            ..
        } = reversed.operation
        else {
            panic!("expected between condition")
        };
        assert_eq!(start, AttributeValue::N("20.5".into()));
        assert_eq!(end, AttributeValue::N("10".into()));
        draft.sort_operator = SortOperator::BeginsWith;
        assert!(draft.validate("Events", &schema(), 50).is_err());
        draft.sort_operator = SortOperator::Equal;
        draft.sort = "not a number".into();
        assert!(draft.validate("Events", &schema(), 50).is_err());
        draft.target = 1;
        assert!(draft.validate("Events", &schema(), 50).is_err());
        draft.sort_operator = SortOperator::Any;
        assert!(draft.validate("Events", &schema(), 50).is_ok());
        draft.partition.clear();
        assert!(draft.validate("Events", &schema(), 50).is_err());
    }
}
