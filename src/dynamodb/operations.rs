//! AWS operations, run exclusively on the service's Tokio runtime.
use super::json::item_json;
use super::{
    Item, ItemPage, KeyDefinition, KeyTarget, LoadError, ReadOperation, ReadRequest, SortOperator,
    TableSchema, sdk_result,
};
use aws_sdk_dynamodb::{
    Client,
    types::{AttributeValue, KeySchemaElement, KeyType},
};
use std::collections::HashMap;

pub(super) async fn list_tables(client: Client, page_size: i32) -> Result<Vec<String>, LoadError> {
    let mut tables = Vec::new();
    let mut pages = client
        .list_tables()
        .limit(page_size)
        .into_paginator()
        .send();
    while let Some(page) = pages.next().await {
        tables.extend(sdk_result(page)?.table_names().iter().cloned());
    }
    tables.sort();
    tables.dedup();
    Ok(tables)
}

pub(super) async fn describe_table(
    client: Client,
    table: String,
) -> Result<TableSchema, LoadError> {
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
pub(super) async fn read_items(
    client: Client,
    request: ReadRequest,
    start: Option<Item>,
) -> Result<ItemPage, LoadError> {
    if !(1..=100).contains(&request.limit) {
        return Err(LoadError::InvalidRequest);
    }
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
            let (expression, names, values) = query_expression(&request.target, partition, sort)?;
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
}
