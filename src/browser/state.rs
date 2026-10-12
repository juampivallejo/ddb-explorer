//! Plain Rust state transitions. GPUI tasks and inputs live in the view.
use crate::dynamodb::{Item, ItemPage, LoadError, ReadOperation, ReadRequest, TableSchema};

#[derive(Default)]
pub(super) enum SchemaState {
    #[default]
    Unselected,
    Loading {
        table: String,
        indicator_visible: bool,
    },
    Ready {
        table: String,
        schema: TableSchema,
    },
    Failed {
        table: String,
        error: LoadError,
    },
}

impl SchemaState {
    pub fn table(&self) -> Option<&str> {
        match self {
            Self::Unselected => None,
            Self::Loading { table, .. }
            | Self::Ready { table, .. }
            | Self::Failed { table, .. } => Some(table),
        }
    }

    pub fn schema(&self) -> Option<&TableSchema> {
        match self {
            Self::Ready { schema, .. } => Some(schema),
            _ => None,
        }
    }

    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading { .. })
    }
    pub fn loading_indicator_visible(&self) -> bool {
        matches!(
            self,
            Self::Loading {
                indicator_visible: true,
                ..
            }
        )
    }
    pub fn error(&self) -> Option<LoadError> {
        match self {
            Self::Failed { error, .. } => Some(*error),
            _ => None,
        }
    }
}

#[derive(Default)]
pub(super) struct BrowserState {
    pub schema: SchemaState,
    pub reads: ReadState,
    schema_generation: u64,
}

impl BrowserState {
    pub fn clear(&mut self) {
        self.schema_generation += 1;
        self.schema = SchemaState::Unselected;
        self.reads.reset();
    }

    pub fn open(&mut self, table: String) -> u64 {
        self.clear();
        self.schema = SchemaState::Loading {
            table,
            indicator_visible: false,
        };
        self.schema_generation
    }

    pub fn reveal_schema_loading(&mut self, generation: u64) -> bool {
        if generation != self.schema_generation {
            return false;
        }
        if let SchemaState::Loading {
            indicator_visible, ..
        } = &mut self.schema
        {
            *indicator_visible = true;
            return true;
        }
        false
    }

    pub fn finish_schema(
        &mut self,
        generation: u64,
        result: Result<TableSchema, LoadError>,
    ) -> bool {
        if generation != self.schema_generation || !self.schema.is_loading() {
            return false;
        }
        let table = self.schema.table().expect("loading has a table").to_owned();
        self.schema = match result {
            Ok(schema) => SchemaState::Ready { table, schema },
            Err(error) => SchemaState::Failed { table, error },
        };
        true
    }
}

pub(super) struct LoadedPage {
    pub id: u64,
    pub request: ReadRequest,
    pub page: ItemPage,
    pub selected_item: Option<usize>,
}

pub(super) struct PreparedScan {
    pub request: ReadRequest,
    pub start: Option<Item>,
}

/// The only value the view can submit to the service after a state transition.
pub(super) struct PendingRead {
    pub id: u64,
    pub request: ReadRequest,
    pub start: Option<Item>,
}

#[derive(Default)]
enum ReadPhase {
    #[default]
    Idle,
    Prepared(PreparedScan),
    Loading {
        id: u64,
        request: ReadRequest,
    },
    Failed(LoadError),
}

#[derive(Default)]
pub(super) struct ReadState {
    pub displayed: Option<LoadedPage>,
    phase: ReadPhase,
    generation: u64,
}

impl ReadState {
    pub fn is_loading(&self) -> bool {
        matches!(self.phase, ReadPhase::Loading { .. })
    }
    pub fn error(&self) -> Option<LoadError> {
        match self.phase {
            ReadPhase::Failed(error) => Some(error),
            _ => None,
        }
    }
    pub fn prepared_scan(&self) -> Option<&PreparedScan> {
        match &self.phase {
            ReadPhase::Prepared(scan) => Some(scan),
            _ => None,
        }
    }

    pub fn reset(&mut self) {
        self.generation += 1;
        self.displayed = None;
        self.phase = ReadPhase::Idle;
    }

    pub fn query(&mut self, request: ReadRequest) -> Option<PendingRead> {
        if self.is_loading() || !matches!(request.operation, ReadOperation::Query { .. }) {
            return None;
        }
        Some(self.begin(request, None))
    }

