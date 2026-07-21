use super::*;
use crate::error::ShimesuError;
use backend::fake::FakeBackend;

#[tokio::test]
async fn test_all_checks_pass() {
    let backend = FakeBackend::all_pass();
    let output = run_doctor_checks(&backend, "test-stack").await;

    assert!(output.healthy);
    assert_eq!(output.checks.len(), 6);
    assert_eq!(output.checks[0].name, DoctorCheckName::Credentials);
    assert_eq!(output.checks[0].status, DoctorCheckStatus::Passed);
    assert_eq!(output.checks[1].name, DoctorCheckName::StackExists);
    assert_eq!(output.checks[1].status, DoctorCheckStatus::Passed);
    assert_eq!(output.checks[2].name, DoctorCheckName::StackStatus);
    assert_eq!(output.checks[2].status, DoctorCheckStatus::Passed);
    assert_eq!(output.checks[3].name, DoctorCheckName::StackOutputs);
    assert_eq!(output.checks[3].status, DoctorCheckStatus::Passed);
    assert_eq!(output.checks[4].name, DoctorCheckName::S3Bucket);
    assert_eq!(output.checks[4].status, DoctorCheckStatus::Passed);
    assert_eq!(output.checks[5].name, DoctorCheckName::DynamodbTable);
    assert_eq!(output.checks[5].status, DoctorCheckStatus::Passed);
}

#[tokio::test]
async fn test_credentials_failure_still_reports_six_checks() {
    let backend = FakeBackend::credentials_fail();
    let output = run_doctor_checks(&backend, "test-stack").await;

    assert!(!output.healthy);
    assert_eq!(output.checks.len(), 6);
    assert_eq!(output.checks[0].name, DoctorCheckName::Credentials);
    assert_eq!(output.checks[0].status, DoctorCheckStatus::Failed);
    assert!(output.checks[0].message.is_some());
    assert_eq!(output.checks[1].name, DoctorCheckName::StackExists);
    assert_eq!(output.checks[1].status, DoctorCheckStatus::Passed);
    assert_eq!(output.checks[2].name, DoctorCheckName::StackStatus);
    assert_eq!(output.checks[2].status, DoctorCheckStatus::Passed);
    assert_eq!(output.checks[3].name, DoctorCheckName::StackOutputs);
    assert_eq!(output.checks[3].status, DoctorCheckStatus::Passed);
    assert_eq!(output.checks[4].name, DoctorCheckName::S3Bucket);
    assert_eq!(output.checks[4].status, DoctorCheckStatus::Passed);
    assert_eq!(output.checks[5].name, DoctorCheckName::DynamodbTable);
    assert_eq!(output.checks[5].status, DoctorCheckStatus::Passed);
}

#[tokio::test]
async fn test_missing_stack_skips_remaining() {
    let backend = FakeBackend::no_stack();
    let output = run_doctor_checks(&backend, "test-stack").await;

    assert!(!output.healthy);
    assert_eq!(output.checks.len(), 6);
    assert_eq!(output.checks[0].status, DoctorCheckStatus::Passed); // credentials
    assert_eq!(output.checks[1].status, DoctorCheckStatus::Failed); // stack_exists
    assert_eq!(output.checks[2].status, DoctorCheckStatus::NotChecked); // stack_status
    assert_eq!(output.checks[3].status, DoctorCheckStatus::NotChecked); // stack_outputs
    assert_eq!(output.checks[4].status, DoctorCheckStatus::NotChecked); // s3_bucket
    assert_eq!(output.checks[5].status, DoctorCheckStatus::NotChecked); // dynamodb_table
}

#[tokio::test]
async fn test_unhealthy_stack_continues_to_connectivity() {
    let backend = FakeBackend::unhealthy_stack();
    let output = run_doctor_checks(&backend, "test-stack").await;

    assert!(!output.healthy);
    assert_eq!(output.checks.len(), 6);
    assert_eq!(output.checks[2].name, DoctorCheckName::StackStatus);
    assert_eq!(output.checks[2].status, DoctorCheckStatus::Failed);
    assert!(output.checks[2].message.is_some());
    assert_eq!(output.checks[4].name, DoctorCheckName::S3Bucket);
    assert_eq!(output.checks[4].status, DoctorCheckStatus::Passed);
    assert_eq!(output.checks[5].name, DoctorCheckName::DynamodbTable);
    assert_eq!(output.checks[5].status, DoctorCheckStatus::Passed);
}

