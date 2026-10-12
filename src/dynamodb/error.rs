use std::error::Error;

use aws_credential_types::provider::error::{CredentialsError, TokenError};
use aws_sdk_dynamodb::error::{ProvideErrorMetadata, SdkError};

use super::Connection;

/// Only curated messages reach the UI; SDK details go to the terminal log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadError {
    MissingCredentials,
    InvalidConfiguration,
    CredentialsUnavailable,
    SignInRequired,
    ExpiredSession,
    InvalidCredentials,
    AccessDenied,
    Network,
    Timeout,
    Throttled,
    ServiceUnavailable,
    InvalidRequest,
    TableNotFound,
    Unexpected,
}

impl LoadError {
    pub fn title(self) -> &'static str {
        match self {
            Self::MissingCredentials => "AWS credentials not found",
            Self::InvalidConfiguration => "AWS profile needs attention",
            Self::CredentialsUnavailable => "Could not load AWS credentials",
            Self::SignInRequired => "Sign in to AWS",
            Self::ExpiredSession => "AWS session expired",
            Self::InvalidCredentials => "AWS credentials were rejected",
            Self::AccessDenied => "Access denied",
            Self::Network => "Could not reach DynamoDB",
            Self::Timeout => "Connection timed out",
            Self::Throttled => "Too many requests",
            Self::ServiceUnavailable => "DynamoDB is temporarily unavailable",
            Self::InvalidRequest => "Check your query",
            Self::TableNotFound => "Table or index not found",
            Self::Unexpected => "Could not complete the request",
        }
    }

    pub fn message(self, connection: Connection) -> &'static str {
        match self {
            Self::MissingCredentials => {
                "Sign in with assume <profile>, then restart the app from that terminal. You can also use Local DynamoDB."
            }
            Self::InvalidConfiguration => {
                "Check your AWS profile configuration and AWS_PROFILE, then refresh."
            }
            Self::CredentialsUnavailable => {
                "Check your AWS profile or sign in again, then refresh. If you use assume, restart the app from that terminal."
            }
            Self::SignInRequired | Self::ExpiredSession => {
                "Sign in again, then refresh. If you use assume, restart the app from that terminal."
            }
            Self::InvalidCredentials => {
                "Check your selected profile or assume the role again, then restart the app from that terminal."
            }
            Self::AccessDenied => {
                "Your role cannot perform this read. Ask for the required DynamoDB read permission or use a different role."
            }
            Self::Network if connection == Connection::Local => {
                "Start the local database with docker compose up -d, then refresh."
            }
            Self::Network => "Check your internet connection or VPN, then refresh.",
            Self::Timeout if connection == Connection::Local => {
                "Check that the local database is running, then refresh."
            }
            Self::Timeout => "Check your connection and AWS sign-in, then refresh.",
            Self::Throttled | Self::ServiceUnavailable => "Wait a moment, then refresh.",
            Self::InvalidRequest => {
                "Check the key values, types, and sort-key condition, then retry."
            }
            Self::TableNotFound => "Refresh the table list and select the table again.",
            Self::Unexpected => "Try again. More details are available in the terminal.",
        }
    }
}

