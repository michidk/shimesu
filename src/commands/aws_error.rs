//! Maps AWS SDK errors to typed `ShimesuError` values with actionable messages.

use crate::error::ShimesuError;
use aws_smithy_runtime_api::client::result::SdkError;
use aws_smithy_types::error::display::DisplayErrorContext;
use aws_smithy_types::error::metadata::ProvideErrorMetadata;
use std::error::Error as StdError;
use std::fmt::Debug;

const THROTTLING_ERROR_CODES: &[&str] = &[
    "Throttling",
    "ThrottlingException",
    "ThrottledException",
    "RequestThrottled",
    "RequestThrottledException",
    "TooManyRequestsException",
    "RequestLimitExceeded",
    "SlowDown",
    "ProvisionedThroughputExceededException",
];

const AUTH_ERROR_CODES: &[&str] = &[
    "AccessDenied",
    "AccessDeniedException",
    "AuthFailure",
    "ExpiredToken",
    "ExpiredTokenException",
    "InvalidAccessKeyId",
    "InvalidClientTokenId",
    "MissingAuthenticationToken",
    "NotAuthorized",
    "SignatureDoesNotMatch",
    "UnauthorizedException",
    "UnauthorizedOperation",
    "UnrecognizedClientException",
];

/// Structured service error code, when the SDK error carries one.
pub(crate) fn sdk_error_code<E, R>(error: &SdkError<E, R>) -> Option<&str>
where
    E: ProvideErrorMetadata,
{
    error
        .as_service_error()
        .and_then(ProvideErrorMetadata::code)
}

/// Full error text including the source chain (service code and message),
/// unlike `SdkError`'s bare `Display` which only prints the outer layer.
pub(crate) fn sdk_error_text<E, R>(error: &SdkError<E, R>) -> String
where
    E: StdError + 'static,
    R: Debug,
{
    DisplayErrorContext(error).to_string()
}

pub(crate) fn map_sdk_error<E, R>(action: &str, error: &SdkError<E, R>) -> ShimesuError
where
    E: ProvideErrorMetadata + StdError + 'static,
    R: Debug,
{
    map_aws_error_with_code(action, sdk_error_code(error), &sdk_error_text(error))
}

pub(crate) fn map_stack_describe_sdk_error<E, R>(
    stack_name: &str,
    error: &SdkError<E, R>,
) -> ShimesuError
where
    E: ProvideErrorMetadata + StdError + 'static,
    R: Debug,
{
    map_stack_describe_error_with_code(stack_name, sdk_error_code(error), &sdk_error_text(error))
}

pub(crate) fn map_site_delete_sdk_error<E, R>(slug: &str, error: &SdkError<E, R>) -> ShimesuError
where
    E: ProvideErrorMetadata + StdError + 'static,
    R: Debug,
{
    map_site_delete_error(slug, sdk_error_code(error), &sdk_error_text(error))
}

pub fn map_aws_error_with_code(action: &str, code: Option<&str>, error_text: &str) -> ShimesuError {
    if let Some(code) = code {
        if THROTTLING_ERROR_CODES.contains(&code) {
            return ShimesuError::AwsThrottled(format!("{action}: {error_text}"));
        }
        if AUTH_ERROR_CODES.contains(&code) {
            return ShimesuError::AwsAuth(format!("{action}: {error_text}"));
        }
    }
    map_aws_error(action, error_text)
}

pub fn map_aws_error(action: &str, error_text: &str) -> ShimesuError {
    let lower = error_text.to_ascii_lowercase();

    if lower.contains("throttl") {
        ShimesuError::AwsThrottled(error_text.to_string())
    } else if lower.contains("accessdenied")
        || lower.contains("access denied")
        || lower.contains("expiredtoken")
        || lower.contains("expired token")
        || lower.contains("unrecognizedclient")
        || lower.contains("invalidclienttokenid")
        || lower.contains("credential")
        || lower.contains("not authorized")
        || lower.contains("unauthorized")
    {
        ShimesuError::AwsAuth(format!("{action}: {error_text}"))
    } else {
        ShimesuError::Generic(format!("{action}: {error_text}"))
    }
}

