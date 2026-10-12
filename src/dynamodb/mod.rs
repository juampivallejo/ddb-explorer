//! Read-only DynamoDB access, independent of GPUI.

mod error;
mod items;
pub use error::LoadError;
pub use items::{
    Item, ItemPage, KeyTarget, ReadOperation, ReadRequest, SortOperator, TableSchema,
    describe_table, read_items,
};

use std::time::Duration;

use aws_config::{BehaviorVersion, meta::region::RegionProviderChain};
use aws_sdk_dynamodb::{
    Client,
    config::{Credentials, Region},
};

const LOCAL_ENDPOINT: &str = "http://127.0.0.1:8000";

/// Run on GPUI's background executor: the SDK requires a Tokio reactor.
pub fn load_tables(connection: Connection) -> Result<Vec<String>, LoadError> {
    load_tables_with_page_size(connection, 100)
}

fn load_tables_with_page_size(
    connection: Connection,
    page_size: i32,
) -> Result<Vec<String>, LoadError> {
    run_request(connection, |client| async move {
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
    })
}

fn run_request<T, F, Fut>(connection: Connection, operation: F) -> Result<T, LoadError>
where
    F: FnOnce(Client) -> Fut,
    Fut: std::future::Future<Output = Result<T, LoadError>>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            log::warn!("Could not start AWS runtime: {error}");
            LoadError::Unexpected
        })?;
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(30), async {
            let client = match connection {
                Connection::Local => {
                    let config = aws_sdk_dynamodb::Config::builder()
                        .behavior_version(BehaviorVersion::latest())
                        .endpoint_url(LOCAL_ENDPOINT)
                        .region(Region::new("us-east-1"))
                        .credentials_provider(Credentials::new(
                            "local", "local", None, None, "local",
                        ))
                        .build();
                    Client::from_conf(config)
                }
                Connection::Terminal => {
                    let config = aws_config::defaults(BehaviorVersion::latest())
                        .region(RegionProviderChain::default_provider().or_else("us-east-1"))
                        .load()
                        .await;
                    Client::new(&config)
                }
            };
            operation(client).await
        })
        .await
        .map_err(|_| LoadError::Timeout)?
    })
}

fn sdk_result<T, E>(result: Result<T, aws_sdk_dynamodb::error::SdkError<E>>) -> Result<T, LoadError>
where
    E: std::error::Error + aws_sdk_dynamodb::error::ProvideErrorMetadata + 'static,
{
    result.map_err(|error| {
        log::warn!(
            "DynamoDB request failed: {}",
            aws_sdk_dynamodb::error::DisplayErrorContext(&error)
        );
        error::classify(&error)
    })
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Connection {
    #[default]
    Local,
    Terminal,
}

impl Connection {
    pub const ALL: [Self; 2] = [Self::Local, Self::Terminal];

    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "Local DynamoDB",
            Self::Terminal => "Terminal session",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Local => "http://127.0.0.1:8000 · us-east-1",
            Self::Terminal => "AWS credentials inherited from the launching terminal",
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum ReadMode {
    #[default]
    Scan,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires running DynamoDB Local with scripts/seed fixtures"]
    fn loads_seeded_local_tables() {
        // One table per page ensures the test exercises pagination.
        let tables = load_tables_with_page_size(Connection::Local, 1)
            .expect("local table discovery should succeed");
        for expected in ["Books", "EmptyTable", "Events"] {
            assert!(
                tables.iter().any(|table| table == expected),
                "missing {expected}: {tables:?}"
            );
        }
        assert!(tables.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
