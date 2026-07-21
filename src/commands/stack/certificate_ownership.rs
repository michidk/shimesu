//! Ownership tags proving a certificate stack belongs to this installation.

use aws_sdk_acm::types::Tag as AcmTag;
use aws_sdk_cloudformation::{
    types::{Stack, Tag as CloudFormationTag},
    Client as CfnClient,
};

use crate::commands::aws_error::map_stack_describe_sdk_error;
use crate::error::{Result, ShimesuError};

use super::certificate::certificate_stack_name;

const APPLICATION_TAG_KEY: &str = "Application";
const APPLICATION_TAG_VALUE: &str = "shimesu";
const STACK_TAG_KEY: &str = "StackName";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ManagedCertificateStack {
    Absent,
    Owned { certificate_arn: String },
}

pub(super) fn cloudformation_ownership_tags(stack_name: &str) -> Vec<CloudFormationTag> {
    [
        (APPLICATION_TAG_KEY, APPLICATION_TAG_VALUE),
        (STACK_TAG_KEY, stack_name),
    ]
    .into_iter()
    .map(|(key, value)| CloudFormationTag::builder().key(key).value(value).build())
    .collect()
}

pub(super) async fn inspect_managed_certificate_stack(
    client: &CfnClient,
    installation_stack_name: &str,
    expected_domain: Option<&str>,
) -> Result<ManagedCertificateStack> {
    let stack_name = certificate_stack_name(installation_stack_name)?;
    let response = match client
        .describe_stacks()
        .stack_name(&stack_name)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return match map_stack_describe_sdk_error(&stack_name, &error) {
                ShimesuError::NotFound(_) => Ok(ManagedCertificateStack::Absent),
                other => Err(other),
            };
        }
    };
    managed_certificate_stack_from_description(
        response.stacks().first(),
        &stack_name,
        installation_stack_name,
        expected_domain,
    )
}

fn managed_certificate_stack_from_description(
    stack: Option<&Stack>,
    stack_name: &str,
    installation_stack_name: &str,
    expected_domain: Option<&str>,
) -> Result<ManagedCertificateStack> {
    let Some(stack) = stack else {
        return Ok(ManagedCertificateStack::Absent);
    };

    if !cloudformation_tags_belong_to_stack(stack.tags(), installation_stack_name) {
        return Err(ShimesuError::Validation(format!(
            "Refusing to mutate certificate stack '{stack_name}' because it is missing required shimesu ownership tags"
        )));
    }
    if let Some(expected_domain) = expected_domain {
        let actual_domain = stack
            .parameters()
            .iter()
            .find(|parameter| parameter.parameter_key() == Some("BaseDomain"))
            .and_then(|parameter| parameter.parameter_value());
        if actual_domain != Some(expected_domain) {
            return Err(ShimesuError::Validation(format!(
                "Refusing to mutate certificate stack '{stack_name}' because its domain '{}' does not match '{expected_domain}'",
                actual_domain.unwrap_or("unknown")
            )));
        }
    }
    let certificate_arn = stack
        .outputs()
        .iter()
        .find(|output| output.output_key() == Some("CertificateArn"))
        .and_then(|output| output.output_value())
        .ok_or_else(|| {
            ShimesuError::Config(format!(
                "Certificate stack '{stack_name}' is missing required output CertificateArn"
            ))
        })?;

    Ok(ManagedCertificateStack::Owned {
        certificate_arn: certificate_arn.to_string(),
    })
}

pub(super) fn cloudformation_tags_belong_to_stack(
    tags: &[CloudFormationTag],
    installation_stack_name: &str,
) -> bool {
    let application_matches = tags.iter().any(|tag| {
        tag.key() == Some(APPLICATION_TAG_KEY) && tag.value() == Some(APPLICATION_TAG_VALUE)
    });
    let stack_matches = tags.iter().any(|tag| {
        tag.key() == Some(STACK_TAG_KEY) && tag.value() == Some(installation_stack_name)
    });
    application_matches && stack_matches
}

