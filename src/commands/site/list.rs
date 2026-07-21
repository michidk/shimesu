//! `site list` command: lists all site records from DynamoDB.

use crate::cli::Output;
use crate::commands::site::record::{site_from_item, Site};
use crate::commands::site::store::{DynamoSiteStore, SiteStore};
use crate::commands::support::{load_aws_sdk_config, load_stack_outputs};
use crate::config::Config;
use crate::error::Result;
use aws_sdk_dynamodb::Client as DynamoClient;
use serde::Serialize;

#[derive(Serialize)]
pub struct SiteListOutput {
    pub sites: Vec<Site>,
    pub count: usize,
}

async fn collect_sites<S: SiteStore>(store: &S, base_domain: &str) -> Result<Vec<Site>> {
    let mut sites: Vec<Site> = store
        .scan_sites()
        .await?
        .iter()
        .filter_map(|item| site_from_item(item, base_domain))
        .collect();
    sites.sort_by(|left, right| left.slug.cmp(&right.slug));
    Ok(sites)
}

fn site_summary(site: &Site) -> String {
    let file_count = site
        .file_count
        .map_or_else(|| "?".to_string(), |count| count.to_string());
    format!(
        "{}  ({} file(s), updated {})",
        site.url, file_count, site.updated_at
    )
}

pub async fn run_list(config: &Config, output: &Output) -> Result<()> {
    output.progress("Fetching sites...");

    let aws_config = load_aws_sdk_config(config).await;
    let list_outputs = load_stack_outputs(&aws_config, &config.stack_name)
        .await?
        .into_list_outputs()?;
    let store = DynamoSiteStore::new(DynamoClient::new(&aws_config), list_outputs.table_name);
    let sites = collect_sites(&store, &list_outputs.base_domain).await?;

    let list_output = SiteListOutput {
        count: sites.len(),
        sites,
    };

    output.render(&list_output, |out, list| {
        out.header("Sites");
        if list.sites.is_empty() {
            out.warn("No sites found");
        } else {
            for site in &list.sites {
                out.kv(&site.slug, &site_summary(site));
            }
            out.ok(&format!("{} site(s) found", list.count));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{collect_sites, site_summary};
    use crate::commands::site::store::testing::FakeSiteStore;
    use crate::commands::site::store::SiteItem;
    use aws_sdk_dynamodb::types::AttributeValue;
    use std::collections::HashMap;

    fn item(slug: &str) -> SiteItem {
        HashMap::from([
            ("slug".to_string(), AttributeValue::S(slug.to_string())),
            (
                "created_at".to_string(),
                AttributeValue::S("2026-07-19T12:00:00Z".to_string()),
            ),
            (
                "updated_at".to_string(),
                AttributeValue::S("2026-07-19T12:05:00Z".to_string()),
            ),
            ("file_count".to_string(), AttributeValue::N("3".to_string())),
        ])
    }

    #[tokio::test]
    async fn collect_sites_sorts_by_slug_and_skips_malformed_records() {
        let malformed = HashMap::from([(
            "created_at".to_string(),
            AttributeValue::S("2026-07-19T12:00:00Z".to_string()),
        )]);
        let store = FakeSiteStore::with_items(vec![item("zeta"), item("alpha"), malformed]);

        let sites = collect_sites(&store, "example.com")
            .await
            .expect("collect should succeed");

        let slugs: Vec<&str> = sites.iter().map(|site| site.slug.as_str()).collect();
        assert_eq!(slugs, vec!["alpha", "zeta"]);
        assert_eq!(sites[0].url, "https://alpha.example.com");
    }

    #[tokio::test]
    async fn site_summary_surfaces_file_count_and_updated_at() {
        let store = FakeSiteStore::with_items(vec![item("demo")]);
        let sites = collect_sites(&store, "example.com")
            .await
            .expect("collect should succeed");

        let summary = site_summary(&sites[0]);

        assert!(summary.contains("3 file(s)"));
        assert!(summary.contains("updated 2026-07-19T12:05:00Z"));
        assert!(summary.contains("https://demo.example.com"));
    }
}
