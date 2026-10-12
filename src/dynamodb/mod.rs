//! Read-only DynamoDB access, independent of GPUI.
mod error;
mod json;
mod models;
mod operations;
mod service;

pub use error::LoadError;
pub use models::{
    Item, ItemPage, KeyDefinition, KeyTarget, ReadOperation, ReadRequest, SortOperator, TableSchema,
};
pub use service::DynamoDbService;

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
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
