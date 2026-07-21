//! Doctor check names, statuses, and output model.

use aws_sdk_cloudformation::types::{Output, Stack, StackStatus};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorCheckName {
    Credentials,
    StackExists,
    StackStatus,
    StackOutputs,
    S3Bucket,
    DynamodbTable,
}

impl DoctorCheckName {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Credentials => "credentials",
            Self::StackExists => "stack_exists",
            Self::StackStatus => "stack_status",
            Self::StackOutputs => "stack_outputs",
            Self::S3Bucket => "s3_bucket",
            Self::DynamodbTable => "dynamodb_table",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Credentials => "Credentials",
            Self::StackExists => "Stack exists",
            Self::StackStatus => "Stack status",
            Self::StackOutputs => "Stack outputs",
            Self::S3Bucket => "S3 bucket",
            Self::DynamodbTable => "DynamoDB table",
        }
    }
}

/// Status of a single doctor check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorCheckStatus {
    Passed,
    Failed,
    NotChecked,
}

/// A single diagnostic check result.
#[derive(Debug, Clone, Serialize)]
pub struct DoctorCheck {
    pub name: DoctorCheckName,
    pub status: DoctorCheckStatus,
    pub message: Option<String>,
}

impl DoctorCheck {
    pub fn passed(name: DoctorCheckName) -> Self {
        Self {
            name,
            status: DoctorCheckStatus::Passed,
            message: None,
        }
    }

    pub fn failed(name: DoctorCheckName, message: impl Into<String>) -> Self {
        Self {
            name,
            status: DoctorCheckStatus::Failed,
            message: Some(message.into()),
        }
    }

    pub fn not_checked(name: DoctorCheckName, message: impl Into<String>) -> Self {
        Self {
            name,
            status: DoctorCheckStatus::NotChecked,
            message: Some(message.into()),
        }
    }
}

/// Snapshot of stack outputs and status.
#[derive(Debug, Clone)]
pub(crate) struct DoctorStackSnapshot {
    pub(crate) status: StackStatus,
    pub(crate) bucket_name: Option<String>,
    pub(crate) table_name: Option<String>,
    pub(crate) distribution_id: Option<String>,
    pub(crate) base_domain: Option<String>,
}

impl DoctorStackSnapshot {
    pub(crate) fn from_stack(stack: &Stack) -> Self {
        let outputs = stack.outputs();
        Self {
            status: stack
                .stack_status()
                .cloned()
                .unwrap_or_else(|| StackStatus::from("UNKNOWN")),
            bucket_name: output_value(outputs, "BucketName"),
            table_name: output_value(outputs, "TableName"),
            distribution_id: output_value(outputs, "DistributionId"),
            base_domain: output_value(outputs, "BaseDomain"),
        }
    }

    pub(crate) fn all_outputs_present(&self) -> bool {
        self.missing_output_names().is_empty()
    }

    pub(crate) fn missing_output_names(&self) -> Vec<&'static str> {
        [
            ("BucketName", value_is_present(&self.bucket_name)),
            ("TableName", value_is_present(&self.table_name)),
            ("DistributionId", value_is_present(&self.distribution_id)),
            ("BaseDomain", value_is_present(&self.base_domain)),
        ]
        .into_iter()
        .filter_map(|(name, present)| if present { None } else { Some(name) })
        .collect()
    }
}

fn value_is_present(value: &Option<String>) -> bool {
    value.as_deref().is_some_and(|text| !text.trim().is_empty())
}

fn output_value(outputs: &[Output], key: &str) -> Option<String> {
    outputs
        .iter()
        .find(|output| output.output_key() == Some(key))
        .and_then(|output| output.output_value())
        .map(str::to_owned)
}

/// Complete doctor output with all checks.
#[derive(Debug, Clone, Serialize)]
pub struct DoctorOutput {
    pub healthy: bool,
    pub checks: Vec<DoctorCheck>,
}

impl DoctorOutput {
    pub fn new(checks: Vec<DoctorCheck>) -> Self {
        let healthy = checks.iter().all(|c| c.status == DoctorCheckStatus::Passed);
        Self { healthy, checks }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_name_serialization() {
        assert_eq!(
            serde_json::to_string(&DoctorCheckName::Credentials).unwrap(),
            "\"credentials\""
        );
        assert_eq!(
            serde_json::to_string(&DoctorCheckName::S3Bucket).unwrap(),
            "\"s3_bucket\""
        );
    }

    #[test]
    fn test_check_passed() {
        let check = DoctorCheck::passed(DoctorCheckName::Credentials);
        assert_eq!(check.status, DoctorCheckStatus::Passed);
        assert_eq!(check.message, None);
    }

    #[test]
    fn test_check_name_label_is_human_readable() {
        assert_eq!(DoctorCheckName::Credentials.label(), "Credentials");
        assert_eq!(DoctorCheckName::StackExists.label(), "Stack exists");
        assert_eq!(DoctorCheckName::DynamodbTable.label(), "DynamoDB table");
    }

    #[test]
    fn test_check_failed() {
        let check = DoctorCheck::failed(DoctorCheckName::StackExists, "not found");
        assert_eq!(check.status, DoctorCheckStatus::Failed);
        assert_eq!(check.message, Some("not found".to_string()));
    }

    #[test]
    fn test_output_healthy_all_passed() {
        let checks = vec![
            DoctorCheck::passed(DoctorCheckName::Credentials),
            DoctorCheck::passed(DoctorCheckName::StackExists),
        ];
        let output = DoctorOutput::new(checks);
        assert!(output.healthy);
    }

    #[test]
    fn test_output_not_healthy_with_failure() {
        let checks = vec![
            DoctorCheck::passed(DoctorCheckName::Credentials),
            DoctorCheck::failed(DoctorCheckName::StackExists, "failed"),
        ];
        let output = DoctorOutput::new(checks);
        assert!(!output.healthy);
    }

    #[test]
    fn test_output_serialization_has_exact_shape_and_order() {
        let output = DoctorOutput::new(vec![
            DoctorCheck::passed(DoctorCheckName::Credentials),
            DoctorCheck::passed(DoctorCheckName::StackExists),
            DoctorCheck::failed(DoctorCheckName::StackStatus, "bad status"),
            DoctorCheck::passed(DoctorCheckName::StackOutputs),
            DoctorCheck::not_checked(DoctorCheckName::S3Bucket, "skipped"),
            DoctorCheck::passed(DoctorCheckName::DynamodbTable),
        ]);

        let value = serde_json::to_value(&output).expect("doctor output should serialize");

        assert_eq!(
            value,
            serde_json::json!({
                "healthy": false,
                "checks": [
                    {"name": "credentials", "status": "passed", "message": null},
                    {"name": "stack_exists", "status": "passed", "message": null},
                    {"name": "stack_status", "status": "failed", "message": "bad status"},
                    {"name": "stack_outputs", "status": "passed", "message": null},
                    {"name": "s3_bucket", "status": "not_checked", "message": "skipped"},
                    {"name": "dynamodb_table", "status": "passed", "message": null}
                ]
            })
        );
    }
}
