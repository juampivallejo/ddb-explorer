//! Connection choices for the read-only explorer.
//!
//! The AWS SDK client will live here, independently of the GPUI views.
//! Local connections must use an explicit endpoint and dummy credentials.
//! Terminal connections will use the SDK's default credential provider chain.

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
