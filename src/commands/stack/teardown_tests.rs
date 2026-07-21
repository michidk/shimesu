use serde_json::json;

use super::{
    bucket_name_matches_stack, table_name, tags_belong_to_stack, StackTeardownInput,
    StackTeardownOutput,
};
use crate::error::ShimesuError;

#[test]
fn teardown_input_requires_explicit_data_loss_confirmation() {
    let result = StackTeardownInput::parse(false);

    assert!(
        matches!(result, Err(ShimesuError::Usage(message)) if message.contains("--confirm-data-loss"))
    );
}

#[test]
fn teardown_input_accepts_explicit_data_loss_confirmation() {
    assert!(StackTeardownInput::parse(true).is_ok());
}

#[test]
fn retained_table_name_is_scoped_to_installation() {
    assert_eq!(
        table_name("shimesu-demo").unwrap_or_else(|error| panic!(
            "valid stack name should produce a table name: {error}"
        )),
        "shimesu-demo-projects"
    );
}

#[test]
fn retained_bucket_name_requires_exact_installation_prefix() {
    assert!(bucket_name_matches_stack(
        "shimesu-demo-contentbucket-a1b2c3",
        "shimesu-demo"
    ));
    assert!(!bucket_name_matches_stack(
        "shimesu-contentbucket-a1b2c3",
        "shimesu-demo"
    ));
    assert!(!bucket_name_matches_stack(
        "shimesu-demo-backups-a1b2c3",
        "shimesu-demo"
    ));
}

#[test]
fn retained_resource_requires_application_and_stack_tags() {
    let owned = [("Application", "shimesu"), ("StackName", "shimesu-demo")];
    let wrong_stack = [("Application", "shimesu"), ("StackName", "other")];
    let missing_application = [("StackName", "shimesu-demo")];

    assert!(tags_belong_to_stack(&owned, "shimesu-demo"));
    assert!(!tags_belong_to_stack(&wrong_stack, "shimesu-demo"));
    assert!(!tags_belong_to_stack(&missing_application, "shimesu-demo"));
}

#[test]
fn teardown_output_serializes_deleted_resource_counts() {
    let output = StackTeardownOutput {
        stack_name: "shimesu-demo".to_string(),
        region: "eu-central-1".to_string(),
        regional_stack_deleted: true,
        buckets_deleted: 2,
        object_versions_deleted: 17,
        table_deleted: true,
        certificate_stack_deleted: true,
        certificate_deleted: true,
    };

    assert_eq!(
        serde_json::to_value(output)
            .unwrap_or_else(|error| panic!("teardown output should serialize: {error}")),
        json!({
            "stack_name": "shimesu-demo",
            "region": "eu-central-1",
            "regional_stack_deleted": true,
            "buckets_deleted": 2,
            "object_versions_deleted": 17,
            "table_deleted": true,
            "certificate_stack_deleted": true,
            "certificate_deleted": true
        })
    );
}

#[test]
fn teardown_output_reports_absent_resources_without_fake_deletions() {
    let output = StackTeardownOutput {
        stack_name: "shimesu-demo".to_string(),
        region: "eu-central-1".to_string(),
        regional_stack_deleted: false,
        buckets_deleted: 0,
        object_versions_deleted: 0,
        table_deleted: false,
        certificate_stack_deleted: false,
        certificate_deleted: false,
    };

    let rows = output.human_rows();

    assert_eq!(
        rows[2],
        ("Regional Stack Deleted".to_string(), "false".to_string())
    );
    assert_eq!(rows[3], ("Buckets Deleted".to_string(), "0".to_string()));
    assert_eq!(
        rows[4],
        ("Object Versions Deleted".to_string(), "0".to_string())
    );
    assert_eq!(rows[5], ("Table Deleted".to_string(), "false".to_string()));
    assert_eq!(
        rows[6],
        ("Certificate Stack Deleted".to_string(), "false".to_string())
    );
    assert_eq!(
        rows[7],
        ("Certificate Deleted".to_string(), "false".to_string())
    );
}
