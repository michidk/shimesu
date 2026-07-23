use super::{
    interpret_delete_poll, map_delete_stack_error, should_wait_for_delete, DeleteRequestOutcome,
    StackDestroyInput, StackDestroyOutput,
};
use crate::config::Config;
use crate::error::ShimesuError;
use aws_sdk_cloudformation::types::StackStatus;
use serde_json::json;

fn test_config() -> Config {
    Config {
        stack_name: "test-stack".to_string(),
        region: Some("us-east-1".to_string()),
        profile: None,
        json: false,
        yes: false,
    }
}

#[test]
fn stack_destroy_input_parse_returns_usage_error_when_confirm_is_false() {
    let result = StackDestroyInput::parse(false);
    assert!(result.is_err());
    match result {
        Err(ShimesuError::Usage(msg)) => {
            assert!(msg.contains("--confirm"));
        }
        _ => panic!("Expected Usage error"),
    }
}

#[test]
fn stack_destroy_input_parse_succeeds_when_confirm_is_true() {
    let result = StackDestroyInput::parse(true);

    match result {
        Ok(token) => assert_eq!(token, StackDestroyInput { _confirmed: () }),
        Err(error) => panic!("expected success, got {error}"),
    }
}

#[test]
fn interpret_delete_poll_returns_true_for_delete_complete() {
    let result = interpret_delete_poll(Ok(StackStatus::DeleteComplete));
    match result {
        Ok(true) => {}
        other => panic!("expected Ok(true), got {other:?}"),
    }
}

#[test]
fn interpret_delete_poll_returns_false_for_delete_in_progress() {
    let result = interpret_delete_poll(Ok(StackStatus::DeleteInProgress));
    match result {
        Ok(false) => {}
        other => panic!("expected Ok(false), got {other:?}"),
    }
}

#[test]
fn interpret_delete_poll_returns_false_for_create_complete() {
    let result = interpret_delete_poll(Ok(StackStatus::CreateComplete));
    match result {
        Ok(false) => {}
        other => panic!("expected Ok(false), got {other:?}"),
    }
}

#[test]
fn interpret_delete_poll_returns_false_for_update_complete() {
    let result = interpret_delete_poll(Ok(StackStatus::UpdateComplete));
    match result {
        Ok(false) => {}
        other => panic!("expected Ok(false), got {other:?}"),
    }
}

#[test]
fn interpret_delete_poll_returns_false_for_rollback_complete() {
    let result = interpret_delete_poll(Ok(StackStatus::RollbackComplete));
    match result {
        Ok(false) => {}
        other => panic!("expected Ok(false), got {other:?}"),
    }
}

#[test]
fn interpret_delete_poll_returns_false_for_unknown_future_status() {
    let result = interpret_delete_poll(Ok(StackStatus::from("FUTURE_DELETE_STATUS")));
    match result {
        Ok(false) => {}
        other => panic!("expected Ok(false), got {other:?}"),
    }
}

#[test]
fn interpret_delete_poll_returns_error_for_delete_failed() {
    let result = interpret_delete_poll(Ok(StackStatus::DeleteFailed));
    assert!(result.is_err());
}

#[test]
fn interpret_delete_poll_returns_true_for_not_found_error() {
    let result = interpret_delete_poll(Err(ShimesuError::NotFound("stack not found".to_string())));
    match result {
        Ok(true) => {}
        other => panic!("expected Ok(true) for NotFound, got {other:?}"),
    }
}

#[test]
fn interpret_delete_poll_propagates_other_errors() {
    let result = interpret_delete_poll(Err(ShimesuError::Generic("other error".to_string())));
    assert!(result.is_err());
}

#[test]
fn map_delete_stack_error_returns_not_found_when_stack_missing() {
    let outcome = map_delete_stack_error(
        "test-stack",
        None,
        "Stack with id test-stack does not exist",
    );

    assert!(matches!(outcome, Ok(DeleteRequestOutcome::AlreadyAbsent)));
}

#[test]
fn map_delete_stack_error_delegates_auth_failures() {
    let error = map_delete_stack_error("test-stack", Some("AccessDenied"), "service error");

    assert!(
        matches!(error, Err(ShimesuError::AwsAuth(message)) if message.contains("Failed to delete stack"))
    );
}

