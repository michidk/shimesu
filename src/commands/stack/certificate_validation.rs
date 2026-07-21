//! Validates certificate status and domain coverage for overrides.

use aws_sdk_acm::Client as AcmClient;

use crate::commands::aws_error::map_sdk_error;
use crate::error::{Result, ShimesuError};

pub(super) async fn validate_certificate_override(
    client: &AcmClient,
    certificate_arn: &str,
    domain: &str,
) -> Result<()> {
    let response = client
        .describe_certificate()
        .certificate_arn(certificate_arn)
        .send()
        .await
        .map_err(|error| map_sdk_error("Failed to validate certificate override", &error))?;
    let certificate = response.certificate().ok_or_else(|| {
        ShimesuError::Generic("ACM returned no certificate details for override".to_string())
    })?;
    validate_certificate_facts(
        certificate.status().map(|status| status.as_str()),
        certificate.subject_alternative_names(),
        domain,
    )
}

fn validate_certificate_facts(
    status: Option<&str>,
    covered_names: &[String],
    domain: &str,
) -> Result<()> {
    if status != Some("ISSUED") {
        return Err(ShimesuError::Validation(format!(
            "Certificate override must be ISSUED, but ACM reported {}",
            status.unwrap_or("unknown")
        )));
    }

    if !covered_names.iter().any(|name| name == domain) {
        return Err(ShimesuError::Validation(format!(
            "Certificate override does not cover base domain '{domain}'"
        )));
    }

    let wildcard = format!("*.{domain}");
    if !covered_names.iter().any(|name| name == &wildcard) {
        return Err(ShimesuError::Validation(format!(
            "Certificate override does not cover wildcard '{wildcard}'"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_certificate_facts;
    use crate::error::ShimesuError;

    fn covered_names() -> Vec<String> {
        vec![
            "pages.example.com".to_string(),
            "*.pages.example.com".to_string(),
        ]
    }

    #[test]
    fn issued_certificate_covering_domain_and_wildcard_is_valid() {
        assert!(
            validate_certificate_facts(Some("ISSUED"), &covered_names(), "pages.example.com")
                .is_ok()
        );
    }

    #[test]
    fn pending_certificate_is_rejected() {
        let result = validate_certificate_facts(
            Some("PENDING_VALIDATION"),
            &covered_names(),
            "pages.example.com",
        );

        assert!(
            matches!(result, Err(ShimesuError::Validation(message)) if message.contains("ISSUED"))
        );
    }

    #[test]
    fn unrelated_certificate_is_rejected() {
        let names = vec![
            "other.example.com".to_string(),
            "*.other.example.com".to_string(),
        ];

        let result = validate_certificate_facts(Some("ISSUED"), &names, "pages.example.com");

        assert!(
            matches!(result, Err(ShimesuError::Validation(message)) if message.contains("pages.example.com"))
        );
    }

    #[test]
    fn certificate_without_wildcard_is_rejected() {
        let names = vec!["pages.example.com".to_string()];

        let result = validate_certificate_facts(Some("ISSUED"), &names, "pages.example.com");

        assert!(
            matches!(result, Err(ShimesuError::Validation(message)) if message.contains("*.pages.example.com"))
        );
    }
}
