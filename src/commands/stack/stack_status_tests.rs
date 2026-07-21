use super::*;
use crate::commands::stack::StackInitInput;

fn top_level_parameter_names(template: &str) -> Vec<String> {
    let mut parameter_names = Vec::new();
    let mut in_parameters_block = false;

    for line in template.lines() {
        let trimmed = line.trim();

        if !in_parameters_block {
            if trimmed == "Parameters:" {
                in_parameters_block = true;
            }
            continue;
        }

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let indent = line.len() - line.trim_start().len();
        if indent == 0 {
            break;
        }

        if indent == 2 && trimmed.ends_with(':') {
            parameter_names.push(trimmed.trim_end_matches(':').to_string());
        }
    }

    parameter_names
}

fn stack_init_contract_parameter_names(template: &str) -> Vec<String> {
    top_level_parameter_names(template)
        .into_iter()
        .filter(|name| name != "HostedZoneId")
        .collect()
}

#[test]
fn stack_template_contract_matches_stack_init_parameters_and_needs_no_capabilities() {
    let declared_parameters = top_level_parameter_names(STACK_TEMPLATE);
    let init_contract_parameters = stack_init_contract_parameter_names(STACK_TEMPLATE);
    let input = StackInitInput::parse(
        "pages.example.com",
        Some("arn:aws:acm:us-east-1:123456789012:certificate/12345678-abcd-1234-abcd-1234567890ab"),
    )
    .and_then(|input| input.with_hosted_zone_id(Some("Z1234567890EXAMPLE")))
    .unwrap_or_else(|error| panic!("expected valid input: {error}"));
    let cloudformation_parameters = input.cloudformation_parameters(
        "arn:aws:acm:us-east-1:123456789012:certificate/12345678-abcd-1234-abcd-1234567890ab",
    );
    let built_parameter_keys: Vec<&str> = cloudformation_parameters
        .iter()
        .map(|parameter| {
            parameter
                .parameter_key()
                .unwrap_or_else(|| panic!("expected parameter key to be present"))
        })
        .collect();

    assert!(
        STACK_TEMPLATE.contains("AWSTemplateFormatVersion"),
        "embedded template must declare an AWS template format version header"
    );
    assert_eq!(
        init_contract_parameters,
        vec![
            "BaseDomain".to_string(),
            "CertificateArn".to_string(),
            "AttachAliases".to_string(),
        ]
    );
    assert!(
        declared_parameters
            .iter()
            .any(|parameter| parameter == "HostedZoneId"),
        "embedded template should still declare HostedZoneId for optional Route 53 automation"
    );
    assert_eq!(
        built_parameter_keys,
        vec!["BaseDomain", "CertificateArn", "HostedZoneId"]
    );
    for key in built_parameter_keys {
        assert!(
            declared_parameters.iter().any(|declared| declared == key),
            "stack init parameter '{key}' must be declared in the embedded template"
        );
    }
    assert!(
        !STACK_TEMPLATE.contains("AWS::IAM::"),
        "embedded template must not add IAM resources that would require capabilities"
    );
}

#[test]
fn certificate_template_tags_the_managed_certificate() {
    assert!(CERTIFICATE_TEMPLATE.contains("InstallationStackName:"));
    assert!(CERTIFICATE_TEMPLATE.contains("Key: Application"));
    assert!(CERTIFICATE_TEMPLATE.contains("Value: shimesu"));
    assert!(CERTIFICATE_TEMPLATE.contains("Key: StackName"));
    assert!(CERTIFICATE_TEMPLATE.contains("Value: !Ref InstallationStackName"));
}

#[test]
fn stack_update_reuses_every_declared_template_parameter() {
    let declared_parameters = top_level_parameter_names(STACK_TEMPLATE);
    let declared: Vec<&str> = declared_parameters.iter().map(String::as_str).collect();

    assert_eq!(declared, super::update::UPDATE_PARAMETER_KEYS);
}

