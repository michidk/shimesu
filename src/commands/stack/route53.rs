//! Validates that a Route 53 hosted zone owns the requested domain.

use aws_sdk_route53::Client as Route53Client;

use crate::commands::aws_error::map_sdk_error;
use crate::error::{Result, ShimesuError};

pub(super) async fn validate_hosted_zone(
    client: &Route53Client,
    hosted_zone_id: &str,
    domain: &str,
) -> Result<()> {
    let response = client
        .get_hosted_zone()
        .id(hosted_zone_id)
        .send()
        .await
        .map_err(|error| map_sdk_error("Failed to validate Route 53 hosted zone", &error))?;
    let zone = response.hosted_zone().ok_or_else(|| {
        ShimesuError::Generic(format!(
            "Route 53 returned no hosted-zone details for '{hosted_zone_id}'"
        ))
    })?;
    if domain_belongs_to_zone(domain, zone.name()) {
        return Ok(());
    }

    Err(ShimesuError::Validation(format!(
        "Hosted zone '{}' does not contain domain '{domain}'",
        zone.name()
    )))
}

fn domain_belongs_to_zone(domain: &str, zone_name: &str) -> bool {
    let zone = zone_name.trim_end_matches('.').to_ascii_lowercase();
    domain == zone || domain.ends_with(&format!(".{zone}"))
}

#[cfg(test)]
mod tests {
    use super::domain_belongs_to_zone;

    #[test]
    fn hosted_zone_accepts_its_domain_and_subdomains() {
        assert!(domain_belongs_to_zone("example.com", "example.com."));
        assert!(domain_belongs_to_zone("pages.example.com", "example.com."));
    }

    #[test]
    fn hosted_zone_rejects_unrelated_and_suffix_lookalike_domains() {
        assert!(!domain_belongs_to_zone("pages.other.com", "example.com."));
        assert!(!domain_belongs_to_zone("notexample.com", "example.com."));
    }
}
