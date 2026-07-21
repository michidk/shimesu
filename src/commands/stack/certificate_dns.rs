//! Reports ACM validation CNAME records for externally managed DNS.

use std::time::Duration;

use aws_sdk_acm::Client as AcmClient;
use aws_sdk_cloudformation::Client as CfnClient;
use tokio::time::{interval, timeout};

use crate::cli::{Output, OutputFormat};
use crate::commands::aws_error::map_sdk_error;
use crate::error::{Result, ShimesuError};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) async fn report_external_dns_validation(
    cfn_client: &CfnClient,
    acm_client: &AcmClient,
    stack_name: &str,
    output: &Output,
) -> Result<()> {
    let certificate_arn = discover_certificate_resource(cfn_client, stack_name).await?;
    let records = discover_validation_records(acm_client, &certificate_arn).await?;

    for (name, value) in records {
        let message = format!("ACM validation CNAME: {name} -> {value}");
        match output.format() {
            OutputFormat::Human => output.warn(&message),
            OutputFormat::Json => eprintln!("{message}"),
        }
    }
    Ok(())
}

async fn discover_validation_records(
    client: &AcmClient,
    certificate_arn: &str,
) -> Result<Vec<(String, String)>> {
    let poll = async {
        let mut ticker = interval(POLL_INTERVAL);
        loop {
            ticker.tick().await;
            let response = client
                .describe_certificate()
                .certificate_arn(certificate_arn)
                .send()
                .await
                .map_err(|error| {
                    map_sdk_error("Failed to read certificate validation records", &error)
                })?;
            let certificate = response.certificate().ok_or_else(|| {
                ShimesuError::Generic("ACM returned no certificate details".to_string())
            })?;
            let records = certificate
                .domain_validation_options()
                .iter()
                .filter_map(|validation| validation.resource_record())
                .map(|record| (record.name().to_string(), record.value().to_string()))
                .collect::<Vec<_>>();
            if let Some(records) = completed_validation_records(records) {
                return Ok(records);
            }
        }
    };

    timeout(DISCOVERY_TIMEOUT, poll).await.map_err(|_| {
        ShimesuError::Timeout(format!(
            "ACM did not expose DNS validation records within {} seconds",
            DISCOVERY_TIMEOUT.as_secs()
        ))
    })?
}

fn completed_validation_records(records: Vec<(String, String)>) -> Option<Vec<(String, String)>> {
    if records.is_empty() {
        None
    } else {
        Some(records)
    }
}

async fn discover_certificate_resource(client: &CfnClient, stack_name: &str) -> Result<String> {
    let poll = async {
        let mut ticker = interval(POLL_INTERVAL);
        loop {
            ticker.tick().await;
            let response = client
                .list_stack_resources()
                .stack_name(stack_name)
                .send()
                .await
                .map_err(|error| {
                    map_sdk_error("Failed to inspect certificate stack resources", &error)
                })?;
            if let Some(certificate_arn) = response
                .stack_resource_summaries()
                .iter()
                .find(|resource| resource.logical_resource_id() == Some("Certificate"))
                .and_then(|resource| resource.physical_resource_id())
            {
                return Ok(certificate_arn.to_string());
            }
        }
    };

    timeout(DISCOVERY_TIMEOUT, poll).await.map_err(|_| {
        ShimesuError::Timeout(format!(
            "Certificate stack '{stack_name}' did not expose its ACM resource within {} seconds",
            DISCOVERY_TIMEOUT.as_secs()
        ))
    })?
}

#[cfg(test)]
mod tests {
    use super::completed_validation_records;

    #[test]
    fn external_dns_discovery_does_not_complete_without_records() {
        assert_eq!(completed_validation_records(Vec::new()), None);
    }

    #[test]
    fn external_dns_discovery_completes_with_validation_records() {
        let records = vec![(
            "_token.static.example.com".to_string(),
            "_value.acm-validations.aws".to_string(),
        )];

        assert_eq!(completed_validation_records(records.clone()), Some(records));
    }
}