pub fn map_stack_describe_error(stack_name: &str, error_text: &str) -> ShimesuError {
    map_stack_describe_error_with_code(stack_name, None, error_text)
}

pub fn map_stack_describe_error_with_code(
    stack_name: &str,
    code: Option<&str>,
    error_text: &str,
) -> ShimesuError {
    let lower = error_text.to_ascii_lowercase();
    if lower.contains("does not exist") || lower.contains("not found") {
        ShimesuError::NotFound(format!("Stack '{}' not found", stack_name))
    } else {
        map_aws_error_with_code("Failed to describe stack", code, error_text)
    }
}

pub fn map_site_delete_error(slug: &str, code: Option<&str>, error_text: &str) -> ShimesuError {
    if code == Some("ConditionalCheckFailedException")
        || error_text
            .to_ascii_lowercase()
            .contains("conditionalcheckfailed")
    {
        ShimesuError::NotFound(format!("Site '{slug}' not found"))
    } else {
        map_aws_error_with_code("Failed to delete site", code, error_text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_aws_error_uses_throttled_variant() {
        let error = map_aws_error("Failed to list sites", "Rate exceeded: throttling");

        assert!(matches!(error, ShimesuError::AwsThrottled(_)));
    }

    #[test]
    fn map_aws_error_uses_auth_variant() {
        let error = map_aws_error(
            "Failed to list sites",
            "AccessDenied: not authorized to perform this action",
        );

        assert!(matches!(error, ShimesuError::AwsAuth(_)));
    }

    #[test]
    fn structured_throttling_code_wins_even_when_text_is_opaque() {
        let error = map_aws_error_with_code(
            "Failed to list sites",
            Some("ThrottlingException"),
            "service error",
        );

        assert!(matches!(error, ShimesuError::AwsThrottled(_)));
    }

    #[test]
    fn structured_auth_code_wins_even_when_text_is_opaque() {
        let error = map_aws_error_with_code(
            "Failed to upload deployment file",
            Some("AccessDeniedException"),
            "service error",
        );

        assert!(
            matches!(error, ShimesuError::AwsAuth(message) if message.contains("Failed to upload deployment file"))
        );
    }

    #[test]
    fn unknown_code_falls_back_to_text_heuristics() {
        let throttled = map_aws_error_with_code(
            "Failed to scan",
            Some("Whatever"),
            "Rate exceeded: throttling",
        );
        let generic = map_aws_error_with_code("Failed to scan", Some("Whatever"), "boom");

        assert!(matches!(throttled, ShimesuError::AwsThrottled(_)));
        assert!(matches!(generic, ShimesuError::Generic(message) if message.contains("boom")));
    }

    #[test]
    fn missing_code_falls_back_to_text_heuristics() {
        let error = map_aws_error_with_code("Failed to scan", None, "dispatch failure: credential");

        assert!(matches!(error, ShimesuError::AwsAuth(_)));
    }

    #[test]
    fn conditional_check_failure_code_maps_to_site_not_found() {
        let error = map_site_delete_error(
            "docs",
            Some("ConditionalCheckFailedException"),
            "service error",
        );

        assert!(
            matches!(error, ShimesuError::NotFound(message) if message == "Site 'docs' not found")
        );
    }

    #[test]
    fn stack_describe_missing_stack_maps_to_not_found() {
        let error = map_stack_describe_error_with_code(
            "shimesu",
            Some("ValidationError"),
            "ValidationError: Stack with id shimesu does not exist",
        );

        assert!(
            matches!(error, ShimesuError::NotFound(message) if message == "Stack 'shimesu' not found")
        );
    }
}
