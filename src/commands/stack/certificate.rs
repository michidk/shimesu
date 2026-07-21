//! Resolves the ACM certificate for `stack init`: managed stack or operator override.

use aws_sdk_acm::Client as AcmClient;
use aws_sdk_cloudformation::Client as CfnClient;

#[cfg(test)]
use super::certificate_stack::{certificate_stack_state, map_certificate_create_error};
use super::route53::validate_hosted_zone;
use super::{
    certificate_dns::report_external_dns_validation,
    certificate_stack::{
        certificate_arn_from_stack, create_certificate_stack, describe_certificate_stack,
    },
    certificate_validation::validate_certificate_override,
    wait_for_stack, StackInitInput, STACK_TIMEOUT,
};
use crate::cli::Output;
use crate::commands::support::load_aws_sdk_config;
use crate::config::Config;
use crate::error::{Result, ShimesuError};

pub(super) const CERTIFICATE_REGION: &str = "us-east-1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CertificateStackState {
    Absent,
    Pending,
    Ready(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CertificateCreateOutcome {
    Started,
    AlreadyExists,
}

pub(super) async fn resolve_certificate_arn(
    config: &Config,
    output: &Output,
    input: StackInitInput<'_>,
) -> Result<String> {
    let certificate_config = certificate_config(config);
    let aws_config = load_aws_sdk_config(&certificate_config).await;
    if let Some(hosted_zone_id) = input.hosted_zone_id() {
        validate_hosted_zone(
            &aws_sdk_route53::Client::new(&aws_config),
            hosted_zone_id,
            input.base_domain(),
        )
        .await?;
    }
    if let Some(certificate_arn) = input.certificate_arn() {
        validate_certificate_override(
            &AcmClient::new(&aws_config),
            certificate_arn,
            input.base_domain(),
        )
        .await?;
        return Ok(certificate_arn.to_string());
    }

    let stack_name = certificate_stack_name(&config.stack_name)?;
    let cfn_client = CfnClient::new(&aws_config);

    match describe_certificate_stack(
        &cfn_client,
        &stack_name,
        &config.stack_name,
        input.base_domain(),
    )
    .await?
    {
        CertificateStackState::Ready(certificate_arn) => return Ok(certificate_arn),
        CertificateStackState::Absent => {
            output.progress(&format!("Creating certificate stack '{stack_name}'..."));
            if create_certificate_stack(&cfn_client, &stack_name, &config.stack_name, input).await?
                == CertificateCreateOutcome::AlreadyExists
            {
                match describe_certificate_stack(
                    &cfn_client,
                    &stack_name,
                    &config.stack_name,
                    input.base_domain(),
                )
                .await?
                {
                    CertificateStackState::Ready(certificate_arn) => return Ok(certificate_arn),
                    CertificateStackState::Pending => {}
                    CertificateStackState::Absent => {
                        return Err(ShimesuError::Generic(format!(
                            "Certificate stack '{stack_name}' disappeared during creation"
                        )))
                    }
                }
            }
        }
        CertificateStackState::Pending => {
            output.progress(&format!("Resuming certificate stack '{stack_name}'..."));
        }
    }

    if input.hosted_zone_id().is_none() {
        report_external_dns_validation(
            &cfn_client,
            &AcmClient::new(&aws_config),
            &stack_name,
            output,
        )
        .await?;
    }

    wait_for_stack(&cfn_client, &stack_name, STACK_TIMEOUT).await?;
    certificate_arn_from_stack(
        &cfn_client,
        &stack_name,
        &config.stack_name,
        input.base_domain(),
    )
    .await
}

pub(super) fn certificate_stack_name(stack_name: &str) -> Result<String> {
    let certificate_stack_name = format!("{stack_name}-certificate");
    if certificate_stack_name.len() > 128 {
        return Err(ShimesuError::Validation(
            "Certificate stack name exceeds CloudFormation's 128-character limit".to_string(),
        ));
    }
    Ok(certificate_stack_name)
}

pub(super) fn certificate_config(config: &Config) -> Config {
    Config {
        stack_name: config.stack_name.clone(),
        region: CERTIFICATE_REGION.to_string(),
        profile: config.profile.clone(),
        json: config.json,
        yes: config.yes,
    }
}

#[cfg(test)]
#[path = "certificate_tests.rs"]
mod tests;
