//! Shared AWS helpers: SDK config loading, stack output lookup, and CloudFront invalidation.

use crate::commands::aws_error::{
    map_sdk_error, map_stack_describe_error, map_stack_describe_sdk_error,
};
use crate::config::Config;
use crate::error::{Result, ShimesuError};
use aws_config::{BehaviorVersion, SdkConfig};
use aws_sdk_cloudformation::{types::Stack, Client as CfnClient};
use aws_sdk_cloudfront::types::{InvalidationBatch, Paths};
use aws_sdk_cloudfront::Client as CloudFrontClient;
use aws_sdk_dynamodb::types::AttributeValue;
use std::collections::HashMap;

pub struct ListOutputs {
    pub table_name: String,
    pub base_domain: String,
}

#[derive(Default)]
pub struct StackOutputs {
    pub table_name: Option<String>,
    pub bucket_name: Option<String>,
    pub base_domain: Option<String>,
    pub certificate_arn: Option<String>,
    pub distribution_id: Option<String>,
    pub distribution_domain_name: Option<String>,
}

pub fn marker_key(slug: &str) -> String {
    format!("{slug}/")
}

pub(crate) fn aws_sdk_config_loader(config: &Config) -> aws_config::ConfigLoader {
    let loader = aws_config::defaults(BehaviorVersion::latest())
        .region(aws_config::Region::new(config.region.clone()));

    match &config.profile {
        Some(profile) => loader.profile_name(profile),
        None => loader,
    }
}

pub(crate) async fn load_aws_sdk_config(config: &Config) -> SdkConfig {
    aws_sdk_config_loader(config).load().await
}

/// Invalidate every cached object below `/{slug}/` on the shared distribution.
/// Used after both publish and delete so viewers never serve stale content.
pub(crate) async fn invalidate_site(
    client: &CloudFrontClient,
    distribution_id: &str,
    slug: &str,
    caller_reference: &str,
) -> Result<String> {
    let paths = Paths::builder()
        .quantity(1)
        .items(format!("/{slug}/*"))
        .build()
        .map_err(|error| {
            ShimesuError::Generic(format!("Failed to build invalidation paths: {error}"))
        })?;
    let batch = InvalidationBatch::builder()
        .caller_reference(caller_reference)
        .paths(paths)
        .build()
        .map_err(|error| ShimesuError::Generic(format!("Failed to build invalidation: {error}")))?;
    let response = client
        .create_invalidation()
        .distribution_id(distribution_id)
        .invalidation_batch(batch)
        .send()
        .await
        .map_err(|error| map_sdk_error("Failed to invalidate site", &error))?;
    response
        .invalidation()
        .map(|invalidation| invalidation.id().to_owned())
        .ok_or_else(|| ShimesuError::Generic("CloudFront returned no invalidation ID".into()))
}

pub(crate) async fn load_stack_outputs(
    aws_config: &SdkConfig,
    stack_name: &str,
) -> Result<StackOutputs> {
    let cfn_client = CfnClient::new(aws_config);

    let stacks = cfn_client
        .describe_stacks()
        .stack_name(stack_name)
        .send()
        .await
        .map_err(|error| map_stack_describe_sdk_error(stack_name, &error))?;

    let stack = stacks
        .stacks()
        .first()
        .ok_or_else(|| map_stack_describe_error(stack_name, "stack not found"))?;

    Ok(stack_outputs_from_description(stack))
}

pub(crate) fn stack_outputs_from_description(stack: &Stack) -> StackOutputs {
    let mut outputs = StackOutputs::default();
    for output in stack.outputs() {
        match output.output_key() {
            Some("TableName") => outputs.table_name = output.output_value().map(String::from),
            Some("BucketName") => outputs.bucket_name = output.output_value().map(String::from),
            Some("BaseDomain") => outputs.base_domain = output.output_value().map(String::from),
            Some("CertificateArn") => {
                outputs.certificate_arn = output.output_value().map(String::from)
            }
            Some("DistributionId") => {
                outputs.distribution_id = output.output_value().map(String::from)
            }
            Some("DistributionDomainName") => {
                outputs.distribution_domain_name = output.output_value().map(String::from)
            }
            _ => {}
        }
    }

    outputs
}

pub fn parse_optional_i64(item: &HashMap<String, AttributeValue>, key: &str) -> Option<i64> {
    item.get(key)
        .and_then(|value| value.as_n().ok())
        .and_then(|value| value.parse::<i64>().ok())
}

impl StackOutputs {
    pub fn require_table_name(&self) -> Result<String> {
        self.table_name.clone().ok_or_else(|| {
            ShimesuError::Config("Stack is missing required output TableName".into())
        })
    }

    pub fn require_bucket_name(&self) -> Result<String> {
        self.bucket_name.clone().ok_or_else(|| {
            ShimesuError::Config("Stack is missing required output BucketName".into())
        })
    }

    pub fn require_base_domain(&self) -> Result<String> {
        self.base_domain.clone().ok_or_else(|| {
            ShimesuError::Config("Stack is missing required output BaseDomain".into())
        })
    }

    pub fn require_distribution_id(&self) -> Result<String> {
        self.distribution_id.clone().ok_or_else(|| {
            ShimesuError::Config("Stack is missing required output DistributionId".into())
        })
    }

    pub fn require_distribution_domain_name(&self) -> Result<String> {
        self.distribution_domain_name.clone().ok_or_else(|| {
            ShimesuError::Config("Stack is missing required output DistributionDomainName".into())
        })
    }

    pub fn into_list_outputs(self) -> Result<ListOutputs> {
        let table_name = self.require_table_name()?;
        let base_domain = self.require_base_domain()?;

        Ok(ListOutputs {
            table_name,
            base_domain,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string_attr(value: &str) -> AttributeValue {
        AttributeValue::S(value.to_string())
    }

    fn number_attr(value: i64) -> AttributeValue {
        AttributeValue::N(value.to_string())
    }

    #[test]
    fn marker_key_appends_trailing_slash() {
        assert_eq!(marker_key("demo"), "demo/");
    }

    #[test]
    fn parse_optional_i64_reads_numeric_attribute() {
        let item = HashMap::from([("file_count".to_string(), number_attr(42))]);

        assert_eq!(parse_optional_i64(&item, "file_count"), Some(42));
    }

    #[test]
    fn list_outputs_require_table_name_and_base_domain() {
        let result = StackOutputs::default().into_list_outputs();

        assert!(matches!(result, Err(ShimesuError::Config(_))));
    }

    #[test]
    fn stack_outputs_require_distribution_id() {
        let result = StackOutputs::default().require_distribution_id();

        assert!(
            matches!(result, Err(ShimesuError::Config(message)) if message.contains("DistributionId"))
        );
    }

    #[test]
    fn parse_optional_i64_rejects_non_numeric_attribute() {
        let item = HashMap::from([("file_count".to_string(), string_attr("nope"))]);

        assert_eq!(parse_optional_i64(&item, "file_count"), None);
    }
}
