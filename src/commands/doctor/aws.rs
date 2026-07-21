//! AWS-backed implementation of the doctor checks.

use super::backend::DoctorBackend;
use super::models::DoctorStackSnapshot;
use crate::commands::aws_error::{
    map_aws_error, map_aws_error_with_code, map_sdk_error, sdk_error_code, sdk_error_text,
};
use crate::error::{Result, ShimesuError};
use aws_config::SdkConfig;
use aws_sdk_cloudformation::Client as CloudFormationClient;
use aws_sdk_dynamodb::operation::describe_table::DescribeTableError;
use aws_sdk_dynamodb::Client as DynamoDbClient;
use aws_sdk_s3::operation::head_bucket::HeadBucketError;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_sts::Client as StsClient;
use std::future::Future;
use std::time::Duration;

const AWS_DOCTOR_API_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) struct AwsDoctorBackend {
    sts: StsClient,
    cloudformation: CloudFormationClient,
    s3: S3Client,
    dynamodb: DynamoDbClient,
}

impl AwsDoctorBackend {
    pub(super) fn new(config: &SdkConfig) -> Self {
        Self {
            sts: StsClient::new(config),
            cloudformation: CloudFormationClient::new(config),
            s3: S3Client::new(config),
            dynamodb: DynamoDbClient::new(config),
        }
    }
}

impl DoctorBackend for AwsDoctorBackend {
    async fn check_credentials(&self) -> Result<()> {
        with_api_timeout(AWS_DOCTOR_API_TIMEOUT, "STS GetCallerIdentity", async {
            self.sts
                .get_caller_identity()
                .send()
                .await
                .map(|_| ())
                .map_err(|error| map_sdk_error("Failed to validate AWS credentials", &error))
        })
        .await
    }

    async fn describe_stack(&self, stack_name: &str) -> Result<Option<DoctorStackSnapshot>> {
        with_api_timeout(
            AWS_DOCTOR_API_TIMEOUT,
            "CloudFormation DescribeStacks",
            async {
                let response = self
                    .cloudformation
                    .describe_stacks()
                    .stack_name(stack_name)
                    .send()
                    .await;

                match response {
                    Ok(output) => Ok(output.stacks().first().map(DoctorStackSnapshot::from_stack)),
                    Err(error) => {
                        let error_text = sdk_error_text(&error);
                        if is_stack_not_found(&error_text) {
                            Ok(None)
                        } else {
                            Err(map_aws_error_with_code(
                                "Failed to describe stack",
                                sdk_error_code(&error),
                                &error_text,
                            ))
                        }
                    }
                }
            },
        )
        .await
    }

    async fn head_bucket(&self, bucket_name: &str) -> Result<()> {
        with_api_timeout(AWS_DOCTOR_API_TIMEOUT, "S3 HeadBucket", async {
            self.s3
                .head_bucket()
                .bucket(bucket_name)
                .send()
                .await
                .map(|_| ())
                .map_err(|error| {
                    if error
                        .as_service_error()
                        .is_some_and(HeadBucketError::is_not_found)
                    {
                        ShimesuError::NotFound(format!("Bucket '{bucket_name}' not found"))
                    } else {
                        map_head_bucket_error(bucket_name, &sdk_error_text(&error))
                    }
                })
        })
        .await
    }

    async fn describe_table(&self, table_name: &str) -> Result<()> {
        with_api_timeout(AWS_DOCTOR_API_TIMEOUT, "DynamoDB DescribeTable", async {
            self.dynamodb
                .describe_table()
                .table_name(table_name)
                .send()
                .await
                .map(|_| ())
                .map_err(|error| {
                    if error
                        .as_service_error()
                        .is_some_and(DescribeTableError::is_resource_not_found_exception)
                    {
                        ShimesuError::NotFound(format!("Table '{table_name}' not found"))
                    } else {
                        map_describe_table_error(table_name, &sdk_error_text(&error))
                    }
                })
        })
        .await
    }
}

async fn with_api_timeout<T, F>(duration: Duration, operation: &str, future: F) -> Result<T>
where
    F: Future<Output = Result<T>>,
{
    match tokio::time::timeout(duration, future).await {
        Ok(result) => result,
        Err(_) => Err(ShimesuError::Timeout(format!(
            "{operation} timed out after {} seconds",
            duration.as_secs()
        ))),
    }
}

fn is_stack_not_found(error_text: &str) -> bool {
    let lower = error_text.to_ascii_lowercase();
    lower.contains("does not exist") || lower.contains("not found")
}

