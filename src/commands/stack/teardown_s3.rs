//! Purges versioned S3 objects and delete markers during teardown.

use aws_sdk_s3::types::{Delete, ObjectIdentifier};
use aws_sdk_s3::Client as S3Client;

use crate::commands::aws_error::{map_sdk_error, sdk_error_code, sdk_error_text};
use crate::error::{Result, ShimesuError};

use super::teardown_discovery::BucketCandidate;

const DELETE_BATCH_LIMIT: usize = 1_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct BucketDeletionStats {
    pub(super) buckets_deleted: usize,
    pub(super) object_versions_deleted: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VersionedObject {
    key: String,
    version_id: String,
}

pub(super) async fn delete_owned_bucket(
    s3_client: &S3Client,
    bucket: &BucketCandidate,
) -> Result<BucketDeletionStats> {
    let mut object_versions_deleted = 0_usize;
    let mut key_marker = None::<String>;
    let mut version_id_marker = None::<String>;

    loop {
        let mut request = s3_client.list_object_versions().bucket(&bucket.name);
        if let Some(value) = key_marker.as_deref() {
            request = request.key_marker(value);
        }
        if let Some(value) = version_id_marker.as_deref() {
            request = request.version_id_marker(value);
        }

        let response = request.send().await.map_err(|error| {
            map_sdk_error(
                &format!("Failed to list object versions in bucket '{}'", bucket.name),
                &error,
            )
        })?;
        let objects = collect_versioned_objects(&response);

        if objects.is_empty() {
            break;
        }

        for batch in delete_batches(&objects) {
            delete_object_batch(s3_client, &bucket.name, batch).await?;
            object_versions_deleted += batch.len();
        }

        if response.is_truncated().unwrap_or(false) {
            key_marker = response.next_key_marker().map(str::to_owned);
            version_id_marker = response.next_version_id_marker().map(str::to_owned);
        } else {
            key_marker = None;
            version_id_marker = None;
        }
    }

    delete_bucket(s3_client, &bucket.name).await?;
    Ok(BucketDeletionStats {
        buckets_deleted: 1,
        object_versions_deleted,
    })
}

fn collect_versioned_objects(
    response: &aws_sdk_s3::operation::list_object_versions::ListObjectVersionsOutput,
) -> Vec<VersionedObject> {
    let versions = response
        .versions()
        .iter()
        .filter_map(|version| Some((version.key()?, version.version_id()?)));
    let delete_markers = response
        .delete_markers()
        .iter()
        .filter_map(|marker| Some((marker.key()?, marker.version_id()?)));

    versions
        .chain(delete_markers)
        .map(|(key, version_id)| VersionedObject {
            key: key.to_string(),
            version_id: version_id.to_string(),
        })
        .collect()
}

fn delete_batches(objects: &[VersionedObject]) -> Vec<&[VersionedObject]> {
    objects.chunks(DELETE_BATCH_LIMIT).collect()
}

async fn delete_object_batch(
    s3_client: &S3Client,
    bucket_name: &str,
    batch: &[VersionedObject],
) -> Result<()> {
    let objects = batch
        .iter()
        .map(|object| {
            ObjectIdentifier::builder()
                .key(&object.key)
                .version_id(&object.version_id)
                .build()
        })
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| {
            ShimesuError::Generic(format!(
                "Failed to build S3 object identifier for bucket '{bucket_name}': {error}"
            ))
        })?;
    let delete = Delete::builder()
        .set_objects(Some(objects))
        .build()
        .map_err(|error| {
            ShimesuError::Generic(format!(
                "Failed to build S3 deletion batch for bucket '{bucket_name}': {error}"
            ))
        })?;
    let response = s3_client
        .delete_objects()
        .bucket(bucket_name)
        .delete(delete)
        .send()
        .await
        .map_err(|error| {
            map_sdk_error(
                &format!("Failed to delete object versions from bucket '{bucket_name}'"),
                &error,
            )
        })?;

    if !response.errors().is_empty() {
        let details = response
            .errors()
            .iter()
            .map(|error| {
                format!(
                    "{}:{}",
                    error.key().unwrap_or("unknown"),
                    error.code().unwrap_or("UnknownError")
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Err(ShimesuError::Generic(format!(
            "Failed to delete all object versions from bucket '{bucket_name}': {details}"
        )));
    }

    Ok(())
}

async fn delete_bucket(s3_client: &S3Client, bucket_name: &str) -> Result<()> {
    match s3_client.delete_bucket().bucket(bucket_name).send().await {
        Ok(_) => Ok(()),
        Err(error) => {
            let code = sdk_error_code(&error);
            let text = sdk_error_text(&error);
            if code == Some("NoSuchBucket") || text.to_ascii_lowercase().contains("not found") {
                Ok(())
            } else {
                Err(map_sdk_error(
                    &format!("Failed to delete bucket '{bucket_name}'"),
                    &error,
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{delete_batches, VersionedObject, DELETE_BATCH_LIMIT};

    #[test]
    fn delete_batches_limits_each_request_to_one_thousand_objects() {
        let objects = (0..=DELETE_BATCH_LIMIT)
            .map(|index| VersionedObject {
                key: format!("key-{index}"),
                version_id: format!("version-{index}"),
            })
            .collect::<Vec<_>>();

        let batches = delete_batches(&objects);

        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].len(), DELETE_BATCH_LIMIT);
        assert_eq!(batches[1].len(), 1);
    }
}