#[test]
fn stack_template_serves_friendly_error_pages() {
    assert!(
        STACK_TEMPLATE.contains("ResponsePagePath: /_shimesu/404.html"),
        "distribution must map origin errors to the packaged 404 page"
    );
    assert!(
        STACK_TEMPLATE.contains("CachePolicyId: 658327ea-f89d-4fab-a63d-7e88639e58f6"),
        "distribution must use the managed CachingOptimized cache policy"
    );
    assert!(
        !STACK_TEMPLATE.contains("ForwardedValues"),
        "deprecated ForwardedValues must not reappear"
    );
    assert!(
        STACK_TEMPLATE.contains("aws:SecureTransport"),
        "bucket policy must deny insecure transport"
    );
    assert!(
        !STACK_TEMPLATE.contains("lohr.dev"),
        "template must not hard-code a personal domain"
    );
    assert!(
        STACK_TEMPLATE.contains("IPV6Enabled: true"),
        "distribution must serve IPv6 viewers"
    );
    assert!(
        !STACK_TEMPLATE.contains("Type: CNAME"),
        "Route 53 records must use free alias records instead of billed CNAME lookups"
    );
}

#[test]
fn host_routing_function_maps_trailing_slashes_to_index_documents() {
    assert!(
        STACK_TEMPLATE.contains("request.uri.endsWith('/')"),
        "host routing must detect subdirectory URLs"
    );
    assert!(
        STACK_TEMPLATE.contains("request.uri += 'index.html'"),
        "host routing must map subdirectory URLs to their index document"
    );
}

#[test]
fn interpret_stack_status_returns_complete_for_terminal_success_states() {
    let statuses = [
        StackStatus::CreateComplete,
        StackStatus::UpdateComplete,
        StackStatus::DeleteComplete,
        StackStatus::ImportComplete,
    ];

    for status in statuses {
        assert!(matches!(interpret_stack_status(status), Ok(true)));
    }
}

#[test]
fn failure_event_message_prefers_specific_resource_validation() {
    let generic = aws_sdk_cloudformation::types::OperationEvent::builder()
        .logical_resource_id("shimesu")
        .resource_type("AWS::CloudFormation::Stack")
        .resource_status_reason("Validation failed")
        .build();
    let specific = aws_sdk_cloudformation::types::OperationEvent::builder()
        .logical_resource_id("ProjectsTable")
        .resource_type("AWS::DynamoDB::Table")
        .validation_status_reason("Resource already exists")
        .build();

    assert_eq!(
        super::failure_event_message(&[generic, specific]),
        Some("ProjectsTable: Resource already exists".to_string())
    );
}

#[test]
fn interpret_stack_status_returns_in_progress_for_pollable_states() {
    let statuses = [
        StackStatus::CreateInProgress,
        StackStatus::UpdateInProgress,
        StackStatus::DeleteInProgress,
        StackStatus::UpdateCompleteCleanupInProgress,
        StackStatus::UpdateRollbackInProgress,
        StackStatus::UpdateRollbackCompleteCleanupInProgress,
        StackStatus::ImportInProgress,
        StackStatus::ImportRollbackInProgress,
        StackStatus::ReviewInProgress,
    ];

    for status in statuses {
        assert!(matches!(interpret_stack_status(status), Ok(false)));
    }

    assert!(matches!(
        interpret_stack_status(StackStatus::from("DELETE_SKIPPED")),
        Ok(false)
    ));
}

#[test]
fn interpret_stack_status_returns_errors_for_terminal_failure_states() {
    let statuses = [
        StackStatus::CreateFailed,
        StackStatus::RollbackComplete,
        StackStatus::RollbackFailed,
        StackStatus::RollbackInProgress,
        StackStatus::DeleteFailed,
        StackStatus::UpdateFailed,
        StackStatus::UpdateRollbackComplete,
        StackStatus::UpdateRollbackFailed,
        StackStatus::ImportRollbackComplete,
        StackStatus::ImportRollbackFailed,
    ];

    for status in statuses {
        let expected_status = status.as_str().to_string();
        let result = interpret_stack_status(status);

        match result {
            Err(ShimesuError::Generic(message)) => {
                assert!(message.contains(&expected_status));
            }
            Err(other) => panic!("expected generic failure, got {other:?}"),
            Ok(value) => panic!("expected error, got {value}"),
        }
    }
}

#[test]
fn interpret_stack_status_returns_unknown_error_for_future_variants() {
    let result = interpret_stack_status(StackStatus::from("NEW_FUTURE_STATUS"));

    match result {
        Err(ShimesuError::Generic(message)) => {
            assert!(message.contains("Unknown stack status"));
            assert!(message.contains("Unknown"));
        }
        Err(other) => panic!("expected generic failure, got {other:?}"),
        Ok(value) => panic!("expected error, got {value}"),
    }
}