fn map_head_bucket_error(bucket_name: &str, error_text: &str) -> ShimesuError {
    let lower = error_text.to_ascii_lowercase();
    if lower.contains("nosuchbucket") || lower.contains("not found") || lower.contains("404") {
        ShimesuError::NotFound(format!("Bucket '{bucket_name}' not found"))
    } else if lower.contains("403")
        || lower.contains("forbidden")
        || lower.contains("accessdenied")
        || lower.contains("access denied")
    {
        ShimesuError::AwsAuth(format!(
            "Failed to check S3 bucket '{bucket_name}': {error_text}"
        ))
    } else {
        map_aws_error(
            &format!("Failed to check S3 bucket '{bucket_name}'"),
            error_text,
        )
    }
}

fn map_describe_table_error(table_name: &str, error_text: &str) -> ShimesuError {
    let lower = error_text.to_ascii_lowercase();
    if lower.contains("resourcenotfoundexception")
        || lower.contains("table not found")
        || lower.contains("non-existent table")
    {
        ShimesuError::NotFound(format!("Table '{table_name}' not found"))
    } else {
        map_aws_error(
            &format!("Failed to describe DynamoDB table '{table_name}'"),
            error_text,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        is_stack_not_found, map_describe_table_error, map_head_bucket_error, with_api_timeout,
    };
    use crate::commands::doctor::models::DoctorStackSnapshot;
    use crate::error::ShimesuError;
    use aws_sdk_cloudformation::types::{Output, Stack, StackStatus};
    use std::future;
    use std::time::Duration;

    #[test]
    fn map_head_bucket_error_returns_not_found_for_missing_bucket() {
        let error = map_head_bucket_error("site-bucket", "NoSuchBucket: bucket does not exist");

        assert!(
            matches!(error, ShimesuError::NotFound(message) if message == "Bucket 'site-bucket' not found")
        );
    }

    #[test]
    fn map_head_bucket_error_returns_auth_for_forbidden_bucket() {
        let error = map_head_bucket_error("site-bucket", "403 Forbidden");

        assert!(
            matches!(error, ShimesuError::AwsAuth(message) if message.contains("Failed to check S3 bucket 'site-bucket'"))
        );
    }

    #[test]
    fn map_describe_table_error_returns_auth_for_access_denied() {
        let error = map_describe_table_error(
            "sites-table",
            "AccessDeniedException: not authorized to perform dynamodb:DescribeTable",
        );

        assert!(
            matches!(error, ShimesuError::AwsAuth(message) if message.contains("Failed to describe DynamoDB table"))
        );
    }

    #[test]
    fn map_describe_table_error_returns_throttled_for_rate_limits() {
        let error = map_describe_table_error("sites-table", "ThrottlingException: Rate exceeded");

        assert!(
            matches!(error, ShimesuError::AwsThrottled(message) if message == "ThrottlingException: Rate exceeded")
        );
    }

    #[test]
    fn stack_not_found_error_is_detected() {
        assert!(is_stack_not_found(
            "ValidationError: Stack with id test-stack does not exist"
        ));
    }

    #[test]
    fn snapshot_from_stack_extracts_expected_outputs_and_unknown_status() {
        let stack = Stack::builder()
            .outputs(
                Output::builder()
                    .output_key("BucketName")
                    .output_value("site-bucket")
                    .build(),
            )
            .outputs(
                Output::builder()
                    .output_key("TableName")
                    .output_value("sites-table")
                    .build(),
            )
            .outputs(
                Output::builder()
                    .output_key("DistributionId")
                    .output_value("DIST123")
                    .build(),
            )
            .outputs(
                Output::builder()
                    .output_key("BaseDomain")
                    .output_value("static.example.com")
                    .build(),
            )
            .build();

        let snapshot = DoctorStackSnapshot::from_stack(&stack);

        assert_eq!(snapshot.status, StackStatus::from("UNKNOWN"));
        assert_eq!(snapshot.bucket_name.as_deref(), Some("site-bucket"));
        assert_eq!(snapshot.table_name.as_deref(), Some("sites-table"));
        assert_eq!(snapshot.distribution_id.as_deref(), Some("DIST123"));
        assert_eq!(snapshot.base_domain.as_deref(), Some("static.example.com"));
    }

    #[tokio::test]
    async fn with_api_timeout_returns_timeout_for_pending_future() {
        let result = with_api_timeout(Duration::from_millis(1), "check credentials", async {
            future::pending::<crate::error::Result<()>>().await
        })
        .await;

        assert!(
            matches!(result, Err(ShimesuError::Timeout(message)) if message.contains("check credentials"))
        );
    }

    #[tokio::test]
    async fn with_api_timeout_propagates_typed_errors() {
        let result =
            with_api_timeout::<(), _>(Duration::from_secs(1), "check credentials", async {
                Err(ShimesuError::AwsAuth("bad credentials".to_string()))
            })
            .await;

        assert!(
            matches!(result, Err(ShimesuError::AwsAuth(message)) if message == "bad credentials")
        );
    }
}
