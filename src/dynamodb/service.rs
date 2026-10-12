use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use aws_config::{BehaviorVersion, meta::region::RegionProviderChain};
use aws_sdk_dynamodb::{
    Client,
    config::{Credentials, Region},
};
use tokio::{runtime::Runtime, sync::Mutex, task::JoinHandle};

use super::{Connection, Item, ItemPage, LoadError, ReadRequest, TableSchema, operations};

const LOCAL_ENDPOINT: &str = "http://127.0.0.1:8000";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Shared by the explorer and browser. AWS futures run on Tokio, never on GPUI's UI thread.
#[derive(Clone)]
pub struct DynamoDbService {
    inner: Arc<ServiceInner>,
}

struct ServiceInner {
    runtime: Option<Runtime>,
    clients: Arc<Mutex<HashMap<Connection, Client>>>,
}

impl Drop for ServiceInner {
    fn drop(&mut self) {
        // Closing the application must not block its UI thread waiting for network work.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

impl DynamoDbService {
    pub fn new() -> Result<Self, std::io::Error> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("dynamodb")
            .enable_all()
            .build()?;
        Ok(Self {
            inner: Arc::new(ServiceInner {
                runtime: Some(runtime),
                clients: Arc::new(Mutex::new(HashMap::new())),
            }),
        })
    }

    pub fn list_tables(&self, connection: Connection) -> Request<Vec<String>> {
        self.list_tables_with_page_size(connection, 100)
    }

    fn list_tables_with_page_size(
        &self,
        connection: Connection,
        page_size: i32,
    ) -> Request<Vec<String>> {
        self.request(connection, move |client| {
            operations::list_tables(client, page_size)
        })
    }

    pub fn describe_table(&self, connection: Connection, table: String) -> Request<TableSchema> {
        self.request(connection, move |client| {
            operations::describe_table(client, table)
        })
    }

    /// One page only; the caller owns pagination and scan confirmation.
    pub fn read_items(
        &self,
        connection: Connection,
        request: ReadRequest,
        start: Option<Item>,
    ) -> Request<ItemPage> {
        self.request(connection, move |client| {
            operations::read_items(client, request, start)
        })
    }

    fn request<T, F, Fut>(&self, connection: Connection, operation: F) -> Request<T>
    where
        T: Send + 'static,
        F: FnOnce(Client) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, LoadError>> + Send + 'static,
    {
        let clients = self.inner.clients.clone();
        let handle = self
            .inner
            .runtime
            .as_ref()
            .expect("service runtime exists")
            .spawn(async move {
                let result = tokio::time::timeout(REQUEST_TIMEOUT, async {
                    let client = client_for(&clients, connection).await;
                    operation(client).await
                })
                .await
                .map_err(|_| LoadError::Timeout)?;
                // Rebuild configuration on retry after a sign-in/profile failure.
                if matches!(
                    result,
                    Err(LoadError::MissingCredentials
                        | LoadError::InvalidConfiguration
                        | LoadError::CredentialsUnavailable
                        | LoadError::SignInRequired
                        | LoadError::ExpiredSession
                        | LoadError::InvalidCredentials)
                ) {
                    clients.lock().await.remove(&connection);
                }
                result
            });
        Request {
            handle,
            _service: self.inner.clone(),
        }
    }
}

async fn client_for(
    clients: &Mutex<HashMap<Connection, Client>>,
    connection: Connection,
) -> Client {
    let mut clients = clients.lock().await;
    if let Some(client) = clients.get(&connection) {
        return client.clone();
    }
    let client = match connection {
        Connection::Local => {
            let config = aws_sdk_dynamodb::Config::builder()
                .behavior_version(BehaviorVersion::latest())
                .endpoint_url(LOCAL_ENDPOINT)
                .region(Region::new("us-east-1"))
                .credentials_provider(Credentials::new("local", "local", None, None, "local"))
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
    clients.insert(connection, client.clone());
    client
}

/// Awaitable from GPUI. Tokio normally detaches on drop; this handle aborts instead.
/// It also keeps the runtime alive for the duration of the request.
pub struct Request<T> {
    handle: JoinHandle<Result<T, LoadError>>,
    _service: Arc<ServiceInner>,
}

impl<T> Future for Request<T> {
    type Output = Result<T, LoadError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.handle).poll(cx).map(|result| {
            result.unwrap_or_else(|error| {
                log::warn!("DynamoDB task failed: {error}");
                Err(LoadError::Unexpected)
            })
        })
    }
}

impl<T> Drop for Request<T> {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ReadOperation, SortOperator};
    use super::*;
    use aws_sdk_dynamodb::types::AttributeValue;