#[test]
fn should_wait_for_delete_returns_true_only_when_delete_started() {
    assert!(should_wait_for_delete(DeleteRequestOutcome::Started));
    assert!(!should_wait_for_delete(DeleteRequestOutcome::AlreadyAbsent));
}

#[test]
fn stack_destroy_output_serializes_to_json() {
    let output = StackDestroyOutput {
        stack_name: "test-stack".to_string(),
        region: "us-east-1".to_string(),
        deleted: true,
        already_absent: false,
        retained_data: true,
        retained_data_verified: true,
        retained_certificate: true,
        retained_certificate_verified: true,
        certificate_stack_name: "test-stack-certificate".to_string(),
    };
    let json = serde_json::to_value(&output)
        .unwrap_or_else(|error| panic!("serialization failed: {error}"));

    assert_eq!(
        json,
        json!({
            "stack_name": "test-stack",
            "region": "us-east-1",
            "deleted": true,
            "already_absent": false,
            "retained_data": true,
            "retained_data_verified": true,
            "retained_certificate": true,
            "retained_certificate_verified": true,
            "certificate_stack_name": "test-stack-certificate"
        })
    );
}

#[test]
fn stack_destroy_output_human_rows_are_stable() {
    let output = StackDestroyOutput {
        stack_name: "test-stack".to_string(),
        region: "us-east-1".to_string(),
        deleted: true,
        already_absent: false,
        retained_data: true,
        retained_data_verified: true,
        retained_certificate: true,
        retained_certificate_verified: true,
        certificate_stack_name: "test-stack-certificate".to_string(),
    };

    assert_eq!(
        output.human_rows(),
        [
            ("Stack Name", "test-stack"),
            ("Region", "us-east-1"),
            ("Deleted", "true"),
            ("Already Absent", "false"),
            ("Retained Data", "true"),
            ("Retained Data Verified", "true"),
            ("Retained Certificate", "true"),
            ("Retained Certificate Verified", "true"),
            ("Certificate Stack", "test-stack-certificate"),
        ]
    );
}

#[test]
fn stack_destroy_output_retained_data_warning_is_stable() {
    let output = StackDestroyOutput {
        stack_name: "test-stack".to_string(),
        region: "us-east-1".to_string(),
        deleted: true,
        already_absent: false,
        retained_data: true,
        retained_data_verified: true,
        retained_certificate: true,
        retained_certificate_verified: true,
        certificate_stack_name: "test-stack-certificate".to_string(),
    };

    assert_eq!(
        output.retained_data_warning(),
        Some("Retained S3 bucket, DynamoDB table, certificate stack 'test-stack-certificate', and ACM certificate remain in your AWS account. Use 'shimesu stack teardown --confirm-data-loss' for permanent removal.".to_string())
    );
}

#[test]
fn absent_stack_output_does_not_claim_deleted_or_retained_resources() {
    let output = StackDestroyOutput::from_config(
        &test_config(),
        DeleteRequestOutcome::AlreadyAbsent,
        false,
        true,
    );

    assert!(!output.deleted);
    assert!(output.already_absent);
    assert!(output.retained_data);
    assert!(!output.retained_data_verified);
    assert!(!output.retained_certificate);
    assert!(output.retained_certificate_verified);
    assert_eq!(
        output.success_message(),
        "Stack 'test-stack' was already absent"
    );
    assert!(output
        .retained_data_warning()
        .is_some_and(|warning| warning.contains("may remain")));
}

#[test]
fn unverified_certificate_state_does_not_hide_regional_deletion() {
    let output = StackDestroyOutput::from_config(
        &test_config(),
        DeleteRequestOutcome::Started,
        false,
        false,
    );

    assert!(output.deleted);
    assert!(!output.already_absent);
    assert!(!output.retained_certificate);
    assert!(!output.retained_certificate_verified);
    assert!(output
        .retained_data_warning()
        .is_some_and(|warning| warning.contains("could not be verified")));
}

#[test]
fn absent_stack_warning_reports_both_unverified_resource_classes() {
    let output = StackDestroyOutput::from_config(
        &test_config(),
        DeleteRequestOutcome::AlreadyAbsent,
        false,
        false,
    );

    let warning = output
        .retained_data_warning()
        .unwrap_or_else(|| panic!("expected retained-resource uncertainty warning"));
    assert!(warning.contains("S3 bucket and DynamoDB table data may remain"));
    assert!(warning.contains("certificate retention also could not be verified"));
}
