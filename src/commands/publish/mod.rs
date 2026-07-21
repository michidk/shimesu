//! `publish` command: uploads a file, directory, or zip archive to a site prefix.

use crate::cli::{confirm, confirmation_mode, ConfirmationMode, Output};
use crate::commands::content_store::S3ContentStore;
use crate::commands::publish::inputs::{prepare_deployment, PreparedDeployment};
use crate::commands::publish::sync::{sync_site_files, SyncSummary};
use crate::commands::site::store::{DynamoSiteStore, SiteStore, SiteUpsert};
use crate::commands::support::{invalidate_site, load_aws_sdk_config, load_stack_outputs};
use crate::config::Config;
use crate::error::Result;
use aws_sdk_cloudfront::Client as CloudFrontClient;
use aws_sdk_dynamodb::Client as DynamoClient;
use aws_sdk_s3::Client as S3Client;
use chrono::Utc;
use serde::Serialize;
use std::path::Path;
use ulid::Ulid;

pub struct PublishRequest<'a> {
    path: &'a Path,
    site: Option<&'a str>,
}

impl<'a> PublishRequest<'a> {
    pub const fn new(path: &'a Path, site: Option<&'a str>) -> Self {
        Self { path, site }
    }
}

#[derive(Serialize)]
pub struct PublishOutput {
    pub slug: String,
    pub url: String,
    pub deployment_id: String,
    pub file_count: usize,
    pub total_bytes: u64,
    pub uploaded: usize,
    pub skipped: usize,
    pub deleted: usize,
    pub updated_at: String,
    pub invalidation_id: String,
}

pub async fn run_publish(
    config: &Config,
    output: &Output,
    request: PublishRequest<'_>,
) -> Result<()> {
    let prepared = prepare_deployment(request.path, request.site)?;
    output.progress(&format!(
        "Preparing {} file(s) for site '{}'...",
        prepared.file_count(),
        prepared.slug
    ));

    let aws_config = load_aws_sdk_config(config).await;
    let stack_outputs = load_stack_outputs(&aws_config, &config.stack_name).await?;
    let table_name = stack_outputs.require_table_name()?;
    let bucket_name = stack_outputs.require_bucket_name()?;
    let base_domain = stack_outputs.require_base_domain()?;
    let distribution_id = stack_outputs.require_distribution_id()?;
    let site_store = DynamoSiteStore::new(DynamoClient::new(&aws_config), table_name);

    if site_store.site_exists(&prepared.slug).await? {
        match confirmation_mode(
            config.yes,
            output.allows_prompts(),
            "Cannot prompt before replacing an existing site in JSON or non-interactive mode. Use --yes to continue.",
        )? {
            ConfirmationMode::Skip => {}
            ConfirmationMode::Prompt => {
                if !confirm(
                    &format!("Replace the current contents of site '{}'?", prepared.slug),
                    false,
                )? {
                    output.warn("Publish cancelled");
                    return Ok(());
                }
            }
        }
    }
    let store = S3ContentStore::new(S3Client::new(&aws_config), bucket_name);
    let sync = sync_site_files(&store, &prepared.slug, &prepared.files, output).await?;
    let deployment_id = Ulid::generate().to_string();
    let updated_at = Utc::now().to_rfc3339();
    let url = format!("https://{}.{}", prepared.slug, base_domain);
    site_store
        .record_deployment(&SiteUpsert {
            slug: &prepared.slug,
            deployment_id: &deployment_id,
            updated_at: &updated_at,
            url: &url,
            file_count: prepared.file_count(),
            total_bytes: prepared.total_bytes,
        })
        .await?;

    let cloudfront_client = CloudFrontClient::new(&aws_config);
    let invalidation_id = invalidate_site(
        &cloudfront_client,
        &distribution_id,
        &prepared.slug,
        &deployment_id,
    )
    .await?;
    emit_result(
        output,
        prepared,
        sync,
        url,
        deployment_id,
        updated_at,
        invalidation_id,
    )
}

fn emit_result(
    output: &Output,
    prepared: PreparedDeployment,
    sync: SyncSummary,
    url: String,
    deployment_id: String,
    updated_at: String,
    invalidation_id: String,
) -> Result<()> {
    let file_count = prepared.file_count();
    let result = PublishOutput {
        slug: prepared.slug,
        url,
        deployment_id,
        file_count,
        total_bytes: prepared.total_bytes,
        uploaded: sync.uploaded,
        skipped: sync.skipped,
        deleted: sync.deleted,
        updated_at,
        invalidation_id,
    };
    output.render(&result, |out, published| {
        out.ok(&format!("Published site '{}'", published.slug));
        out.kv("URL", &published.url);
        out.kv("Deployment", &published.deployment_id);
        out.kv("Files", &published.file_count.to_string());
        out.kv("Uploaded", &published.uploaded.to_string());
        out.kv("Skipped", &published.skipped.to_string());
        out.kv("Deleted", &published.deleted.to_string());
    })
}

pub(crate) mod inputs;
mod sync;

#[cfg(test)]
mod tests;