#[tokio::test]
async fn test_missing_outputs_skips_s3_and_dynamodb() {
    let backend = FakeBackend::missing_outputs();
    let output = run_doctor_checks(&backend, "test-stack").await;

    assert!(!output.healthy);
    assert_eq!(output.checks.len(), 6);
    assert_eq!(output.checks[0].status, DoctorCheckStatus::Passed); // credentials
    assert_eq!(output.checks[1].status, DoctorCheckStatus::Passed); // stack_exists
    assert_eq!(output.checks[2].status, DoctorCheckStatus::Passed); // stack_status
    assert_eq!(output.checks[3].status, DoctorCheckStatus::Failed); // stack_outputs
    assert_eq!(output.checks[4].status, DoctorCheckStatus::NotChecked); // s3_bucket
    assert_eq!(output.checks[5].status, DoctorCheckStatus::NotChecked); // dynamodb_table
    assert!(output.checks[4].message.is_some());
    assert!(output.checks[5].message.is_some());
}

#[tokio::test]
async fn test_empty_output_values_fail_stack_outputs_and_skip_storage_checks() {
    let backend = FakeBackend::empty_output_values();
    let output = run_doctor_checks(&backend, "test-stack").await;

    assert!(!output.healthy);
    assert_eq!(output.checks[3].status, DoctorCheckStatus::Failed);
    assert_eq!(output.checks[4].status, DoctorCheckStatus::NotChecked);
    assert_eq!(output.checks[5].status, DoctorCheckStatus::NotChecked);
}

#[tokio::test]
async fn test_s3_failure_with_dynamodb_still_attempted() {
    let backend = FakeBackend::s3_fail();
    let output = run_doctor_checks(&backend, "test-stack").await;

    assert!(!output.healthy);
    assert_eq!(output.checks.len(), 6);
    assert_eq!(output.checks[4].status, DoctorCheckStatus::Failed); // s3_bucket
    assert_eq!(output.checks[5].status, DoctorCheckStatus::Passed); // dynamodb_table still runs
}

#[tokio::test]
async fn test_dynamodb_failure_with_s3_still_passes() {
    let backend = FakeBackend::dynamodb_fail();
    let output = run_doctor_checks(&backend, "test-stack").await;

    assert!(!output.healthy);
    assert_eq!(output.checks[4].status, DoctorCheckStatus::Passed);
    assert_eq!(output.checks[5].status, DoctorCheckStatus::Failed);
}

#[tokio::test]
async fn test_json_serialization_shape() {
    let backend = FakeBackend::all_pass();
    let output = run_doctor_checks(&backend, "test-stack").await;

    let json = serde_json::to_value(&output).expect("should serialize");
    assert!(json.get("healthy").is_some());
    assert_eq!(json.get("healthy").unwrap(), true);
    assert!(json.get("checks").is_some());

    let checks = json.get("checks").unwrap().as_array().unwrap();
    assert_eq!(checks.len(), 6);

    let first_check = &checks[0];
    assert_eq!(first_check.get("name").unwrap(), "credentials");
    assert_eq!(first_check.get("status").unwrap(), "passed");
    assert!(first_check.get("message").is_some());
}

#[tokio::test]
async fn test_json_check_order_preserved() {
    let backend = FakeBackend::all_pass();
    let output = run_doctor_checks(&backend, "test-stack").await;

    let json = serde_json::to_value(&output).expect("should serialize");
    let checks = json.get("checks").unwrap().as_array().unwrap();

    let expected_names = [
        "credentials",
        "stack_exists",
        "stack_status",
        "stack_outputs",
        "s3_bucket",
        "dynamodb_table",
    ];

    for (i, expected_name) in expected_names.iter().enumerate() {
        let name = checks[i].get("name").unwrap().as_str().unwrap();
        assert_eq!(name, *expected_name);
    }
}

#[tokio::test]
async fn test_failed_check_includes_message() {
    let backend = FakeBackend::no_stack();
    let output = run_doctor_checks(&backend, "test-stack").await;

    let stack_exists_check = &output.checks[1];
    assert_eq!(stack_exists_check.status, DoctorCheckStatus::Failed);
    assert!(stack_exists_check.message.is_some());
    assert_eq!(
        stack_exists_check.message.as_ref().unwrap(),
        "stack not found"
    );
}

#[tokio::test]
async fn test_not_checked_includes_reason() {
    let backend = FakeBackend::no_stack();
    let output = run_doctor_checks(&backend, "test-stack").await;

    let s3_check = &output.checks[4];
    assert_eq!(s3_check.status, DoctorCheckStatus::NotChecked);
    assert!(s3_check.message.is_some());
    assert_eq!(s3_check.message.as_ref().unwrap(), "stack not found");
}

#[test]
fn test_unhealthy_doctor_output_returns_validation_error() {
    let output = DoctorOutput::new(vec![
        DoctorCheck::passed(DoctorCheckName::Credentials),
        DoctorCheck::failed(DoctorCheckName::StackExists, "missing"),
        DoctorCheck::not_checked(DoctorCheckName::StackStatus, "skipped"),
    ]);

    let result = validate_doctor_output(&output);

    assert!(
        matches!(result, Err(ShimesuError::Validation(message)) if message.contains("2 non-passed checks"))
    );
}
