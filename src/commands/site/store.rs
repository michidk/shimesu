//! DynamoDB site metadata store behind the `SiteStore` trait.

use crate::commands::aws_error::{map_sdk_error, map_site_delete_sdk_error};
use crate::error::Result;
use aws_sdk_dynamodb::types::AttributeValue;
use aws_sdk_dynamodb::Client as DynamoClient;
use std::collections::HashMap;

pub(crate) type SiteItem = HashMap<String, AttributeValue>;

pub(crate) struct SiteUpsert<'a> {
    pub slug: &'a str,
    pub deployment_id: &'a str,
    pub updated_at: &'a str,
    pub url: &'a str,
    pub file_count: usize,
    pub total_bytes: u64,
}

pub(crate) trait SiteStore: Sync {
    async fn site_exists(&self, slug: &str) -> Result<bool>;
    async fn record_deployment(&self, upsert: &SiteUpsert<'_>) -> Result<()>;
    async fn fetch_site(&self, slug: &str) -> Result<Option<SiteItem>>;
    async fn scan_sites(&self) -> Result<Vec<SiteItem>>;
    async fn delete_site(&self, slug: &str) -> Result<()>;
}

pub(crate) struct DynamoSiteStore {
    client: DynamoClient,
    table_name: String,
}

impl DynamoSiteStore {
    pub(crate) fn new(client: DynamoClient, table_name: String) -> Self {
        Self { client, table_name }
    }
}

impl SiteStore for DynamoSiteStore {
    async fn site_exists(&self, slug: &str) -> Result<bool> {
        let response = self
            .client
            .get_item()
            .table_name(&self.table_name)
            .key("slug", AttributeValue::S(slug.to_string()))
            .projection_expression("slug")
            .send()
            .await
            .map_err(|error| map_sdk_error("Failed to inspect site before publish", &error))?;
        Ok(response.item().is_some())
    }

    async fn record_deployment(&self, upsert: &SiteUpsert<'_>) -> Result<()> {
        self.client
            .update_item()
            .table_name(&self.table_name)
            .key("slug", AttributeValue::S(upsert.slug.to_string()))
            .update_expression(
                "SET created_at = if_not_exists(created_at, :now), updated_at = :now, deployment_id = :deployment_id, #url = :url, file_count = :file_count, total_bytes = :total_bytes",
            )
            .expression_attribute_names("#url", "url")
            .expression_attribute_values(":now", AttributeValue::S(upsert.updated_at.to_string()))
            .expression_attribute_values(
                ":deployment_id",
                AttributeValue::S(upsert.deployment_id.to_string()),
            )
            .expression_attribute_values(":url", AttributeValue::S(upsert.url.to_string()))
            .expression_attribute_values(
                ":file_count",
                AttributeValue::N(upsert.file_count.to_string()),
            )
            .expression_attribute_values(
                ":total_bytes",
                AttributeValue::N(upsert.total_bytes.to_string()),
            )
            .send()
            .await
            .map_err(|error| map_sdk_error("Failed to update site metadata", &error))?;
        Ok(())
    }

    async fn fetch_site(&self, slug: &str) -> Result<Option<SiteItem>> {
        let response = self
            .client
            .get_item()
            .table_name(&self.table_name)
            .key("slug", AttributeValue::S(slug.to_string()))
            .send()
            .await
            .map_err(|error| map_sdk_error("Failed to inspect site", &error))?;
        Ok(response.item().cloned())
    }

    async fn scan_sites(&self) -> Result<Vec<SiteItem>> {
        let mut items = Vec::new();
        let mut exclusive_start_key: Option<SiteItem> = None;
        loop {
            let response = self
                .client
                .scan()
                .table_name(&self.table_name)
                .set_exclusive_start_key(exclusive_start_key)
                .send()
                .await
                .map_err(|error| map_sdk_error("Failed to scan sites", &error))?;
            items.extend(response.items().iter().cloned());
            exclusive_start_key = response.last_evaluated_key().cloned();
            if exclusive_start_key.is_none() {
                return Ok(items);
            }
        }
    }

    async fn delete_site(&self, slug: &str) -> Result<()> {
        self.client
            .delete_item()
            .table_name(&self.table_name)
            .key("slug", AttributeValue::S(slug.to_string()))
            .condition_expression("attribute_exists(slug)")
            .send()
            .await
            .map_err(|error| map_site_delete_sdk_error(slug, &error))?;
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use crate::error::ShimesuError;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub(crate) struct FakeSiteStore {
        pub items: Mutex<BTreeMap<String, SiteItem>>,
    }

    impl FakeSiteStore {
        pub(crate) fn with_items(items: Vec<SiteItem>) -> Self {
            let store = Self::default();
            {
                let mut guard = store.items.lock().expect("fake lock");
                for item in items {
                    let slug = item
                        .get("slug")
                        .and_then(|value| value.as_s().ok())
                        .cloned()
                        .unwrap_or_default();
                    guard.insert(slug, item);
                }
            }
            store
        }
    }

    impl SiteStore for FakeSiteStore {
        async fn site_exists(&self, slug: &str) -> Result<bool> {
            Ok(self.items.lock().expect("fake lock").contains_key(slug))
        }

        async fn record_deployment(&self, upsert: &SiteUpsert<'_>) -> Result<()> {
            let mut items = self.items.lock().expect("fake lock");
            let item = items.entry(upsert.slug.to_string()).or_default();
            item.insert(
                "slug".to_string(),
                AttributeValue::S(upsert.slug.to_string()),
            );
            item.entry("created_at".to_string())
                .or_insert_with(|| AttributeValue::S(upsert.updated_at.to_string()));
            item.insert(
                "updated_at".to_string(),
                AttributeValue::S(upsert.updated_at.to_string()),
            );
            item.insert(
                "deployment_id".to_string(),
                AttributeValue::S(upsert.deployment_id.to_string()),
            );
            item.insert(
                "file_count".to_string(),
                AttributeValue::N(upsert.file_count.to_string()),
            );
            item.insert(
                "total_bytes".to_string(),
                AttributeValue::N(upsert.total_bytes.to_string()),
            );
            Ok(())
        }

        async fn fetch_site(&self, slug: &str) -> Result<Option<SiteItem>> {
            Ok(self.items.lock().expect("fake lock").get(slug).cloned())
        }

        async fn scan_sites(&self) -> Result<Vec<SiteItem>> {
            Ok(self
                .items
                .lock()
                .expect("fake lock")
                .values()
                .cloned()
                .collect())
        }

        async fn delete_site(&self, slug: &str) -> Result<()> {
            match self.items.lock().expect("fake lock").remove(slug) {
                Some(_) => Ok(()),
                None => Err(ShimesuError::NotFound(format!("Site '{slug}' not found"))),
            }
        }
    }
}
