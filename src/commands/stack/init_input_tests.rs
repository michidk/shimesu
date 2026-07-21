use super::StackInitInput;
use crate::error::ShimesuError;

fn valid_certificate_arn() -> &'static str {
    "arn:aws:acm:us-east-1:123456789012:certificate/12345678-abcd-1234-abcd-1234567890ab"
}

#[test]
fn stack_init_input_returns_usage_error_when_domain_is_empty() {
    let result = StackInitInput::parse("", None);

    match result {
        Err(ShimesuError::Usage(message)) => {
            assert!(message.contains("domain"));
        }
        Err(other) => panic!("expected usage error, got {other:?}"),
        Ok(_) => panic!("expected usage error for empty domain"),
    }
}

#[test]
fn stack_init_input_returns_validation_error_for_malformed_base_domain() {
    let overlong_label = "a".repeat(64);
    let malformed_domains = [
        "Pages.example.com".to_string(),
        "-pages.example.com".to_string(),
        overlong_label + ".example.com",
    ];

    for base_domain in malformed_domains {
        let result = StackInitInput::parse(&base_domain, None);

        match result {
            Err(ShimesuError::Validation(message)) => {
                assert!(message.contains("base domain"));
            }
            Err(other) => panic!("expected validation error, got {other:?}"),
            Ok(_) => panic!("expected validation error for malformed base domain"),
        }
    }
}

#[test]
fn stack_init_input_returns_validation_error_for_malformed_certificate_arn() {
    let malformed_arns = [
        "arn:aws:acm:eu-central-1:123456789012:certificate/12345678-abcd-1234-abcd-1234567890ab",
        "arn:aws:acm:us-east-1:12345:certificate/12345678-abcd-1234-abcd-1234567890ab",
        "arn:aws:acm:us-east-1:123456789012:certificate/",
    ];

    for certificate_arn in malformed_arns {
        let result = StackInitInput::parse("pages.example.com", Some(certificate_arn));

        match result {
            Err(ShimesuError::Validation(message)) => {
                assert!(message.contains("certificate arn"));
            }
            Err(other) => panic!("expected validation error, got {other:?}"),
            Ok(_) => panic!("expected validation error for malformed certificate arn"),
        }
    }
}

#[test]
fn stack_init_input_returns_validation_error_for_invalid_certificate_id_characters() {
    let result = StackInitInput::parse(
        "pages.example.com",
        Some("arn:aws:acm:us-east-1:123456789012:certificate/12345678-abcd-1234-abcd-1234567890ag"),
    );

    match result {
        Err(ShimesuError::Validation(message)) => {
            assert!(message.contains("certificate arn"));
        }
        Err(other) => panic!("expected validation error, got {other:?}"),
        Ok(_) => panic!("expected validation error for malformed certificate arn"),
    }
}

#[test]
fn stack_init_input_accepts_managed_certificate() {
    let input = StackInitInput::parse("pages.example.com", None)
        .unwrap_or_else(|error| panic!("expected valid input to parse: {error}"));

    assert_eq!(input.base_domain(), "pages.example.com");
    assert_eq!(input.certificate_arn(), None);
}

#[test]
fn stack_init_input_accepts_certificate_override() {
    let input = StackInitInput::parse("pages.example.com", Some(valid_certificate_arn()))
        .unwrap_or_else(|error| panic!("expected valid input to parse: {error}"));

    assert_eq!(input.certificate_arn(), Some(valid_certificate_arn()));
}

#[test]
fn stack_init_input_accepts_valid_hosted_zone_id() {
    let input = StackInitInput::parse("pages.example.com", None)
        .and_then(|input| input.with_hosted_zone_id(Some("Z1234567890EXAMPLE")))
        .unwrap_or_else(|error| panic!("expected valid input to parse: {error}"));

    let keys: Vec<String> = input
        .cloudformation_parameters(valid_certificate_arn())
        .iter()
        .filter_map(|parameter| parameter.parameter_key().map(String::from))
        .collect();
    assert!(keys.contains(&"HostedZoneId".to_string()));
}

#[test]
fn managed_certificate_parameters_include_domain_and_hosted_zone() {
    let input = StackInitInput::parse("pages.example.com", None)
        .and_then(|input| input.with_hosted_zone_id(Some("Z1234567890EXAMPLE")))
        .unwrap_or_else(|error| panic!("expected valid input to parse: {error}"));

    let parameters = input.certificate_stack_parameters("shimesu-demo");

    assert_eq!(parameters.len(), 3);
    assert_eq!(parameters[0].parameter_key(), Some("BaseDomain"));
    assert_eq!(parameters[0].parameter_value(), Some("pages.example.com"));
    assert_eq!(parameters[1].parameter_key(), Some("HostedZoneId"));
    assert_eq!(parameters[1].parameter_value(), Some("Z1234567890EXAMPLE"));
    assert_eq!(parameters[2].parameter_key(), Some("InstallationStackName"));
    assert_eq!(parameters[2].parameter_value(), Some("shimesu-demo"));
}

#[test]
fn stack_init_input_rejects_malformed_hosted_zone_ids() {
    for hosted_zone_id in ["", "z123lower", "A1234567890", "Z", "Z-DASHES"] {
        let result = StackInitInput::parse("pages.example.com", None)
            .and_then(|input| input.with_hosted_zone_id(Some(hosted_zone_id)));

        assert!(
            matches!(result, Err(ShimesuError::Validation(ref message)) if message.contains("hosted zone")),
            "expected validation error for '{hosted_zone_id}'"
        );
    }
}

#[test]
fn stack_init_input_omits_hosted_zone_parameter_when_not_provided() {
    let input = StackInitInput::parse("pages.example.com", None)
        .unwrap_or_else(|error| panic!("expected valid input to parse: {error}"));

    let keys: Vec<String> = input
        .cloudformation_parameters(valid_certificate_arn())
        .iter()
        .filter_map(|parameter| parameter.parameter_key().map(String::from))
        .collect();
    assert_eq!(keys, vec!["BaseDomain", "CertificateArn"]);
}
