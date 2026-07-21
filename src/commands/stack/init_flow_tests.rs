use aws_sdk_cloudformation::operation::create_stack::CreateStackError;
use aws_sdk_cloudformation::types::error::AlreadyExistsException;
use serde_json::json;

use super::{
    map_create_stack_service_error, StackInitInput, StackInitOutput, StackInitResolvedOutputs,
};
use crate::commands::support::StackOutputs;
use crate::config::Config;
use crate::error::ShimesuError;

fn config() -> Config {
    Config {
        stack_name: "shimesu-demo".to_string(),
        region: "eu-central-1".to_string(),
        profile: None,
        json: false,
        yes: false,
    }
}

fn stack_outputs() -> StackOutputs {
    StackOutputs {
        base_domain: Some("pages.example.com".to_string()),
        bucket_name: Some("bucket-demo".to_string()),
        table_name: Some("shimesu-demo-projects".to_string()),
        certificate_arn: None,
        distribution_id: Some("E1234567890".to_string()),
        distribution_domain_name: Some("d111111abcdef8.cloudfront.net".to_string()),
    }
}

#[test]
fn stack_init_parameters_include_base_domain_and_certificate_arn() {
    let input = StackInitInput::parse("pages.example.com", None)
        .unwrap_or_else(|error| panic!("expected valid input: {error}"));

    let parameters = input.cloudformation_parameters(
        "arn:aws:acm:us-east-1:123456789012:certificate/12345678-abcd-1234-abcd-1234567890ab",
    );

    assert_eq!(parameters.len(), 2);
    assert_eq!(parameters[0].parameter_key(), Some("BaseDomain"));
    assert_eq!(parameters[0].parameter_value(), Some("pages.example.com"));
    assert_eq!(parameters[1].parameter_key(), Some("CertificateArn"));
    assert_eq!(
        parameters[1].parameter_value(),
        Some("arn:aws:acm:us-east-1:123456789012:certificate/12345678-abcd-1234-abcd-1234567890ab")
    );
}

#[test]
fn stack_init_outputs_require_all_required_stack_outputs() {
    for missing_key in [
        "BaseDomain",
        "BucketName",
        "TableName",
        "DistributionId",
        "DistributionDomainName",
    ] {
        let mut outputs = stack_outputs();
        match missing_key {
            "BaseDomain" => outputs.base_domain = None,
            "BucketName" => outputs.bucket_name = None,
            "TableName" => outputs.table_name = None,
            "DistributionId" => outputs.distribution_id = None,
            "DistributionDomainName" => outputs.distribution_domain_name = None,
            other => panic!("unexpected missing output key: {other}"),
        }

        let result = StackInitResolvedOutputs::from_stack_outputs(&outputs);

        assert!(
            matches!(result, Err(ShimesuError::Config(message)) if message.contains(missing_key))
        );
    }
}

#[test]
fn stack_init_existing_stack_error_maps_to_validation() {
    let error = map_create_stack_service_error(
        "shimesu-demo",
        Some(&CreateStackError::AlreadyExistsException(
            AlreadyExistsException::builder().build(),
        )),
        "AlreadyExistsException",
    );

    assert!(
        matches!(error, ShimesuError::Validation(message) if message.contains("already exists"))
    );
}

#[test]
fn stack_init_output_serializes_stably() {
    let output = StackInitOutput::from_resolved_outputs(
        &config(),
        StackInitResolvedOutputs::from_stack_outputs(&stack_outputs())
            .unwrap_or_else(|error| panic!("expected outputs to resolve: {error}")),
        false,
    );

    let json_value = serde_json::to_value(&output)
        .unwrap_or_else(|error| panic!("expected stack init output to serialize: {error}"));

    assert_eq!(
        json_value,
        json!({
            "stack_name": "shimesu-demo",
            "region": "eu-central-1",
            "base_domain": "pages.example.com",
            "bucket_name": "bucket-demo",
            "table_name": "shimesu-demo-projects",
            "distribution_id": "E1234567890",
            "distribution_domain_name": "d111111abcdef8.cloudfront.net",
            "dns_managed": false,
            "dns_records": [
                {
                    "name": "pages.example.com",
                    "type": "CNAME",
                    "value": "d111111abcdef8.cloudfront.net"
                },
                {
                    "name": "*.pages.example.com",
                    "type": "CNAME",
                    "value": "d111111abcdef8.cloudfront.net"
                }
            ]
        })
    );
}

#[test]
fn stack_init_output_human_rows_are_stable() {
    let output = StackInitOutput::from_resolved_outputs(
        &config(),
        StackInitResolvedOutputs::from_stack_outputs(&stack_outputs())
            .unwrap_or_else(|error| panic!("expected outputs to resolve: {error}")),
        false,
    );

    assert_eq!(
        output.human_rows(),
        [
            ("Stack Name", "shimesu-demo"),
            ("Region", "eu-central-1"),
            ("Base Domain", "pages.example.com"),
            ("Bucket", "bucket-demo"),
            ("Table", "shimesu-demo-projects"),
            ("Distribution", "E1234567890"),
            ("Distribution Domain", "d111111abcdef8.cloudfront.net"),
        ]
    );

    assert_eq!(output.dns_records[0].name, "pages.example.com");
    assert_eq!(output.dns_records[0].record_type, "CNAME");
    assert_eq!(output.dns_records[0].value, "d111111abcdef8.cloudfront.net");
    assert_eq!(output.dns_records[1].name, "*.pages.example.com");
    assert_eq!(output.dns_records[1].record_type, "CNAME");
    assert_eq!(output.dns_records[1].value, "d111111abcdef8.cloudfront.net");
}

#[test]
fn stack_init_route53_output_reports_managed_alias_records() {
    let output = StackInitOutput::from_resolved_outputs(
        &config(),
        StackInitResolvedOutputs::from_stack_outputs(&stack_outputs())
            .unwrap_or_else(|error| panic!("expected outputs to resolve: {error}")),
        true,
    );

    assert!(output.dns_managed);
    assert_eq!(output.dns_records.len(), 4);
    assert_eq!(output.dns_records[0].record_type, "A");
    assert_eq!(output.dns_records[1].record_type, "AAAA");
    assert_eq!(output.dns_records[2].record_type, "A");
    assert_eq!(output.dns_records[3].record_type, "AAAA");
}
