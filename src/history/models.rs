//! History uses its own versioned format, independent of AWS SDK types.
use crate::dynamodb::{
    Connection, ItemPage, KeyTarget, ReadOperation, ReadRequest, SortOperator, TableSchema,
};
use aws_sdk_dynamodb::types::AttributeValue;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

pub const KEY_LIMIT: usize = 50;
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

pub(super) fn primary_key(item: &Value, primary: &KeyTarget) -> Option<Value> {
    let mut key = serde_json::Map::new();
    key.insert(
        primary.partition.name.clone(),
        item.get(&primary.partition.name)?.clone(),
    );
    if let Some(sort) = &primary.sort {
        key.insert(sort.name.clone(), item.get(&sort.name)?.clone());
    }
    key.sort_keys();
    Some(Value::Object(key))
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedQuery {
    pub index: Option<String>,
    pub partition_key: String,
    pub partition: String,
    pub sort_key: Option<String>,
    pub operator: SortOperator,
    pub sort: String,
    pub end: String,
    pub summary: String,
}

fn input_value(value: &AttributeValue) -> String {
    match value {
        AttributeValue::S(value) | AttributeValue::N(value) => value.clone(),
        AttributeValue::B(value) => STANDARD.encode(value.as_ref()),
        _ => String::new(),
    }
}

impl SavedQuery {
    pub fn target_index(&self, schema: &TableSchema) -> Option<usize> {
        schema.targets.iter().position(|target| {
            target.index_name == self.index
                && target.partition.name == self.partition_key
                && target.sort.as_ref().map(|key| &key.name) == self.sort_key.as_ref()
        })
    }
    pub fn from_request(request: &ReadRequest) -> Option<Self> {
        let ReadOperation::Query { partition, sort } = &request.operation else {
            return None;
        };
        Some(Self {
            index: request.target.index_name.clone(),
            partition_key: request.target.partition.name.clone(),
            partition: input_value(partition),
            sort_key: request.target.sort.as_ref().map(|key| key.name.clone()),
            operator: sort.as_ref().map_or(SortOperator::Any, |(op, _, _)| *op),
            sort: sort
                .as_ref()
                .map_or_else(String::new, |(_, value, _)| input_value(value)),
            end: sort
                .as_ref()
                .and_then(|(_, _, end)| end.as_ref())
                .map_or_else(String::new, input_value),
            summary: request.summary(),
        })
    }
}

/// Metadata stays in memory; snapshot payloads are loaded only when opened.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub version: u32,
    pub id: String,
    pub created_ms: u64,
    pub connection: String,
    pub table: String,
    pub query: Option<SavedQuery>,
    pub result_count: usize,
    pub pages: usize,
    pub keys: Vec<Value>,
    pub snapshot: bool,
}

impl Entry {
    pub fn new(connection: Connection, table: String, query: Option<SavedQuery>) -> Self {
        let created_ms = now_ms();
        Self {
            version: 1,
            id: format!(
                "{created_ms}-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ),
            created_ms,
            connection: connection_tag(connection),
            table,
            query,
            result_count: 0,
            pages: 0,
            keys: Vec::new(),
            snapshot: false,
        }
    }

    pub fn add_page(&mut self, page: &ItemPage, primary: &KeyTarget) {
        self.pages += 1;
        self.result_count += page.items.len();
        for item in &page.items {
            if self.keys.len() == KEY_LIMIT {
                break;
            }
            if let Some(key) = primary_key(item, primary) {
                self.keys.push(key);
            }
        }
    }
}

fn connection_tag(connection: Connection) -> String {
    if connection == Connection::Local {
        return format!("{} · {}", connection.label(), connection.description());
    }
    let mut tag = connection.label().to_owned();
    for name in ["AWS_PROFILE", "AWS_REGION", "AWS_DEFAULT_REGION"] {
        if let Ok(value) = std::env::var(name) {
            tag.push_str(&format!(" · {value}"));
        }
    }
    tag
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dynamodb::KeyDefinition;
    use aws_sdk_dynamodb::types::ScalarAttributeType;

    fn primary() -> KeyTarget {
        KeyTarget {
            index_name: None,
            partition: KeyDefinition {
                name: "pk".into(),
                kind: ScalarAttributeType::S,
            },
            sort: Some(KeyDefinition {
                name: "sk".into(),
                kind: ScalarAttributeType::N,
            }),
        }
    }

    #[test]
    fn pagination_keeps_only_fifty_primary_keys_and_no_payload() {
        let mut entry = Entry::new(Connection::Local, "Events".into(), None);
        for start in [0, 40] {
            let page = ItemPage {
                items: (start..start + 40).map(|index| serde_json::json!({"pk": "user", "sk": index, "gsi_pk": "other", "large_payload": "private"})).collect(),
                next_key: None,
            };
            entry.add_page(&page, &primary());
        }
        assert_eq!(entry.pages, 2);
        assert_eq!(entry.result_count, 80);
        assert_eq!(entry.keys.len(), 50);
        assert_eq!(entry.keys[49], serde_json::json!({"pk": "user", "sk": 49}));
        let bytes = serde_json::to_string(&entry).unwrap();
        assert!(!bytes.contains("private"));
        assert!(!bytes.contains("gsi_pk"));
        let reloaded: Entry = serde_json::from_str(&bytes).unwrap();
        assert_eq!(reloaded.keys, entry.keys);
    }

    #[test]
    fn saved_query_retains_exact_form_values_and_excludes_scans() {
        let mut request = ReadRequest {
            table: "Events".into(),
            target: primary(),
            limit: 50,
            operation: ReadOperation::Query {
                partition: AttributeValue::S(" leading space ".into()),
                sort: Some((
                    SortOperator::Between,
                    AttributeValue::N("12345678901234567890.123456789".into()),
                    Some(AttributeValue::N("99999999999999999999".into())),
                )),
            },
        };
        let saved = SavedQuery::from_request(&request).unwrap();
        let saved: SavedQuery =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        assert_eq!(saved.partition, " leading space ");
        assert_eq!(saved.operator, SortOperator::Between);
        assert_eq!(saved.sort, "12345678901234567890.123456789");
        assert_eq!(saved.end, "99999999999999999999");
        let mut index = primary();
        index.index_name = Some("ByUser".into());
        request.target = index.clone();
        let indexed = SavedQuery::from_request(&request).unwrap();
        let mut schema = TableSchema {
            targets: vec![primary(), index],
        };
        assert_eq!(indexed.target_index(&schema), Some(1));
        schema.targets.swap(0, 1);
        assert_eq!(indexed.target_index(&schema), Some(0));
        schema.targets.remove(0);
        assert_eq!(indexed.target_index(&schema), None);
        assert_eq!(input_value(&AttributeValue::B(vec![0, 255].into())), "AP8=");
        request.operation = ReadOperation::Scan;
        assert!(SavedQuery::from_request(&request).is_none());
    }
}
