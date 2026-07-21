//! Site record model parsed from DynamoDB items.

use crate::commands::support::parse_optional_i64;
use aws_sdk_dynamodb::types::AttributeValue;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Site {
    pub slug: String,
    pub url: String,
    pub created_at: String,
    pub updated_at: String,
    pub deployment_id: Option<String>,
    pub file_count: Option<i64>,
    pub total_bytes: Option<i64>,
}

pub fn site_from_item(item: &HashMap<String, AttributeValue>, base_domain: &str) -> Option<Site> {
    let slug = item.get("slug")?.as_s().ok()?.to_string();
    let created_at = item.get("created_at")?.as_s().ok()?.to_string();
    let updated_at = item.get("updated_at")?.as_s().ok()?.to_string();
    let deployment_id = item
        .get("deployment_id")
        .and_then(|value| value.as_s().ok())
        .map(String::from);

    Some(Site {
        slug: slug.clone(),
        url: format!("https://{slug}.{base_domain}"),
        created_at,
        updated_at,
        deployment_id,
        file_count: parse_optional_i64(item, "file_count"),
        total_bytes: parse_optional_i64(item, "total_bytes"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string_attr(value: &str) -> AttributeValue {
        AttributeValue::S(value.to_string())
    }

    fn number_attr(value: i64) -> AttributeValue {
        AttributeValue::N(value.to_string())
    }

    #[test]
    fn site_from_item_builds_site_url_and_stats() {
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
            (
                "deployment_id".to_string(),
                string_attr("01J0123456789ABCDEFGHJKMNP"),
            ),
            ("file_count".to_string(), number_attr(42)),
            ("total_bytes".to_string(), number_attr(1024)),
        ]);

        let site = site_from_item(&item, "example.com").expect("site should parse");

        assert_eq!(site.slug, "demo");
        assert_eq!(site.url, "https://demo.example.com");
        assert_eq!(site.file_count, Some(42));
        assert_eq!(site.total_bytes, Some(1024));
    }

    #[test]
    fn site_from_item_rejects_missing_required_fields() {
        let item = HashMap::from([(
            "created_at".to_string(),
            string_attr("2026-07-19T12:00:00Z"),
        )]);

        assert!(site_from_item(&item, "example.com").is_none());
    }
}
