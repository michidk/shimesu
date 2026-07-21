//! `stack init` command: provisions the regional data plane and landing pages.

use aws_sdk_cloudformation::{operation::create_stack::CreateStackError, Client};
use aws_sdk_s3::Client as S3Client;
use serde::Serialize;

use super::certificate_ownership::cloudformation_ownership_tags;
use super::{
    assets::upload_landing_assets, certificate::resolve_certificate_arn, progress::StackProgress,
    wait_for_stack, STACK_TEMPLATE, STACK_TIMEOUT,
};
use crate::cli::{bool_text, Output};
use crate::commands::aws_error::{map_aws_error_with_code, sdk_error_text};
use crate::commands::support::{load_aws_sdk_config, load_stack_outputs, StackOutputs};
use crate::{
    config::Config,
    error::{Result, ShimesuError},
};

use super::input::StackInitInput;

#[derive(Debug, Clone, PartialEq, Eq)]
struct StackInitResolvedOutputs {
    base_domain: String,
    bucket_name: String,
    table_name: String,
    distribution_id: String,
    distribution_domain_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DnsRecord {
    pub name: String,
    #[serde(rename = "type")]
    pub record_type: &'static str,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StackInitOutput {
    pub stack_name: String,
    pub region: String,
    pub base_domain: String,
    pub bucket_name: String,
    pub table_name: String,
    pub distribution_id: String,
    pub distribution_domain_name: String,
    pub dns_managed: bool,
    pub dns_records: Vec<DnsRecord>,
}

impl StackInitResolvedOutputs {
    fn from_stack_outputs(outputs: &StackOutputs) -> Result<Self> {
        Ok(Self {
            base_domain: outputs.require_base_domain()?,
            bucket_name: outputs.require_bucket_name()?,
            table_name: outputs.require_table_name()?,
            distribution_id: outputs.require_distribution_id()?,
            distribution_domain_name: outputs.require_distribution_domain_name()?,
        })
    }
}

impl StackInitOutput {
    fn from_resolved_outputs(
        config: &Config,
        outputs: StackInitResolvedOutputs,
        dns_managed: bool,
    ) -> Self {
        let dns_records = build_dns_records(&outputs, dns_managed);

        Self {
            stack_name: config.stack_name.clone(),
            region: config.region.clone(),
            base_domain: outputs.base_domain,
            bucket_name: outputs.bucket_name,
            table_name: outputs.table_name,
            distribution_id: outputs.distribution_id,
            distribution_domain_name: outputs.distribution_domain_name,
            dns_managed,
            dns_records,
        }
    }

    fn human_rows(&self) -> [(&'static str, &str); 7] {
        [
            ("Stack Name", &self.stack_name),
            ("Region", &self.region),
            ("Base Domain", &self.base_domain),
            ("Bucket", &self.bucket_name),
            ("Table", &self.table_name),
            ("Distribution", &self.distribution_id),
            ("Distribution Domain", &self.distribution_domain_name),
        ]
    }
}

pub async fn run_stack_init(
    config: &Config,
    output: &Output,
    input: StackInitInput<'_>,
) -> Result<()> {
    let mut spinner = StackProgress::new(
        output,
        &format!("Creating stack '{}'...", config.stack_name),
    );
    let aws_config = load_aws_sdk_config(config).await;
    let cfn_client = aws_sdk_cloudformation::Client::new(&aws_config);
    let certificate_arn = resolve_certificate_arn(config, output, input).await?;
    create_stack(&cfn_client, &config.stack_name, input, &certificate_arn).await?;
    spinner.set_message(&format!("Waiting for stack '{}'...", config.stack_name));
    let _status = wait_for_stack(&cfn_client, &config.stack_name, STACK_TIMEOUT).await?;
    let stack_outputs = load_stack_outputs(&aws_config, &config.stack_name).await?;
    let resolved_outputs = StackInitResolvedOutputs::from_stack_outputs(&stack_outputs)?;
    spinner.set_message(&format!(
        "Uploading landing pages to '{}'...",
        resolved_outputs.bucket_name
    ));
    let s3_client = S3Client::new(&aws_config);
    upload_landing_assets(&s3_client, &resolved_outputs.bucket_name).await?;
    let init_output = StackInitOutput::from_resolved_outputs(
        config,
        resolved_outputs,
        input.hosted_zone_id().is_some(),
    );
    spinner.clear();
    render_stack_init_output(output, &init_output)
}

fn build_dns_records(outputs: &StackInitResolvedOutputs, dns_managed: bool) -> Vec<DnsRecord> {
    let names = [
        outputs.base_domain.clone(),
        format!("*.{}", outputs.base_domain),
    ];
    let record_types: &[&'static str] = if dns_managed {
        &["A", "AAAA"]
    } else {
        &["CNAME"]
    };

    names
        .into_iter()
        .flat_map(|name| {
            record_types.iter().map(move |record_type| DnsRecord {
                name: name.clone(),
                record_type,
                value: outputs.distribution_domain_name.clone(),
            })
        })
        .collect()
}

async fn create_stack(
    cfn_client: &Client,
    stack_name: &str,
    input: StackInitInput<'_>,
    certificate_arn: &str,
) -> Result<()> {
    cfn_client
        .create_stack()
        .stack_name(stack_name)
        .template_body(STACK_TEMPLATE)
        .set_parameters(Some(input.cloudformation_parameters(certificate_arn)))
        .set_tags(Some(cloudformation_ownership_tags(stack_name)))
        .send()
        .await
        .map_err(|error| {
            map_create_stack_service_error(
                stack_name,
                error.as_service_error(),
                &sdk_error_text(&error),
            )
        })?;
    Ok(())
}

fn map_create_stack_service_error(
    stack_name: &str,
    service_error: Option<&CreateStackError>,
    error_text: &str,
) -> ShimesuError {
    match service_error {
        Some(error) if error.is_already_exists_exception() => ShimesuError::Validation(format!(
            "Stack '{stack_name}' already exists. Use 'shimesu stack update' to apply template changes."
        )),
        _ => map_aws_error_with_code(
            "Failed to create stack",
            service_error.and_then(aws_sdk_cloudformation::error::ProvideErrorMetadata::code),
            error_text,
        ),
    }
}

fn render_stack_init_output(output: &Output, stack_output: &StackInitOutput) -> Result<()> {
    output.render(stack_output, |out, stack| {
        out.ok(&format!("Created stack '{}'", stack.stack_name));
        for (key, value) in stack.human_rows() {
            out.kv(key, value);
        }
        out.kv("DNS Managed", bool_text(stack.dns_managed));
        for record in &stack.dns_records {
            out.kv(
                &format!("DNS {} {}", record.record_type, record.name),
                &record.value,
            );
        }
    })
}

#[cfg(test)]
#[path = "init_flow_tests.rs"]
mod flow_tests;
