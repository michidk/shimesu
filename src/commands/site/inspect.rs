//! `site inspect` command: shows one site record.

use crate::cli::Output;
use crate::commands::site::record::{site_from_item, Site};
use crate::commands::site::store::{DynamoSiteStore, SiteStore};
use crate::commands::support::{load_aws_sdk_config, load_stack_outputs};
use crate::config::Config;
use crate::core::{normalize_slug, validate_slug};
use crate::error::{Result, ShimesuError};
use aws_sdk_dynamodb::types::AttributeValue;
use aws_sdk_dynamodb::Client as DynamoClient;
use std::collections::HashMap;

fn site_from_lookup(
    item: Option<&HashMap<String, AttributeValue>>,
    base_domain: &str,
    slug: &str,
) -> Result<Site> {
    match item {
        Some(site) => site_from_item(site, base_domain)
            .ok_or_else(|| ShimesuError::Generic(format!("Site '{slug}' record is malformed"))),
        None => Err(ShimesuError::NotFound(format!("Site '{slug}' not found"))),
    }
}

pub async fn run_inspect(config: &Config, output: &Output, slug: &str) -> Result<()> {
    let slug = normalize_slug(slug);
    let slug = slug.as_str();
    validate_slug(slug)?;
    output.progress(&format!("Inspecting site '{slug}'..."));

    let aws_config = load_aws_sdk_config(config).await;
    let stack_outputs = load_stack_outputs(&aws_config, &config.stack_name).await?;
    let table_name = stack_outputs.require_table_name()?;
    let base_domain = stack_outputs.require_base_domain()?;
    let store = DynamoSiteStore::new(DynamoClient::new(&aws_config), table_name);

    let item = store.fetch_site(slug).await?;
    let site = site_from_lookup(item.as_ref(), &base_domain, slug)?;

    output.render(&site, |out, site| {
        out.header("Site");
        out.kv("Slug", &site.slug);
        out.kv("URL", &site.url);
        out.kv("Created", &site.created_at);
        out.kv("Updated", &site.updated_at);
        if let Some(deployment_id) = &site.deployment_id {
            out.kv("Deployment", deployment_id);
        }
        if let Some(file_count) = site.file_count {
            out.kv("File Count", &file_count.to_string());
        }
        if let Some(total_bytes) = site.total_bytes {
            out.kv("Total Bytes", &total_bytes.to_string());
        }
        out.ok(&format!("Loaded site '{}'", site.slug));
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string_attr(value: &str) -> AttributeValue {
        AttributeValue::S(value.to_string())
    }

    #[test]
    fn site_from_lookup_returns_not_found_when_item_is_missing() {
        let result = site_from_lookup(None, "example.com", "demo");

        assert!(matches!(result, Err(ShimesuError::NotFound(message)) if message.contains("demo")));
    }

    #[test]
    fn site_from_lookup_parses_present_item() {
        let item = HashMap::from([
            ("slug".to_string(), string_attr("demo")),
            (
                "created_at".to_string(),
                string_attr("2026-07-19T12:00:00Z"),
            ),
            (
                "updated_at".to_string(),
                string_attr("2026-07-19T12:05:00Z"),
            ),
        ]);

        let site = site_from_lookup(Some(&item), "example.com", "demo")
            .unwrap_or_else(|error| panic!("expected site lookup to parse: {error}"));

        assert_eq!(site.url, "https://demo.example.com");
        assert_eq!(site.slug, "demo");
    }
}