    pub fn prepare_scan(&mut self, request: ReadRequest, start: Option<Item>) {
        if !self.is_loading() && matches!(request.operation, ReadOperation::Scan) {
            self.phase = ReadPhase::Prepared(PreparedScan { request, start });
        }
    }

    pub fn cancel_scan(&mut self) {
        if matches!(self.phase, ReadPhase::Prepared(_)) {
            self.phase = ReadPhase::Idle;
        }
    }

    pub fn confirm_scan(&mut self) -> Option<PendingRead> {
        if !matches!(self.phase, ReadPhase::Prepared(_)) {
            return None;
        }
        let ReadPhase::Prepared(scan) = std::mem::take(&mut self.phase) else {
            unreachable!()
        };
        Some(self.begin(scan.request, scan.start))
    }

    pub fn next_page(&mut self) -> Option<PendingRead> {
        if self.is_loading() {
            return None;
        }
        let displayed = self.displayed.as_ref()?;
        let start = displayed.page.next_key.clone()?;
        let request = displayed.request.clone();
        if matches!(request.operation, ReadOperation::Scan) {
            self.prepare_scan(request, Some(start));
            None
        } else {
            Some(self.begin(request, Some(start)))
        }
    }

    fn begin(&mut self, request: ReadRequest, start: Option<Item>) -> PendingRead {
        self.generation += 1;
        let id = self.generation;
        self.phase = ReadPhase::Loading {
            id,
            request: request.clone(),
        };
        PendingRead { id, request, start }
    }

    pub fn finish(&mut self, id: u64, result: Result<ItemPage, LoadError>) -> bool {
        if !matches!(self.phase, ReadPhase::Loading { id: current, .. } if current == id) {
            return false;
        }
        let ReadPhase::Loading { request, .. } = std::mem::take(&mut self.phase) else {
            unreachable!()
        };
        match result {
            Ok(page) => {
                self.displayed = Some(LoadedPage {
                    id,
                    selected_item: (!page.items.is_empty()).then_some(0),
                    request,
                    page,
                });
            }
            Err(error) => self.phase = ReadPhase::Failed(error),
        }
        true
    }

