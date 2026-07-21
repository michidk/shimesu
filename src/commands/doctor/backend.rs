//! `DoctorBackend` trait boundary so checks are testable without AWS.

use super::models::DoctorStackSnapshot;
use crate::error::Result;

/// Backend trait for doctor checks. Implementations handle AWS API calls or fakes.
pub(crate) trait DoctorBackend: Send + Sync {
    /// Check if credentials are available and valid.
    async fn check_credentials(&self) -> Result<()>;

    /// Describe the stack. Returns None if stack does not exist.
    async fn describe_stack(&self, stack_name: &str) -> Result<Option<DoctorStackSnapshot>>;

    /// Check if S3 bucket exists and is accessible.
    async fn head_bucket(&self, bucket_name: &str) -> Result<()>;

    /// Check if DynamoDB table exists and is accessible.
    async fn describe_table(&self, table_name: &str) -> Result<()>;
}

#[cfg(test)]
pub mod fake {
    use super::*;
    use aws_sdk_cloudformation::types::StackStatus as AwsStackStatus;

    pub struct FakeBackend {
        pub credentials_ok: bool,
        pub stack_snapshot: Option<DoctorStackSnapshot>,
        pub bucket_ok: bool,
        pub table_ok: bool,
    }

    impl FakeBackend {
        pub fn all_pass() -> Self {
            Self {
                credentials_ok: true,
                stack_snapshot: Some(DoctorStackSnapshot {
                    status: AwsStackStatus::CreateComplete,
                    bucket_name: Some("test-bucket".to_string()),
                    table_name: Some("test-table".to_string()),
                    distribution_id: Some("DIST123".to_string()),
                    base_domain: Some("example.com".to_string()),
                }),
                bucket_ok: true,
                table_ok: true,
            }
        }

        pub fn no_stack() -> Self {
            Self {
                credentials_ok: true,
                stack_snapshot: None,
                bucket_ok: true,
                table_ok: true,
            }
        }

        pub fn unhealthy_stack() -> Self {
            Self {
                credentials_ok: true,
                stack_snapshot: Some(DoctorStackSnapshot {
                    status: AwsStackStatus::UpdateRollbackComplete,
                    bucket_name: Some("test-bucket".to_string()),
                    table_name: Some("test-table".to_string()),
                    distribution_id: Some("DIST123".to_string()),
                    base_domain: Some("example.com".to_string()),
                }),
                bucket_ok: true,
                table_ok: true,
            }
        }

        pub fn missing_outputs() -> Self {
            Self {
                credentials_ok: true,
                stack_snapshot: Some(DoctorStackSnapshot {
                    status: AwsStackStatus::CreateComplete,
                    bucket_name: None,
                    table_name: None,
                    distribution_id: None,
                    base_domain: None,
                }),
                bucket_ok: true,
                table_ok: true,
            }
        }

        pub fn empty_output_values() -> Self {
            Self {
                credentials_ok: true,
                stack_snapshot: Some(DoctorStackSnapshot {
                    status: AwsStackStatus::CreateComplete,
                    bucket_name: Some(" ".to_string()),
                    table_name: Some("".to_string()),
                    distribution_id: Some("DIST123".to_string()),
                    base_domain: Some("\t".to_string()),
                }),
                bucket_ok: true,
                table_ok: true,
            }
        }

        pub fn credentials_fail() -> Self {
            Self {
                credentials_ok: false,
                stack_snapshot: Some(DoctorStackSnapshot {
                    status: AwsStackStatus::CreateComplete,
                    bucket_name: Some("test-bucket".to_string()),
                    table_name: Some("test-table".to_string()),
                    distribution_id: Some("DIST123".to_string()),
                    base_domain: Some("example.com".to_string()),
                }),
                bucket_ok: true,
                table_ok: true,
            }
        }

        pub fn s3_fail() -> Self {
            Self {
                credentials_ok: true,
                stack_snapshot: Some(DoctorStackSnapshot {
                    status: AwsStackStatus::CreateComplete,
                    bucket_name: Some("test-bucket".to_string()),
                    table_name: Some("test-table".to_string()),
                    distribution_id: Some("DIST123".to_string()),
                    base_domain: Some("example.com".to_string()),
                }),
                bucket_ok: false,
                table_ok: true,
            }
        }

        pub fn dynamodb_fail() -> Self {
            Self {
                credentials_ok: true,
                stack_snapshot: Some(DoctorStackSnapshot {
                    status: AwsStackStatus::CreateComplete,
                    bucket_name: Some("test-bucket".to_string()),
                    table_name: Some("test-table".to_string()),
                    distribution_id: Some("DIST123".to_string()),
                    base_domain: Some("example.com".to_string()),
                }),
                bucket_ok: true,
                table_ok: false,
            }
        }
    }

    impl DoctorBackend for FakeBackend {
        async fn check_credentials(&self) -> Result<()> {
            if self.credentials_ok {
                Ok(())
            } else {
                Err(crate::error::ShimesuError::AwsAuth(
                    "credentials not available".to_string(),
                ))
            }
        }

        async fn describe_stack(&self, _stack_name: &str) -> Result<Option<DoctorStackSnapshot>> {
            Ok(self.stack_snapshot.clone())
        }

        async fn head_bucket(&self, _bucket_name: &str) -> Result<()> {
            if self.bucket_ok {
                Ok(())
            } else {
                Err(crate::error::ShimesuError::Generic(
                    "bucket not accessible".to_string(),
                ))
            }
        }

        async fn describe_table(&self, _table_name: &str) -> Result<()> {
            if self.table_ok {
                Ok(())
            } else {
                Err(crate::error::ShimesuError::Generic(
                    "table not accessible".to_string(),
                ))
            }
        }
    }
}
