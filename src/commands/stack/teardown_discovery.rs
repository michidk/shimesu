//! Discovers retained, installation-owned resources eligible for teardown.

use aws_sdk_cloudformation::Client as CfnClient;
use aws_sdk_dynamodb::Client as DynamoClient;
use aws_sdk_s3::Client as S3Client;

use crate::commands::aws_error::{map_sdk_error, sdk_error_code, sdk_error_text};
use crate::commands::support::StackOutputs;
use crate::error::{Result, ShimesuError};

use super::certificate_ownership::{inspect_managed_certificate_stack, ManagedCertificateStack};
use super::ownership::{inspect_installation_ownership, InstallationOwnership};
use super::teardown::{bucket_name_matches_stack, table_name, tags_belong_to_stack};
use super::teardown_delete::validate_certificate_ownership;

/// Snapshot of stack outputs captured before any destructive step so that
/// retained resources can be located and cleaned up when a stack is absent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct TeardownSnapshot {
    pub(super) regional_bucket_name: Option<String>,
    pub(super) certificate_stack_owned: bool,
    /// ARN from the managed `<stack>-certificate` stack only.
    /// Operator-supplied external certificates are never included here.
    pub(super) certificate_stack_certificate_arn: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BucketCandidate {
    pub(super) name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TableCandidate {
    pub(super) arn: String,
}

/// Capture the regional and certificate-stack outputs before any deletion.
/// The certificate ARN is taken only from the managed certificate stack so
/// that operator-supplied external certificates are never deleted.
pub(super) async fn discover_teardown_snapshot(
    stack_name: &str,
    aws_config: &aws_config::SdkConfig,
    certificate_aws_config: &aws_config::SdkConfig,
) -> Result<TeardownSnapshot> {
    let regional_outputs = match inspect_installation_ownership(aws_config, stack_name).await? {
        InstallationOwnership::Absent => None,
        InstallationOwnership::Owned(outputs) => Some(outputs),
    };
    let expected_domain = certificate_cleanup_domain(regional_outputs.as_ref());
    let managed_certificate = if let Some(expected_domain) = expected_domain {
        inspect_managed_certificate_stack(
            &CfnClient::new(certificate_aws_config),
            stack_name,
            Some(expected_domain),
        )
        .await?
    } else {
        ManagedCertificateStack::Absent
    };
    let (certificate_stack_owned, certificate_stack_certificate_arn) = match managed_certificate {
        ManagedCertificateStack::Absent => (false, None),
        ManagedCertificateStack::Owned { certificate_arn } => {
            let certificate_exists = validate_certificate_ownership(
                &aws_sdk_acm::Client::new(certificate_aws_config),
                &certificate_arn,
                stack_name,
            )
            .await?;
            (true, certificate_exists.then_some(certificate_arn))
        }
    };

    Ok(TeardownSnapshot {
        regional_bucket_name: regional_outputs
            .as_ref()
            .and_then(|outputs| outputs.bucket_name.clone()),
        certificate_stack_owned,
        certificate_stack_certificate_arn,
    })
}

fn certificate_cleanup_domain(outputs: Option<&StackOutputs>) -> Option<&str> {
    outputs.and_then(|outputs| outputs.base_domain.as_deref())
}

pub(super) async fn discover_owned_buckets(
    s3_client: &S3Client,
    stack_name: &str,
    expected_bucket_name: Option<&str>,
) -> Result<Vec<BucketCandidate>> {
    let response = s3_client
        .list_buckets()
        .send()
        .await
        .map_err(|error| map_sdk_error("Failed to list S3 buckets", &error))?;

    let mut buckets = response
        .buckets()
        .iter()
        .filter_map(|bucket| bucket.name())
        .filter(|bucket_name| bucket_name_matches_stack(bucket_name, stack_name))
        .collect::<Vec<_>>();
    buckets.sort_unstable();

    let mut owned = Vec::new();
    for bucket_name in buckets {
        let tag_matches = bucket_tags_match(s3_client, bucket_name, stack_name).await?;
        if Some(bucket_name) == expected_bucket_name && !tag_matches {
            return Err(ShimesuError::Validation(format!(
                "Refusing to delete bucket '{bucket_name}' because it is missing required shimesu ownership tags"
            )));
        }
        if tag_matches {
            owned.push(BucketCandidate {
                name: bucket_name.to_string(),
            });
        }
    }

    Ok(owned)
}

pub(super) async fn discover_owned_table(
    dynamo_client: &DynamoClient,
    stack_name: &str,
) -> Result<Option<TableCandidate>> {
    let table_name = table_name(stack_name)?;
    let response = match dynamo_client
        .describe_table()
        .table_name(&table_name)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            let code = sdk_error_code(&error);
            let text = sdk_error_text(&error);
            if is_absent_resource(code, &text) {
                return Ok(None);
            }
            return Err(map_sdk_error(
                &format!("Failed to describe DynamoDB table '{table_name}'"),
                &error,
            ));
        }
    };

    let table = response.table().ok_or_else(|| {
        ShimesuError::Generic(format!(
            "DynamoDB returned no table details for '{table_name}'"
        ))
    })?;
    let table_arn = table.table_arn().ok_or_else(|| {
        ShimesuError::Generic(format!("DynamoDB returned no ARN for table '{table_name}'"))
    })?;
    let tags_response = dynamo_client
        .list_tags_of_resource()
        .resource_arn(table_arn)
        .send()
        .await
        .map_err(|error| {
            map_sdk_error(
                &format!("Failed to read tags for DynamoDB table '{table_name}'"),
                &error,
            )
        })?;
    let tags = tags_response
        .tags()
        .iter()
        .map(|tag| (tag.key(), tag.value()))
        .collect::<Vec<_>>();

    if !tags_belong_to_stack(&tags, stack_name) {
        return Err(ShimesuError::Validation(format!(
            "Refusing to delete table '{table_name}' because it is missing required shimesu ownership tags"
        )));
    }

    Ok(Some(TableCandidate {
        arn: table_arn.to_string(),
    }))
}