    pub fn select_item(&mut self, index: usize) {
        if let Some(displayed) = &mut self.displayed
            && index < displayed.page.items.len()
        {
            displayed.selected_item = Some(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dynamodb::{KeyDefinition, KeyTarget};
    use aws_sdk_dynamodb::types::{AttributeValue, ScalarAttributeType};
    use std::collections::HashMap;

    fn request(pk: &str) -> ReadRequest {
        ReadRequest {
            table: "Events".into(),
            target: KeyTarget {
                index_name: None,
                partition: KeyDefinition {
                    name: "pk".into(),
                    kind: ScalarAttributeType::S,
                },
                sort: None,
            },
            operation: ReadOperation::Query {
                partition: AttributeValue::S(pk.into()),
                sort: None,
            },
            limit: 50,
        }
    }

    fn page() -> ItemPage {
        ItemPage {
            items: vec![
                serde_json::json!({"pk": "one"}),
                serde_json::json!({"pk": "two"}),
            ],
            next_key: None,
        }
    }

    #[test]
    fn delayed_schema_indicator_only_appears_for_the_current_pending_request() {
        let mut state = BrowserState::default();
        let first = state.open("Events".into());
        assert!(!state.schema.loading_indicator_visible());
        let second = state.open("Books".into());
        assert!(!state.reveal_schema_loading(first));
        assert!(!state.schema.loading_indicator_visible());
        assert!(state.reveal_schema_loading(second));
        assert!(state.schema.loading_indicator_visible());
        assert!(state.finish_schema(second, Err(LoadError::Network)));
        assert!(!state.reveal_schema_loading(second));
        assert!(!state.schema.loading_indicator_visible());
        let third = state.open("Events".into());
        state.clear();
        assert!(!state.reveal_schema_loading(third));
    }

    #[test]
    fn old_results_cannot_finish_or_replace_a_new_request() {
        let mut state = ReadState::default();
        let old = state.query(request("old")).unwrap();
        state.reset();
        let new = state.query(request("new")).unwrap();
        assert!(!state.finish(old.id, Ok(page())));
        assert!(state.is_loading());
        assert!(state.finish(new.id, Ok(page())));
        let expected = state.displayed.as_ref().unwrap().request.summary();
        assert!(!state.finish(old.id, Err(LoadError::Network)));
        assert_eq!(
            state.displayed.as_ref().unwrap().request.summary(),
            expected
        );
        assert_eq!(state.error(), None);
    }

    #[test]
    fn failed_refresh_preserves_results_selection_and_original_query() {
        let mut state = ReadState::default();
        let first = state.query(request("original")).unwrap();
        assert!(state.finish(first.id, Ok(page())));
        state.select_item(1);
        let summary = state.displayed.as_ref().unwrap().request.summary();
        let refresh = state.query(request("replacement")).unwrap();
        assert_eq!(state.displayed.as_ref().unwrap().request.summary(), summary);
        assert!(state.finish(refresh.id, Err(LoadError::Network)));
        let displayed = state.displayed.as_ref().unwrap();
        assert_eq!(displayed.request.summary(), summary);
        assert_eq!(displayed.page.items, page().items);
        assert_eq!(displayed.selected_item, Some(1));
        assert_eq!(state.error(), Some(LoadError::Network));
    }

    #[test]
    fn changing_tables_clears_results_and_rejects_old_schema_and_reads() {
        let mut state = BrowserState::default();
        let old_schema = state.open("Events".into());
        let old_read = state.reads.query(request("old")).unwrap();
        state.reads.finish(old_read.id, Ok(page()));
        let pending = state.reads.query(request("pending")).unwrap();
        let new_schema = state.open("Books".into());
        assert!(state.reads.displayed.is_none());
        assert!(!state.reads.finish(pending.id, Ok(page())));
        assert!(!state.finish_schema(old_schema, Ok(TableSchema { targets: vec![] })));
        assert_eq!(state.schema.table(), Some("Books"));
        assert!(state.schema.is_loading());
        assert!(state.finish_schema(new_schema, Ok(TableSchema { targets: vec![] })));
        assert!(!state.schema.is_loading());
    }

    #[test]
    fn scans_require_confirmation_including_every_additional_page() {
        let mut state = ReadState::default();
        let mut scan = request("unused");
        scan.operation = ReadOperation::Scan;
        assert!(state.query(scan.clone()).is_none());
        assert!(state.confirm_scan().is_none());
        let generation = state.generation;
        state.prepare_scan(scan.clone(), None);
        assert_eq!(state.generation, generation);
        assert!(!state.is_loading());
        state.cancel_scan();
        assert!(state.confirm_scan().is_none());
        assert_eq!(state.generation, generation);
        state.prepare_scan(scan, None);
        let first = state.confirm_scan().unwrap();
        assert!(state.is_loading());
        assert!(state.confirm_scan().is_none());
        let cursor = HashMap::from([("pk".into(), AttributeValue::S("cursor".into()))]);
        state.finish(
            first.id,
            Ok(ItemPage {
                items: vec![],
                next_key: Some(cursor.clone()),
            }),
        );
        assert!(state.next_page().is_none());
        assert!(!state.is_loading());
        assert_eq!(state.prepared_scan().unwrap().start, Some(cursor.clone()));
        let next = state.confirm_scan().unwrap();
        assert_eq!(next.start, Some(cursor));
        assert!(state.is_loading());
    }

    #[test]
    fn query_pagination_uses_the_displayed_request_and_cursor() {
        let mut state = ReadState::default();
        let initial = state.query(request("original")).unwrap();
        let cursor = HashMap::from([("pk".into(), AttributeValue::S("cursor".into()))]);
        state.finish(
            initial.id,
            Ok(ItemPage {
                items: vec![],
                next_key: Some(cursor.clone()),
            }),
        );
        let next = state.next_page().unwrap();
        assert_eq!(next.start, Some(cursor));
        assert_eq!(next.request.summary(), request("original").summary());
        assert!(state.is_loading());
        assert!(state.next_page().is_none());
        state.finish(
            next.id,
            Ok(ItemPage {
                items: vec![],
                next_key: None,
            }),
        );
        assert!(state.next_page().is_none());
        assert_eq!(state.displayed.as_ref().unwrap().selected_item, None);
    }
}
