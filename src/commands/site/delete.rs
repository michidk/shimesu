//! `site delete` command: removes site files, metadata, and cached edge content.

use crate::cli::{confirm, confirmation_mode, ConfirmationMode, Output};
use crate::commands::content_store::{ContentStore, S3ContentStore};
use crate::commands::site::store::{DynamoSiteStore, SiteStore};
use crate::commands::support::{
    invalidate_site, load_aws_sdk_config, load_stack_outputs, marker_key,
};
use crate::config::Config;
use crate::core::{normalize_slug, validate_slug};
use crate::error::Result;
use aws_sdk_cloudfront::Client as CloudFrontClient;
use aws_sdk_dynamodb::Client as DynamoClient;
use aws_sdk_s3::Client as S3Client;
use serde::Serialize;
use ulid::Ulid;

#[derive(Serialize)]
pub struct SiteDeleteOutput {
    pub slug: String,
    pub deleted_metadata: bool,
    pub deleted_files: bool,
    pub deleted_file_count: usize,
    pub kept_files: bool,
    pub invalidation_id: Option<String>,
}

fn site_prefix(slug: &str) -> String {
    marker_key(slug)
}

async fn delete_site_files<S: ContentStore>(store: &S, slug: &str) -> Result<usize> {
    let keys = store.list_keys(&site_prefix(slug)).await?;
    store.delete_keys(&keys).await?;
    Ok(keys.len())
}

pub async fn run_delete(
    config: &Config,
    output: &Output,
    slug: &str,
    keep_files: bool,
) -> Result<()> {
    let slug = normalize_slug(slug);
    let slug = slug.as_str();
    validate_slug(slug)?;

    match confirmation_mode(
        config.yes,
        output.allows_prompts(),
        "Cannot prompt for confirmation in JSON or non-interactive mode. Use --yes to skip prompts.",
    )? {
        ConfirmationMode::Skip => {}
        ConfirmationMode::Prompt => {
            let confirmed = confirm(
                &format!(
                    "Delete site '{slug}'{}?",
                    if keep_files {
                        " (metadata only, S3 files kept)"
                    } else {
                        " and all files under its S3 prefix"
                    }
                ),
                false,
            )?;
            if !confirmed {
                output.warn("Deletion cancelled");
                return Ok(());
            }
        }
    }

    output.progress(&format!("Deleting site '{slug}'..."));
    if !keep_files {
        output.warn(
            "Delete order: S3 objects under the site prefix are removed first, then metadata. If the metadata delete fails, partial cleanup may require manual repair.",
        );
    }

    let aws_config = load_aws_sdk_config(config).await;
    let stack_outputs = load_stack_outputs(&aws_config, &config.stack_name).await?;
    let table_name = stack_outputs.require_table_name()?;
    let site_store = DynamoSiteStore::new(DynamoClient::new(&aws_config), table_name);

    let deleted_file_count = if keep_files {
        0
    } else {
        stack_outputs.require_distribution_id()?;
        let bucket_name = stack_outputs.require_bucket_name()?;
        let store = S3ContentStore::new(S3Client::new(&aws_config), bucket_name);
        delete_site_files(&store, slug).await?
    };

    site_store.delete_site(slug).await?;

    let invalidation_id = if keep_files {
        None
    } else {
        let distribution_id = stack_outputs.require_distribution_id()?;
        let cloudfront_client = CloudFrontClient::new(&aws_config);
        let caller_reference = Ulid::generate().to_string();
        Some(
            invalidate_site(
                &cloudfront_client,
                &distribution_id,
                slug,
                &caller_reference,
            )
            .await?,
        )
    };

    let delete_output = SiteDeleteOutput {
        slug: slug.to_string(),
        deleted_metadata: true,
        deleted_files: !keep_files,
        deleted_file_count,
        kept_files: keep_files,
        invalidation_id,
    };

    output.render(&delete_output, |out, deleted| {
        if deleted.kept_files {
            out.ok(&format!("Deleted site '{}' metadata only", deleted.slug));
        } else {
            out.ok(&format!(
                "Deleted site '{}' and {} S3 object(s)",
                deleted.slug, deleted.deleted_file_count
            ));
            if let Some(invalidation) = &deleted.invalidation_id {
                out.kv("Invalidation", invalidation);
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::commands::content_store::testing::FakeContentStore;

    #[test]
    fn site_prefix_scopes_to_slug_directory() {
        assert_eq!(site_prefix("demo"), "demo/");
    }

    #[tokio::test]
    async fn delete_site_files_is_bounded_to_the_exact_slug_prefix() {
        let store = FakeContentStore::with_objects(&[
            ("docs/", ""),
            ("docs/index.html", "a"),
            ("docs/assets/app.js", "b"),
            ("docs-old/index.html", "keep"),
            ("other/index.html", "keep"),
        ]);

        let deleted = delete_site_files(&store, "docs")
            .await
            .expect("delete should succeed");

        assert_eq!(deleted, 3);
        assert_eq!(
            store.keys(),
            vec![
                "docs-old/index.html".to_string(),
                "other/index.html".to_string()
            ]
        );
    }

    #[test]
    fn delete_output_reports_invalidation_when_files_are_removed() {
        let output = SiteDeleteOutput {
            slug: "docs".to_string(),
            deleted_metadata: true,
            deleted_files: true,
            deleted_file_count: 3,
            kept_files: false,
            invalidation_id: Some("I2J3K4L5M6N7O8".to_string()),
        };

        let json = serde_json::to_value(&output).expect("delete output should serialize");

        assert_eq!(json["invalidation_id"], "I2J3K4L5M6N7O8");
        assert_eq!(json["deleted_file_count"], 3);
    }
}
