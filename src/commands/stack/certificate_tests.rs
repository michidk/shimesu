use aws_sdk_cloudformation::{
    operation::create_stack::CreateStackError,
    types::{error::AlreadyExistsException, Output, Parameter, Stack, StackStatus, Tag},
};

use super::{
    certificate_stack_name, certificate_stack_state, map_certificate_create_error,
    CertificateCreateOutcome, CertificateStackState,
};

fn certificate_stack(status: StackStatus, domain: &str, arn: Option<&str>) -> Stack {
    let mut builder = Stack::builder()
        .stack_name("test-certificate")
        .stack_status(status)
        .parameters(
            Parameter::builder()
                .parameter_key("BaseDomain")
                .parameter_value(domain)
                .build(),
        )
        .tags(Tag::builder().key("Application").value("shimesu").build())
        .tags(
            Tag::builder()
                .key("StackName")
                .value("shimesu-demo")
                .build(),
        );
    if let Some(arn) = arn {
        builder = builder.outputs(
            Output::builder()
                .output_key("CertificateArn")
                .output_value(arn)
                .build(),
        );
    }
    builder.build()
}

#[test]
fn certificate_stack_name_uses_installation_stack_name() {
    let name = certificate_stack_name("shimesu-demo")
        .unwrap_or_else(|error| panic!("expected valid stack name: {error}"));

    assert_eq!(name, "shimesu-demo-certificate");
}

#[test]
fn certificate_stack_name_rejects_cloudformation_overflow() {
    let stack_name = "a".repeat(117);

    assert!(certificate_stack_name(&stack_name).is_err());
}

#[test]
fn certificate_create_race_resumes_existing_stack() {
    let error = CreateStackError::AlreadyExistsException(AlreadyExistsException::builder().build());

    let outcome = map_certificate_create_error(Some(&error), "AlreadyExistsException")
        .unwrap_or_else(|failure| panic!("expected resumable create race: {failure}"));

    assert_eq!(outcome, CertificateCreateOutcome::AlreadyExists);
}

#[test]
fn certificate_stack_is_absent_when_description_is_empty() {
    let state = certificate_stack_state(
        None,
        "test-certificate",
        "shimesu-demo",
        "static.example.com",
    )
    .unwrap_or_else(|error| panic!("expected absent state: {error}"));

    assert_eq!(state, CertificateStackState::Absent);
}

#[test]
fn certificate_stack_is_pending_while_creation_is_in_progress() {
    let stack = certificate_stack(StackStatus::CreateInProgress, "static.example.com", None);

    let state = certificate_stack_state(
        Some(&stack),
        "test-certificate",
        "shimesu-demo",
        "static.example.com",
    )
    .unwrap_or_else(|error| panic!("expected pending state: {error}"));

    assert_eq!(state, CertificateStackState::Pending);
}

#[test]
fn certificate_stack_is_ready_after_successful_creation() {
    let stack = certificate_stack(
        StackStatus::CreateComplete,
        "static.example.com",
        Some("arn:aws:acm:us-east-1:123456789012:certificate/example"),
    );

    let state = certificate_stack_state(
        Some(&stack),
        "test-certificate",
        "shimesu-demo",
        "static.example.com",
    )
    .unwrap_or_else(|error| panic!("expected ready state: {error}"));

    assert_eq!(
        state,
        CertificateStackState::Ready(
            "arn:aws:acm:us-east-1:123456789012:certificate/example".to_string()
        )
    );
}

#[test]
fn certificate_stack_rejects_a_different_domain() {
    let stack = certificate_stack(
        StackStatus::CreateComplete,
        "other.example.com",
        Some("arn:aws:acm:us-east-1:123456789012:certificate/example"),
    );

    let error = certificate_stack_state(
        Some(&stack),
        "test-certificate",
        "shimesu-demo",
        "static.example.com",
    )
    .expect_err("a certificate stack for another domain must be rejected");

    assert!(error
        .to_string()
        .contains("belongs to domain 'other.example.com'"));
}

#[test]
fn ready_certificate_stack_requires_certificate_arn_output() {
    let stack = certificate_stack(StackStatus::CreateComplete, "static.example.com", None);

    let error = certificate_stack_state(
        Some(&stack),
        "test-certificate",
        "shimesu-demo",
        "static.example.com",
    )
    .expect_err("a completed certificate stack without its ARN must be rejected");

    assert!(error
        .to_string()
        .contains("missing required output CertificateArn"));
}

#[test]
fn certificate_stack_without_ownership_tags_is_rejected() {
    let stack = Stack::builder()
        .stack_name("test-certificate")
        .stack_status(StackStatus::CreateComplete)
        .parameters(
            Parameter::builder()
                .parameter_key("BaseDomain")
                .parameter_value("static.example.com")
                .build(),
        )
        .build();

    let error = certificate_stack_state(
        Some(&stack),
        "test-certificate",
        "shimesu-demo",
        "static.example.com",
    )
    .expect_err("an unowned certificate stack must be rejected");

    assert!(error.to_string().contains("ownership tags"));
}