pub(super) fn acm_tags_belong_to_stack(tags: &[AcmTag], installation_stack_name: &str) -> bool {
    let application_matches = tags
        .iter()
        .any(|tag| tag.key() == APPLICATION_TAG_KEY && tag.value() == Some(APPLICATION_TAG_VALUE));
    let stack_matches = tags
        .iter()
        .any(|tag| tag.key() == STACK_TAG_KEY && tag.value() == Some(installation_stack_name));
    application_matches && stack_matches
}

#[cfg(test)]
mod tests {
    use aws_sdk_acm::types::Tag as AcmTag;
    use aws_sdk_cloudformation::types::{Output, Parameter, Stack, Tag as CloudFormationTag};

    use super::{
        acm_tags_belong_to_stack, cloudformation_tags_belong_to_stack,
        managed_certificate_stack_from_description, ManagedCertificateStack,
    };

    fn certificate_stack(tags: Vec<CloudFormationTag>, domain: &str) -> Stack {
        Stack::builder()
            .stack_name("shimesu-demo-certificate")
            .set_tags(Some(tags))
            .parameters(
                Parameter::builder()
                    .parameter_key("BaseDomain")
                    .parameter_value(domain)
                    .build(),
            )
            .outputs(
                Output::builder()
                    .output_key("CertificateArn")
                    .output_value("arn:aws:acm:us-east-1:123456789012:certificate/example")
                    .build(),
            )
            .build()
    }

    fn ownership_tags() -> Vec<CloudFormationTag> {
        super::cloudformation_ownership_tags("shimesu-demo")
    }

    #[test]
    fn cloudformation_ownership_requires_both_exact_tags() {
        let tags = vec![
            CloudFormationTag::builder()
                .key("Application")
                .value("shimesu")
                .build(),
            CloudFormationTag::builder()
                .key("StackName")
                .value("shimesu-demo")
                .build(),
        ];

        assert!(cloudformation_tags_belong_to_stack(&tags, "shimesu-demo"));
        assert!(!cloudformation_tags_belong_to_stack(&tags, "other"));
    }

    #[test]
    fn acm_ownership_requires_both_exact_tags() {
        let tags = vec![
            AcmTag::builder()
                .key("Application")
                .value("shimesu")
                .build()
                .unwrap_or_else(|error| panic!("expected valid ACM tag: {error}")),
            AcmTag::builder()
                .key("StackName")
                .value("shimesu-demo")
                .build()
                .unwrap_or_else(|error| panic!("expected valid ACM tag: {error}")),
        ];

        assert!(acm_tags_belong_to_stack(&tags, "shimesu-demo"));
        assert!(!acm_tags_belong_to_stack(&tags, "other"));
    }

    #[test]
    fn managed_certificate_requires_owned_stack_and_matching_domain() {
        let stack = certificate_stack(ownership_tags(), "pages.example.com");

        let result = managed_certificate_stack_from_description(
            Some(&stack),
            "shimesu-demo-certificate",
            "shimesu-demo",
            Some("pages.example.com"),
        )
        .unwrap_or_else(|error| panic!("expected owned certificate stack: {error}"));

        assert!(matches!(result, ManagedCertificateStack::Owned { .. }));
    }

    #[test]
    fn unrelated_certificate_stack_is_rejected() {
        let stack = certificate_stack(Vec::new(), "pages.example.com");

        let error = managed_certificate_stack_from_description(
            Some(&stack),
            "shimesu-demo-certificate",
            "shimesu-demo",
            Some("pages.example.com"),
        )
        .expect_err("an untagged certificate stack must be rejected");

        assert!(error.to_string().contains("ownership tags"));
    }

    #[test]
    fn certificate_stack_for_another_domain_is_rejected() {
        let stack = certificate_stack(ownership_tags(), "other.example.com");

        let error = managed_certificate_stack_from_description(
            Some(&stack),
            "shimesu-demo-certificate",
            "shimesu-demo",
            Some("pages.example.com"),
        )
        .expect_err("a mismatched certificate domain must be rejected");

        assert!(error.to_string().contains("does not match"));
    }
}