pub(super) fn classify<E: Error + ProvideErrorMetadata + 'static>(
    error: &SdkError<E>,
) -> LoadError {
    // Credential failures are often nested inside an SDK construction/dispatch error.
    let mut source: Option<&(dyn Error + 'static)> = Some(error);
    let mut credential_error = None;
    while let Some(cause) = source {
        if let Some(error) = cause.downcast_ref::<CredentialsError>() {
            credential_error = Some(match error {
                CredentialsError::CredentialsNotLoaded(_) => LoadError::MissingCredentials,
                CredentialsError::InvalidConfiguration(_) => LoadError::InvalidConfiguration,
                CredentialsError::ProviderTimedOut(_) => LoadError::Timeout,
                _ => LoadError::CredentialsUnavailable,
            });
        }
        if let Some(error) = cause.downcast_ref::<TokenError>() {
            credential_error = Some(match error {
                TokenError::TokenNotLoaded(_) => LoadError::SignInRequired,
                TokenError::InvalidConfiguration(_) => LoadError::InvalidConfiguration,
                TokenError::ProviderTimedOut(_) => LoadError::Timeout,
                _ => LoadError::CredentialsUnavailable,
            });
        }
        source = cause.source();
    }
    if let Some(error) = credential_error {
        return error;
    }

    if let Some(service) = error.as_service_error() {
        return classify_service_code(service.code());
    }
    match error {
        SdkError::TimeoutError(_) => LoadError::Timeout,
        SdkError::DispatchFailure(failure) if failure.is_timeout() => LoadError::Timeout,
        SdkError::DispatchFailure(_) => LoadError::Network,
        _ => LoadError::Unexpected,
    }
}

fn classify_service_code(code: Option<&str>) -> LoadError {
    match code {
        Some("ExpiredToken" | "ExpiredTokenException" | "RequestExpired") => {
            LoadError::ExpiredSession
        }
        Some(
            "UnrecognizedClientException"
            | "InvalidClientTokenId"
            | "InvalidSignatureException"
            | "SignatureDoesNotMatch",
        ) => LoadError::InvalidCredentials,
        Some("AccessDenied" | "AccessDeniedException" | "UnauthorizedOperation") => {
            LoadError::AccessDenied
        }
        Some(
            "Throttling"
            | "ThrottlingException"
            | "RequestLimitExceeded"
            | "LimitExceededException",
        ) => LoadError::Throttled,
        Some(
            "InternalServerError"
            | "InternalServerErrorException"
            | "ServiceUnavailable"
            | "ServiceUnavailableException",
        ) => LoadError::ServiceUnavailable,
        Some("ValidationException") => LoadError::InvalidRequest,
        Some("ResourceNotFoundException") => LoadError::TableNotFound,
        _ => LoadError::Unexpected,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use aws_sdk_dynamodb::error::ConnectorError;

    use super::*;
    use aws_sdk_dynamodb::operation::list_tables::ListTablesError;
    type TestSdkError = SdkError<ListTablesError>;

    #[test]
    fn recognizes_nested_missing_credentials() {
        let error = TestSdkError::construction_failure(CredentialsError::provider_error(
            CredentialsError::not_loaded_no_source(),
        ));
        assert_eq!(classify(&error), LoadError::MissingCredentials);
    }

    #[test]
    fn distinguishes_credential_configuration_and_sign_in() {
        let cases = [
            (
                CredentialsError::invalid_configuration("bad profile"),
                LoadError::InvalidConfiguration,
            ),
            (
                CredentialsError::provider_timed_out(Duration::from_secs(1)),
                LoadError::Timeout,
            ),
            (
                CredentialsError::provider_error(TokenError::not_loaded("no cached SSO token")),
                LoadError::SignInRequired,
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(
                classify(&TestSdkError::construction_failure(error)),
                expected
            );
        }
    }

    #[test]
    fn distinguishes_aws_service_errors() {
        for (code, expected) in [
            ("AccessDeniedException", LoadError::AccessDenied),
            ("ExpiredTokenException", LoadError::ExpiredSession),
            ("UnrecognizedClientException", LoadError::InvalidCredentials),
            ("ThrottlingException", LoadError::Throttled),
            ("InternalServerError", LoadError::ServiceUnavailable),
            ("NewUnexpectedError", LoadError::Unexpected),
        ] {
            assert_eq!(classify_service_code(Some(code)), expected);
        }
        assert_eq!(classify_service_code(None), LoadError::Unexpected);
    }

    #[test]
    fn distinguishes_network_timeout_and_unknown_failures() {
        let network = TestSdkError::dispatch_failure(ConnectorError::io(
            std::io::Error::other("connection refused").into(),
        ));
        assert_eq!(classify(&network), LoadError::Network);
        assert_eq!(
            classify(&TestSdkError::timeout_error("timeout")),
            LoadError::Timeout
        );
        assert_eq!(
            classify(&TestSdkError::construction_failure("raw internal details")),
            LoadError::Unexpected
        );
        assert_ne!(
            LoadError::Network.message(Connection::Local),
            LoadError::Network.message(Connection::Terminal)
        );
        assert!(
            !LoadError::Unexpected
                .message(Connection::Terminal)
                .contains("raw internal details")
        );
    }
}
