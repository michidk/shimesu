//! `stack teardown` command: permanently removes retained installation resources.

use aws_sdk_acm::Client as AcmClient;
use aws_sdk_cloudformation::Client as CfnClient;
use aws_sdk_dynamodb::Client as DynamoClient;
use aws_sdk_s3::Client as S3Client;
use serde::Serialize;

use crate::cli::{bool_text, Output};
use crate::commands::support::load_aws_sdk_config;
use crate::config::Config;
use crate::error::{Result, ShimesuError};

use super::certificate::{certificate_config, certificate_stack_name};
use super::teardown_delete::{
    delete_certificate_if_present, delete_owned_buckets, delete_owned_table,
    delete_stack_if_present,
};
use super::teardown_discovery::{discover_owned_buckets, discover_teardown_snapshot};

const APPLICATION_TAG_VALUE: &str = "shimesu";
const CONTENT_BUCKET_SEGMENT: &str = "-contentbucket-";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackTeardownInput {
    _confirmed_data_loss: (),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StackTeardownOutput {
    pub stack_name: String,
    pub region: String,
    pub regional_stack_deleted: bool,
    pub buckets_deleted: usize,
    pub object_versions_deleted: usize,
    pub table_deleted: bool,
    pub certificate_stack_deleted: bool,
    pub certificate_deleted: bool,
}

impl StackTeardownInput {
    pub fn parse(confirm_data_loss: bool) -> Result<Self> {
        if !confirm_data_loss {
            return Err(ShimesuError::Usage(
                "stack teardown requires --confirm-data-loss to proceed".to_string(),
            ));
        }
        Ok(Self {
            _confirmed_data_loss: (),
        })
    }
}

impl StackTeardownOutput {
    fn human_rows(&self) -> [(String, String); 8] {
        [
            ("Stack Name".to_string(), self.stack_name.clone()),
            ("Region".to_string(), self.region.clone()),
            (
                "Regional Stack Deleted".to_string(),
                bool_text(self.regional_stack_deleted).to_string(),
            ),
            (
                "Buckets Deleted".to_string(),
                self.buckets_deleted.to_string(),
            ),
            (
                "Object Versions Deleted".to_string(),
                self.object_versions_deleted.to_string(),
            ),
            (
                "Table Deleted".to_string(),
                bool_text(self.table_deleted).to_string(),
            ),
            (
                "Certificate Stack Deleted".to_string(),
                bool_text(self.certificate_stack_deleted).to_string(),
            ),
            (
                "Certificate Deleted".to_string(),
                bool_text(self.certificate_deleted).to_string(),
            ),
        ]
    }
}

pub async fn run_stack_teardown(
    config: &Config,
    output: &Output,
    _input: StackTeardownInput,
) -> Result<()> {
    let aws_config = load_aws_sdk_config(config).await;
    let certificate_stack_config = certificate_config(config);
    let certificate_aws_config = load_aws_sdk_config(&certificate_stack_config).await;
    let cert_stack_name = certificate_stack_name(&config.stack_name)?;
    let snapshot =
        discover_teardown_snapshot(&config.stack_name, &aws_config, &certificate_aws_config)
            .await?;

    output.progress(&format!(
        "Deleting regional stack '{}'...",
        config.stack_name
    ));
    let regional_stack_deleted =
        delete_stack_if_present(&CfnClient::new(&aws_config), &config.stack_name).await?;

    output.progress(&format!(
        "Discovering retained buckets for '{}'...",
        config.stack_name
    ));
    let owned_buckets = discover_owned_buckets(
        &S3Client::new(&aws_config),
        &config.stack_name,
        snapshot.regional_bucket_name.as_deref(),
    )
    .await?;
    let bucket_stats = delete_owned_buckets(&S3Client::new(&aws_config), &owned_buckets).await?;

    output.progress(&format!(
        "Deleting retained table for '{}'...",
        config.stack_name
    ));
    let table_deleted =
        delete_owned_table(&DynamoClient::new(&aws_config), &config.stack_name).await?;

    output.progress(&format!(
        "Deleting certificate stack '{cert_stack_name}'..."
    ));
    let certificate_stack_deleted = if snapshot.certificate_stack_owned {
        delete_stack_if_present(&CfnClient::new(&certificate_aws_config), &cert_stack_name).await?
    } else {
        false
    };

    output.progress("Deleting retained ACM certificate...");
    // Only delete the certificate that was created by the managed certificate stack.
    // Operator-supplied external certificates (from --certificate-arn) are never deleted.
    let certificate_deleted = delete_certificate_if_present(
        &AcmClient::new(&certificate_aws_config),
        snapshot.certificate_stack_certificate_arn.as_deref(),
        &config.stack_name,
    )
    .await?;

    let teardown_output = StackTeardownOutput {
        stack_name: config.stack_name.clone(),
        region: config.region.as_deref().unwrap_or("auto").to_string(),
        regional_stack_deleted,
        buckets_deleted: bucket_stats.buckets_deleted,
        object_versions_deleted: bucket_stats.object_versions_deleted,
        table_deleted,
        certificate_stack_deleted,
        certificate_deleted,
    };
    render_stack_teardown_output(output, &teardown_output)
}

pub(super) fn table_name(stack_name: &str) -> Result<String> {
    let table_name = format!("{stack_name}-projects");
    if table_name.len() > 255 {
        return Err(ShimesuError::Validation(
            "Teardown table name exceeds DynamoDB's 255-character limit".to_string(),
        ));
    }
    Ok(table_name)
}

pub(super) fn bucket_name_matches_stack(bucket_name: &str, stack_name: &str) -> bool {
    let prefix = format!(
        "{}{CONTENT_BUCKET_SEGMENT}",
        stack_name.to_ascii_lowercase()
    );
    bucket_name.starts_with(&prefix)
}

pub(super) fn tags_belong_to_stack(tags: &[(&str, &str)], stack_name: &str) -> bool {
    let has_application_tag = tags
        .iter()
        .any(|(key, value)| *key == "Application" && *value == APPLICATION_TAG_VALUE);
    let has_stack_tag = tags
        .iter()
        .any(|(key, value)| *key == "StackName" && *value == stack_name);
    has_application_tag && has_stack_tag
}

fn render_stack_teardown_output(
    output: &Output,
    teardown_output: &StackTeardownOutput,
) -> Result<()> {
    output.render(teardown_output, |out, teardown| {
        out.ok(&format!("Completed teardown for '{}'", teardown.stack_name));
        for (key, value) in teardown.human_rows() {
            out.kv(&key, &value);
        }
    })
}

#[cfg(test)]
#[path = "teardown_tests.rs"]
mod tests;
