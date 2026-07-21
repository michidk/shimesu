//! `status` command: caller identity and regional stack health.

use crate::cli::Output;
use crate::commands::aws_error::map_stack_describe_sdk_error;
use crate::config::Config;
use crate::error::{Result, ShimesuError};
use aws_sdk_cloudformation::Client as CfnClient;
use aws_sdk_sts::Client as StsClient;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Serialize)]
pub struct StatusOutput {
    pub stack_name: String,
    pub stack_status: String,
    pub base_domain: Option<String>,
    pub bucket_name: Option<String>,
    pub distribution_id: Option<String>,
    pub table_name: Option<String>,
    pub caller_identity: CallerIdentity,
}

#[derive(Serialize)]
pub struct CallerIdentity {
    pub account: String,
    pub arn: String,
    pub user_id: String,
}

#[derive(Debug, PartialEq, Eq)]
struct RequiredStackOutputs {
    base_domain: String,
    bucket_name: String,
    distribution_id: String,
    table_name: String,
}

fn parse_required_stack_outputs(
    stack_name: &str,
    outputs: &HashMap<String, String>,
) -> Result<RequiredStackOutputs> {
    Ok(RequiredStackOutputs {
        base_domain: required_output(stack_name, outputs, "BaseDomain")?,
        bucket_name: required_output(stack_name, outputs, "BucketName")?,
        distribution_id: required_output(stack_name, outputs, "DistributionId")?,
        table_name: required_output(stack_name, outputs, "TableName")?,
    })
}

fn required_output(
    stack_name: &str,
    outputs: &HashMap<String, String>,
    key: &str,
) -> Result<String> {
    outputs.get(key).cloned().ok_or_else(|| {
        ShimesuError::Config(format!(
            "CloudFormation stack '{}' is missing required output '{key}'",
            stack_name
        ))
    })
}

fn build_status_output(
    config: &Config,
    stack_status: String,
    caller_identity: CallerIdentity,
    required_outputs: RequiredStackOutputs,
) -> StatusOutput {
    StatusOutput {
        stack_name: config.stack_name.clone(),
        stack_status,
        base_domain: Some(required_outputs.base_domain),
        bucket_name: Some(required_outputs.bucket_name),
        distribution_id: Some(required_outputs.distribution_id),
        table_name: Some(required_outputs.table_name),
        caller_identity,
    }
}

pub async fn run_status(config: &Config, output: &Output) -> Result<()> {
    output.progress("Checking AWS credentials...");

    let aws_config = crate::commands::support::load_aws_sdk_config(config).await;

    let sts_client = StsClient::new(&aws_config);
    let identity = sts_client
        .get_caller_identity()
        .send()
        .await
        .map_err(|error| {
            ShimesuError::AwsAuth(format!("Failed to get caller identity: {error}"))
        })?;

    let caller_identity = CallerIdentity {
        account: identity.account().unwrap_or("unknown").to_string(),
        arn: identity.arn().unwrap_or("unknown").to_string(),
        user_id: identity.user_id().unwrap_or("unknown").to_string(),
    };

    output.ok(&format!("Authenticated as {}", caller_identity.arn));
    output.progress(&format!("Checking stack '{}'...", config.stack_name));

    let cfn_client = CfnClient::new(&aws_config);
    let stacks = cfn_client
        .describe_stacks()
        .stack_name(&config.stack_name)
        .send()
        .await
        .map_err(|error| map_stack_describe_sdk_error(&config.stack_name, &error))?;

    let stack = stacks.stacks().first().ok_or_else(|| {
        ShimesuError::NotFound(format!("Stack '{}' not found", config.stack_name))
    })?;

    let stack_status = stack
        .stack_status()
        .map(|status| status.as_str().to_string())
        .unwrap_or_else(|| "UNKNOWN".to_string());

    let outputs: HashMap<String, String> = stack
        .outputs()
        .iter()
        .filter_map(|output_entry| {
            let key = output_entry.output_key()?;
            let value = output_entry.output_value()?;
            Some((key.to_string(), value.to_string()))
        })
        .collect();

    let required_outputs = parse_required_stack_outputs(&config.stack_name, &outputs)?;
    let status_output = build_status_output(
        config,
        stack_status.clone(),
        caller_identity,
        required_outputs,
    );

    output.render(&status_output, |out, status| {
        out.header("Stack Status");
        out.kv("Stack Name", &status.stack_name);
        out.kv("Status", &status.stack_status);

        if let Some(domain) = &status.base_domain {
            out.kv("Base Domain", domain);
        }
        if let Some(bucket) = &status.bucket_name {
            out.kv("Bucket", bucket);
        }
        if let Some(distribution) = &status.distribution_id {
            out.kv("Distribution", distribution);
        }
        if let Some(table) = &status.table_name {
            out.kv("Table", table);
        }

        out.header("Identity");
        out.kv("Account", &status.caller_identity.account);
        out.kv("ARN", &status.caller_identity.arn);

        if status.stack_status.contains("COMPLETE") && !status.stack_status.contains("DELETE") {
            out.ok("Stack is healthy");
        } else {
            out.warn(&format!("Stack status: {}", status.stack_status));
        }
    })
}
#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