async fn bucket_tags_match(
    s3_client: &S3Client,
    bucket_name: &str,
    stack_name: &str,
) -> Result<bool> {
    let response = match s3_client
        .get_bucket_tagging()
        .bucket(bucket_name)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            let code = sdk_error_code(&error);
            let text = sdk_error_text(&error);
            if code == Some("NoSuchTagSet") || is_absent_resource(code, &text) {
                return Ok(false);
            }
            return Err(map_sdk_error(
                &format!("Failed to read tags for bucket '{bucket_name}'"),
                &error,
            ));
        }
    };

    let tags = response
        .tag_set()
        .iter()
        .map(|tag| (tag.key(), tag.value()))
        .collect::<Vec<_>>();
    Ok(tags_belong_to_stack(&tags, stack_name))
}

fn is_absent_resource(code: Option<&str>, error_text: &str) -> bool {
    code == Some("ResourceNotFoundException")
        || code == Some("NoSuchBucket")
        || error_text.to_ascii_lowercase().contains("not found")
        || error_text.to_ascii_lowercase().contains("does not exist")
}

#[cfg(test)]
mod tests {
    use crate::commands::support::StackOutputs;

    use super::certificate_cleanup_domain;

    #[test]
    fn certificate_cleanup_requires_regional_domain_proof() {
        assert_eq!(certificate_cleanup_domain(None), None);
    }

    #[test]
    fn certificate_cleanup_uses_the_regional_stack_domain() {
        let outputs = StackOutputs {
            base_domain: Some("pages.example.com".to_string()),
            ..StackOutputs::default()
        };

        assert_eq!(
            certificate_cleanup_domain(Some(&outputs)),
            Some("pages.example.com")
        );
    }
}
