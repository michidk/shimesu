//! Parses and validates `stack init` inputs: domain, certificate ARN, hosted zone.

use aws_sdk_cloudformation::types::Parameter;

use crate::error::{Result, ShimesuError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackInitInput<'a> {
    base_domain: &'a str,
    certificate_arn: Option<&'a str>,
    hosted_zone_id: Option<&'a str>,
}

impl<'a> StackInitInput<'a> {
    pub fn parse(base_domain: &'a str, certificate_arn: Option<&'a str>) -> Result<Self> {
        if base_domain.is_empty() {
            return Err(ShimesuError::Usage(
                "stack init requires a non-empty domain".to_string(),
            ));
        }
        validate_base_domain(base_domain)?;
        if let Some(certificate_arn) = certificate_arn {
            if certificate_arn.is_empty() {
                return Err(ShimesuError::Usage(
                    "stack init certificate arn cannot be empty".to_string(),
                ));
            }
            validate_certificate_arn(certificate_arn)?;
        }
        Ok(Self {
            base_domain,
            certificate_arn,
            hosted_zone_id: None,
        })
    }

    pub fn with_hosted_zone_id(mut self, hosted_zone_id: Option<&'a str>) -> Result<Self> {
        if let Some(hosted_zone_id) = hosted_zone_id {
            validate_hosted_zone_id(hosted_zone_id)?;
        }
        self.hosted_zone_id = hosted_zone_id;
        Ok(self)
    }

    pub const fn base_domain(&self) -> &'a str {
        self.base_domain
    }

    pub const fn certificate_arn(&self) -> Option<&'a str> {
        self.certificate_arn
    }

    pub(super) const fn hosted_zone_id(&self) -> Option<&'a str> {
        self.hosted_zone_id
    }

    pub(super) fn cloudformation_parameters(&self, certificate_arn: &str) -> Vec<Parameter> {
        let mut parameters = vec![
            Parameter::builder()
                .parameter_key("BaseDomain")
                .parameter_value(self.base_domain)
                .build(),
            Parameter::builder()
                .parameter_key("CertificateArn")
                .parameter_value(certificate_arn)
                .build(),
        ];
        if let Some(hosted_zone_id) = self.hosted_zone_id {
            parameters.push(
                Parameter::builder()
                    .parameter_key("HostedZoneId")
                    .parameter_value(hosted_zone_id)
                    .build(),
            );
        }
        parameters
    }

    pub(super) fn certificate_stack_parameters(
        &self,
        installation_stack_name: &str,
    ) -> Vec<Parameter> {
        let mut parameters = vec![Parameter::builder()
            .parameter_key("BaseDomain")
            .parameter_value(self.base_domain)
            .build()];
        if let Some(hosted_zone_id) = self.hosted_zone_id {
            parameters.push(
                Parameter::builder()
                    .parameter_key("HostedZoneId")
                    .parameter_value(hosted_zone_id)
                    .build(),
            );
        }
        parameters.push(
            Parameter::builder()
                .parameter_key("InstallationStackName")
                .parameter_value(installation_stack_name)
                .build(),
        );
        parameters
    }
}

fn validate_hosted_zone_id(hosted_zone_id: &str) -> Result<()> {
    let is_valid = (2..=33).contains(&hosted_zone_id.len())
        && hosted_zone_id.starts_with('Z')
        && hosted_zone_id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'A'..=b'Z'));

    if is_valid {
        Ok(())
    } else {
        Err(ShimesuError::Validation(
            "stack init hosted zone id must be a Route 53 hosted zone ID such as Z1234567890EXAMPLE"
                .to_string(),
        ))
    }
}

fn validate_base_domain(base_domain: &str) -> Result<()> {
    if !(1..=253).contains(&base_domain.len()) {
        return Err(ShimesuError::Validation(
            "stack init base domain must be a lowercase DNS name such as pages.example.com"
                .to_string(),
        ));
    }

    let labels: Vec<&str> = base_domain.split('.').collect();
    let Some((final_label, parent_labels)) = labels.split_last() else {
        return Err(ShimesuError::Validation(
            "stack init base domain must be a lowercase DNS name such as pages.example.com"
                .to_string(),
        ));
    };

    if parent_labels.is_empty() {
        return Err(ShimesuError::Validation(
            "stack init base domain must be a lowercase DNS name such as pages.example.com"
                .to_string(),
        ));
    }

    for label in parent_labels {
        validate_standard_dns_label(label)?;
    }

    validate_tld_label(final_label)
}

fn validate_standard_dns_label(label: &str) -> Result<()> {
    if !(1..=63).contains(&label.len()) {
        return Err(ShimesuError::Validation(
            "stack init base domain must be a lowercase DNS name such as pages.example.com"
                .to_string(),
        ));
    }

    let bytes = label.as_bytes();
    let Some(first) = bytes.first() else {
        return Err(ShimesuError::Validation(
            "stack init base domain must be a lowercase DNS name such as pages.example.com"
                .to_string(),
        ));
    };
    let Some(last) = bytes.last() else {
        return Err(ShimesuError::Validation(
            "stack init base domain must be a lowercase DNS name such as pages.example.com"
                .to_string(),
        ));
    };

    if *first == b'-' || *last == b'-' {
        return Err(ShimesuError::Validation(
            "stack init base domain must be a lowercase DNS name such as pages.example.com"
                .to_string(),
        ));
    }

    if bytes
        .iter()
        .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'))
    {
        return Ok(());
    }

    Err(ShimesuError::Validation(
        "stack init base domain must be a lowercase DNS name such as pages.example.com".to_string(),
    ))
}

fn validate_tld_label(label: &str) -> Result<()> {
    if !(2..=63).contains(&label.len()) {
        return Err(ShimesuError::Validation(
            "stack init base domain must be a lowercase DNS name such as pages.example.com"
                .to_string(),
        ));
    }

    if label.as_bytes().iter().all(u8::is_ascii_lowercase) {
        return Ok(());
    }

    Err(ShimesuError::Validation(
        "stack init base domain must be a lowercase DNS name such as pages.example.com".to_string(),
    ))
}

fn validate_certificate_arn(certificate_arn: &str) -> Result<()> {
    let parts: Vec<&str> = certificate_arn.splitn(6, ':').collect();
    let [prefix, partition, service, region, account_id, resource] = parts.as_slice() else {
        return Err(ShimesuError::Validation(
            "stack init certificate arn must be an ACM certificate ARN from us-east-1".to_string(),
        ));
    };

    let Some(certificate_id) = resource.strip_prefix("certificate/") else {
        return Err(ShimesuError::Validation(
            "stack init certificate arn must be an ACM certificate ARN from us-east-1".to_string(),
        ));
    };

    if *prefix != "arn"
        || !is_aws_partition(partition)
        || *service != "acm"
        || *region != "us-east-1"
        || !account_id
            .chars()
            .all(|character| character.is_ascii_digit())
        || account_id.len() != 12
        || certificate_id.is_empty()
        || !is_valid_certificate_id(certificate_id)
    {
        return Err(ShimesuError::Validation(
            "stack init certificate arn must be an ACM certificate ARN from us-east-1".to_string(),
        ));
    }

    Ok(())
}

fn is_valid_certificate_id(certificate_id: &str) -> bool {
    certificate_id
        .chars()
        .all(|c| matches!(c, '0'..='9' | 'a'..='f' | '-'))
}

fn is_aws_partition(partition: &str) -> bool {
    partition == "aws"
        || (partition.starts_with("aws-")
            && partition
                .as_bytes()
                .iter()
                .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-')))
}

#[cfg(test)]
#[path = "init_input_tests.rs"]
mod input_tests;
