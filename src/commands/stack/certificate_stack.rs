//! Creates and inspects the us-east-1 certificate CloudFormation stack.

use aws_sdk_cloudformation::{
    operation::create_stack::CreateStackError, types::Stack, Client as CfnClient,
};
use ulid::Ulid;

use crate::commands::aws_error::{
    map_aws_error_with_code, map_stack_describe_sdk_error, sdk_error_text,
};
use crate::error::{Result, ShimesuError};

use super::certificate::{CertificateCreateOutcome, CertificateStackState};
use super::certificate_ownership::{
    cloudformation_ownership_tags, cloudformation_tags_belong_to_stack,
};
use super::{StackInitInput, CERTIFICATE_TEMPLATE};

pub(super) async fn create_certificate_stack(
    client: &CfnClient,
    stack_name: &str,
    installation_stack_name: &str,
    input: StackInitInput<'_>,
) -> Result<CertificateCreateOutcome> {
    let tags = cloudformation_ownership_tags(installation_stack_name);
    match client
        .create_stack()
        .stack_name(stack_name)
        .template_body(CERTIFICATE_TEMPLATE)
        .set_parameters(Some(
            input.certificate_stack_parameters(installation_stack_name),
        ))
        .set_tags(Some(tags))
        .disable_rollback(true)
        .timeout_in_minutes(120)
        .client_request_token(Ulid::generate().to_string())
        .send()
        .await
    {
        Ok(_) => Ok(CertificateCreateOutcome::Started),
        Err(error) => {
            map_certificate_create_error(error.as_service_error(), &sdk_error_text(&error))
        }
    }
}

pub(super) fn map_certificate_create_error(
    service_error: Option<&CreateStackError>,
    error_text: &str,
) -> Result<CertificateCreateOutcome> {
    match service_error {
        Some(error) if error.is_already_exists_exception() => {
            Ok(CertificateCreateOutcome::AlreadyExists)
        }
        _ => Err(map_aws_error_with_code(
            "Failed to create certificate stack",
            service_error.and_then(aws_sdk_cloudformation::error::ProvideErrorMetadata::code),
            error_text,
        )),
    }
}

pub(super) async fn describe_certificate_stack(
    client: &CfnClient,
    stack_name: &str,
    installation_stack_name: &str,
    domain: &str,
) -> Result<CertificateStackState> {
    let response = match client.describe_stacks().stack_name(stack_name).send().await {
        Ok(response) => response,
        Err(error) => {
            return match map_stack_describe_sdk_error(stack_name, &error) {
                ShimesuError::NotFound(_) => Ok(CertificateStackState::Absent),
                other => Err(other),
            };
        }
    };
    certificate_stack_state(
        response.stacks().first(),
        stack_name,
        installation_stack_name,
        domain,
    )
}

pub(super) fn certificate_stack_state(
    stack: Option<&Stack>,
    stack_name: &str,
    installation_stack_name: &str,
    domain: &str,
) -> Result<CertificateStackState> {
    let Some(stack) = stack else {
        return Ok(CertificateStackState::Absent);
    };
    if !cloudformation_tags_belong_to_stack(stack.tags(), installation_stack_name) {
        return Err(ShimesuError::Validation(format!(
            "Refusing to reuse certificate stack '{stack_name}' because it is missing required shimesu ownership tags"
        )));
    }
    validate_stack_domain(stack, domain)?;

    match stack.stack_status().map(|status| status.as_str()) {
        Some("CREATE_COMPLETE" | "UPDATE_COMPLETE") => Ok(CertificateStackState::Ready(
            certificate_arn_from_description(stack, stack_name)?,
        )),
        Some("CREATE_IN_PROGRESS" | "UPDATE_IN_PROGRESS") => {
            Ok(CertificateStackState::Pending)
        }
        Some(status) => Err(ShimesuError::Validation(format!(
            "Certificate stack '{stack_name}' is in state {status}; resolve or delete it before retrying"
        ))),
        None => Err(ShimesuError::Generic(format!(
            "Certificate stack '{stack_name}' returned no status"
        ))),
    }
}

fn validate_stack_domain(stack: &Stack, domain: &str) -> Result<()> {
    let stack_domain = stack
        .parameters()
        .iter()
        .find(|parameter| parameter.parameter_key() == Some("BaseDomain"))
        .and_then(|parameter| parameter.parameter_value());
    if stack_domain == Some(domain) {
        return Ok(());
    }
    Err(ShimesuError::Validation(format!(
        "Certificate stack '{}' belongs to domain '{}', not '{domain}'",
        stack.stack_name().unwrap_or("unknown"),
        stack_domain.unwrap_or("unknown")
    )))
}

fn certificate_arn_from_description(stack: &Stack, stack_name: &str) -> Result<String> {
    stack
        .outputs()
        .iter()
        .find(|output| output.output_key() == Some("CertificateArn"))
        .and_then(|output| output.output_value())
        .map(String::from)
        .ok_or_else(|| {
            ShimesuError::Config(format!(
                "Certificate stack '{stack_name}' is missing required output CertificateArn"
            ))
        })
}

pub(super) async fn certificate_arn_from_stack(
    client: &CfnClient,
    stack_name: &str,
    installation_stack_name: &str,
    domain: &str,
) -> Result<String> {
    match describe_certificate_stack(client, stack_name, installation_stack_name, domain).await? {
        CertificateStackState::Ready(certificate_arn) => Ok(certificate_arn),
        CertificateStackState::Absent | CertificateStackState::Pending => {
            Err(ShimesuError::Generic(format!(
                "Certificate stack '{stack_name}' completed without an issued certificate"
            )))
        }
    }
}
