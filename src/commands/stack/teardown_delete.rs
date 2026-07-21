//! Deletion primitives for teardown: stacks, buckets, table, and certificate.

use aws_sdk_acm::Client as AcmClient;
use aws_sdk_cloudformation::Client as CfnClient;
use aws_sdk_dynamodb::{client::Waiters, Client as DynamoClient};
use aws_sdk_s3::Client as S3Client;
use ulid::Ulid;

use crate::commands::aws_error::{map_sdk_error, sdk_error_code, sdk_error_text};
use crate::error::{Result, ShimesuError};

use super::certificate_ownership::acm_tags_belong_to_stack;
use super::destroy::{request_stack_delete, wait_for_stack_delete, DeleteRequestOutcome};
use super::teardown::table_name;
use super::teardown_discovery::{discover_owned_table, BucketCandidate};
use super::teardown_s3::{delete_owned_bucket, BucketDeletionStats};
use super::STACK_TIMEOUT;

pub(super) async fn delete_stack_if_present(
    cfn_client: &CfnClient,
    stack_name: &str,
) -> Result<bool> {
    let request_token = Ulid::generate().to_string();
    match request_stack_delete(cfn_client, stack_name, &request_token).await? {
        DeleteRequestOutcome::Started => {
            wait_for_stack_delete(cfn_client, stack_name).await?;
            Ok(true)
        }
        DeleteRequestOutcome::AlreadyAbsent => Ok(false),
    }
}

pub(super) async fn delete_owned_buckets(
    s3_client: &S3Client,
    buckets: &[BucketCandidate],
) -> Result<BucketDeletionStats> {
    let mut aggregate = BucketDeletionStats::default();
    for bucket in buckets {
        let stats = delete_owned_bucket(s3_client, bucket).await?;
        aggregate.buckets_deleted += stats.buckets_deleted;
        aggregate.object_versions_deleted += stats.object_versions_deleted;
    }
    Ok(aggregate)
}

pub(super) async fn delete_owned_table(
    dynamo_client: &DynamoClient,
    stack_name: &str,
) -> Result<bool> {
    let candidate = discover_owned_table(dynamo_client, stack_name).await?;
    let Some(candidate) = candidate else {
        return Ok(false);
    };
    let table_name = table_name(stack_name)?;
    dynamo_client
        .delete_table()
        .table_name(&table_name)
        .send()
        .await
        .map_err(|error| {
            map_sdk_error(
                &format!(
                    "Failed to delete DynamoDB table '{table_name}' ({})",
                    candidate.arn
                ),
                &error,
            )
        })?;
    dynamo_client
        .wait_until_table_not_exists()
        .table_name(&table_name)
        .wait(STACK_TIMEOUT)
        .await
        .map_err(|error| {
            ShimesuError::Timeout(format!(
                "DynamoDB table '{table_name}' deletion did not complete within {} seconds: {error}",
                STACK_TIMEOUT.as_secs()
            ))
        })?;
    Ok(true)
}

pub(super) async fn delete_certificate_if_present(
    acm_client: &AcmClient,
    certificate_arn: Option<&str>,
    stack_name: &str,
) -> Result<bool> {
    let Some(certificate_arn) = certificate_arn else {
        return Ok(false);
    };
    if !validate_certificate_ownership(acm_client, certificate_arn, stack_name).await? {
        return Ok(false);
    }

    match acm_client
        .delete_certificate()
        .certificate_arn(certificate_arn)
        .send()
        .await
    {
        Ok(_) => Ok(true),
        Err(error) => {
            let code = sdk_error_code(&error);
            let text = sdk_error_text(&error);
            if is_missing_certificate(code, &text) {
                Ok(false)
            } else {
                Err(map_sdk_error(
                    &format!("Failed to delete ACM certificate '{certificate_arn}'"),
                    &error,
                ))
            }
        }
    }
}

pub(super) async fn validate_certificate_ownership(
    acm_client: &AcmClient,
    certificate_arn: &str,
    stack_name: &str,
) -> Result<bool> {
    let tags_response = match acm_client
        .list_tags_for_certificate()
        .certificate_arn(certificate_arn)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            let code = sdk_error_code(&error);
            let text = sdk_error_text(&error);
            if is_missing_certificate(code, &text) {
                return Ok(false);
            }
            return Err(map_sdk_error(
                &format!("Failed to read tags for ACM certificate '{certificate_arn}'"),
                &error,
            ));
        }
    };
    if !acm_tags_belong_to_stack(tags_response.tags(), stack_name) {
        return Err(ShimesuError::Validation(format!(
            "Refusing to delete ACM certificate '{certificate_arn}' because it is missing required shimesu ownership tags"
        )));
    }
    Ok(true)
}

fn is_missing_certificate(code: Option<&str>, text: &str) -> bool {
    code == Some("ResourceNotFoundException") || text.to_ascii_lowercase().contains("not found")
}