    #[test]
    #[ignore = "requires running DynamoDB Local with scripts/seed fixtures"]
    fn loads_seeded_local_tables() {
        let service = DynamoDbService::new().unwrap();
        service.inner.runtime.as_ref().unwrap().block_on(async {
            let tables = service
                .list_tables_with_page_size(Connection::Local, 1)
                .await
                .unwrap();
            for expected in ["Books", "EmptyTable", "Events"] {
                assert!(tables.iter().any(|table| table == expected));
            }
            assert!(tables.windows(2).all(|pair| pair[0] < pair[1]));
        });
    }

    #[test]
    fn clients_are_cached_but_sign_in_failures_allow_configuration_reload() {
        let service = DynamoDbService::new().unwrap();
        service.inner.runtime.as_ref().unwrap().block_on(async {
            for _ in 0..2 {
                service
                    .request(Connection::Local, |_| async { Ok(()) })
                    .await
                    .unwrap();
                assert_eq!(service.inner.clients.lock().await.len(), 1);
            }
            let failed = service
                .request(Connection::Local, |_| async {
                    Err::<(), _>(LoadError::MissingCredentials)
                })
                .await;
            assert_eq!(failed, Err(LoadError::MissingCredentials));
            assert!(service.inner.clients.lock().await.is_empty());
            service
                .request(Connection::Local, |_| async { Ok(()) })
                .await
                .unwrap();
            assert_eq!(service.inner.clients.lock().await.len(), 1);
        });
    }

    #[test]
    fn dropping_request_cancels_work() {
        struct OnDrop(Option<tokio::sync::oneshot::Sender<()>>);
        impl Drop for OnDrop {
            fn drop(&mut self) {
                if let Some(sender) = self.0.take() {
                    let _ = sender.send(());
                }
            }
        }
        let service = DynamoDbService::new().unwrap();
        let runtime = service.inner.runtime.as_ref().unwrap();
        runtime.block_on(async {
            let (started, ready) = tokio::sync::oneshot::channel();
            let (dropped, cancelled) = tokio::sync::oneshot::channel();
            let handle = runtime.spawn(async move {
                let _guard = OnDrop(Some(dropped));
                let _ = started.send(());
                std::future::pending::<Result<(), LoadError>>().await
            });
            let request = Request {
                handle,
                _service: service.inner.clone(),
            };
            ready.await.unwrap();
            drop(request);
            tokio::time::timeout(Duration::from_secs(1), cancelled)
                .await
                .unwrap()
                .unwrap();
        });
    }

    #[test]
    #[ignore = "requires seeded DynamoDB Local"]
    fn scans_queries_and_pagination_work_locally() {
        let service = DynamoDbService::new().unwrap();
        service.inner.runtime.as_ref().unwrap().block_on(async {
            let schema = service
                .describe_table(Connection::Local, "Events".into())
                .await
                .unwrap();
            let request = ReadRequest {
                table: "Events".into(),
                target: schema.targets[0].clone(),
                operation: ReadOperation::Scan,
                limit: 5,
            };
            let first = service
                .read_items(Connection::Local, request.clone(), None)
                .await
                .unwrap();
            assert_eq!(first.items.len(), 5);
            let second = service
                .read_items(Connection::Local, request, first.next_key)
                .await
                .unwrap();
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
                    service
                        .read_items(Connection::Local, request, None)
                        .await
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
                service
                    .read_items(Connection::Local, request, None)
                    .await
                    .unwrap()
                    .items
                    .len(),
                3
            );
            let empty = service
                .describe_table(Connection::Local, "EmptyTable".into())
                .await
                .unwrap();
            assert!(
                service
                    .read_items(
                        Connection::Local,
                        ReadRequest {
                            table: "EmptyTable".into(),
                            target: empty.targets[0].clone(),
                            operation: ReadOperation::Scan,
                            limit: 50
                        },
                        None
                    )
                    .await
                    .unwrap()
                    .items
                    .is_empty()
            );
            assert_eq!(service.inner.clients.lock().await.len(), 1);
        });
    }
}
