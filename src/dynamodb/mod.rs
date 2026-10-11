//! Read-only DynamoDB access, independent of GPUI.

mod error;
pub use error::LoadError;

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
                    // Do not consult inherited profiles or credentials for local access.
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

            let mut tables = Vec::new();
            let mut pages = client
                .list_tables()
                .limit(page_size)
                .into_paginator()
                .send();
            while let Some(page) = pages.next().await {
                let page = page.map_err(|error| {
                    log::warn!(
                        "DynamoDB table discovery failed: {}",
                        aws_sdk_dynamodb::error::DisplayErrorContext(&error)
                    );
                    error::classify(&error)
                })?;
                tables.extend(page.table_names().iter().cloned());
            }
            tables.sort();
            tables.dedup();
            Ok(tables)
        })
        .await
        .map_err(|_| {
            log::warn!("DynamoDB table discovery timed out after 30 seconds");
            LoadError::Timeout
        })?
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
