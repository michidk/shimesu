//! S3-backed content storage: uploads with cache headers, listing, and batched deletes.

use crate::commands::aws_error::map_sdk_error;
use crate::commands::publish::inputs::PreparedFile;
use crate::error::{Result, ShimesuError};
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{Delete, ObjectIdentifier};
use aws_sdk_s3::Client as S3Client;

const DELETE_BATCH_SIZE: usize = 1_000;
const HTML_CACHE_CONTROL: &str = "public, max-age=60";
const ASSET_CACHE_CONTROL: &str = "public, max-age=86400";

pub(super) fn cache_control_for(content_type: &str) -> &'static str {
    if content_type == "text/html" {
        HTML_CACHE_CONTROL
    } else {
        ASSET_CACHE_CONTROL
    }
}

pub(super) trait ContentStore: Sync {
    async fn list_keys(&self, prefix: &str) -> Result<Vec<String>>;
    async fn head_sha256(&self, key: &str) -> Result<Option<String>>;
    async fn upload(&self, file: &PreparedFile) -> Result<()>;
    async fn put_marker(&self, key: &str) -> Result<()>;
    async fn delete_keys(&self, keys: &[String]) -> Result<()>;
}

pub(super) struct S3ContentStore {
    client: S3Client,
    bucket: String,
}

impl S3ContentStore {
    pub(super) fn new(client: S3Client, bucket: String) -> Self {
        Self { client, bucket }
    }
}

impl ContentStore for S3ContentStore {
    async fn list_keys(&self, prefix: &str) -> Result<Vec<String>> {
        let mut keys = Vec::new();
        let mut continuation_token = None;
        loop {
            let response = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(prefix)
                .set_continuation_token(continuation_token)
                .send()
                .await
                .map_err(|error| map_sdk_error("Failed to list site files", &error))?;
            keys.extend(
                response
                    .contents()
                    .iter()
                    .filter_map(|object| object.key().map(ToOwned::to_owned)),
            );
            continuation_token = response.next_continuation_token().map(ToOwned::to_owned);
            if continuation_token.is_none() {
                return Ok(keys);
            }
        }
    }

    async fn head_sha256(&self, key: &str) -> Result<Option<String>> {
        let response = self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|error| map_sdk_error("Failed to inspect site file", &error))?;
        Ok(response
            .metadata()
            .and_then(|metadata| metadata.get("sha256"))
            .cloned())
    }

    async fn upload(&self, file: &PreparedFile) -> Result<()> {
        let body = ByteStream::from_path(&file.source).await.map_err(|error| {
            ShimesuError::Generic(format!(
                "Failed to read '{}' for upload: {error}",
                file.source.display()
            ))
        })?;
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&file.object_key)
            .content_type(&file.content_type)
            .cache_control(cache_control_for(&file.content_type))
            .metadata("sha256", &file.sha256)
            .body(body)
            .send()
            .await
            .map_err(|error| map_sdk_error("Failed to upload site file", &error))?;
        Ok(())
    }

    async fn put_marker(&self, key: &str) -> Result<()> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(ByteStream::from_static(b""))
            .send()
            .await
            .map_err(|error| map_sdk_error("Failed to create site marker", &error))?;
        Ok(())
    }

    async fn delete_keys(&self, keys: &[String]) -> Result<()> {
        for batch in keys.chunks(DELETE_BATCH_SIZE) {
            let objects = batch
                .iter()
                .map(|key| {
                    ObjectIdentifier::builder()
                        .key(key)
                        .build()
                        .map_err(|error| {
                            ShimesuError::Generic(format!(
                                "Failed to build S3 delete request: {error}"
                            ))
                        })
                })
                .collect::<Result<Vec<_>>>()?;
            let delete = Delete::builder()
                .set_objects(Some(objects))
                .build()
                .map_err(|error| {
                    ShimesuError::Generic(format!("Failed to build S3 delete batch: {error}"))
                })?;
            let response = self
                .client
                .delete_objects()
                .bucket(&self.bucket)
                .delete(delete)
                .send()
                .await
                .map_err(|error| map_sdk_error("Failed to delete site files", &error))?;
            if let Some(error) = response.errors().first() {
                return Err(ShimesuError::Generic(format!(
                    "Failed to delete site files: {} deleting '{}': {}",
                    error.code().unwrap_or("Unknown"),
                    error.key().unwrap_or("unknown"),
                    error.message().unwrap_or("unknown delete failure")
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::cache_control_for;

    #[test]
    fn html_gets_short_cache_and_assets_get_long_cache() {
        assert_eq!(cache_control_for("text/html"), "public, max-age=60");
        assert_eq!(cache_control_for("text/css"), "public, max-age=86400");
        assert_eq!(
            cache_control_for("application/javascript"),
            "public, max-age=86400"
        );
    }
}

#[cfg(test)]
pub(super) mod testing {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub(crate) struct FakeContentStore {
        pub objects: Mutex<BTreeMap<String, String>>,
        pub uploads: Mutex<Vec<String>>,
        pub deletes: Mutex<Vec<String>>,
    }

    impl FakeContentStore {
        pub(crate) fn with_objects(entries: &[(&str, &str)]) -> Self {
            let store = Self::default();
            store.objects.lock().expect("fake lock").extend(
                entries
                    .iter()
                    .map(|(key, sha)| ((*key).to_string(), (*sha).to_string())),
            );
            store
        }

        pub(crate) fn keys(&self) -> Vec<String> {
            self.objects
                .lock()
                .expect("fake lock")
                .keys()
                .cloned()
                .collect()
        }
    }

    impl ContentStore for FakeContentStore {
        async fn list_keys(&self, prefix: &str) -> Result<Vec<String>> {
            Ok(self
                .objects
                .lock()
                .expect("fake lock")
                .keys()
                .filter(|key| key.starts_with(prefix))
                .cloned()
                .collect())
        }

        async fn head_sha256(&self, key: &str) -> Result<Option<String>> {
            Ok(self.objects.lock().expect("fake lock").get(key).cloned())
        }

        async fn upload(&self, file: &PreparedFile) -> Result<()> {
            self.objects
                .lock()
                .expect("fake lock")
                .insert(file.object_key.clone(), file.sha256.clone());
            self.uploads
                .lock()
                .expect("fake lock")
                .push(file.object_key.clone());
            Ok(())
        }

        async fn put_marker(&self, key: &str) -> Result<()> {
            self.objects
                .lock()
                .expect("fake lock")
                .insert(key.to_string(), String::new());
            Ok(())
        }

        async fn delete_keys(&self, keys: &[String]) -> Result<()> {
            let mut objects = self.objects.lock().expect("fake lock");
            let mut deletes = self.deletes.lock().expect("fake lock");
            for key in keys {
                objects.remove(key);
                deletes.push(key.clone());
            }
            Ok(())
        }
    }
}
