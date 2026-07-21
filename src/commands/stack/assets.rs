//! Uploads the packaged landing and error pages to the content bucket.

use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client as S3Client;
use sha2::{Digest, Sha256};

use crate::commands::aws_error::map_sdk_error;
use crate::error::Result;

pub(super) struct LandingAsset {
    pub key: &'static str,
    pub bytes: &'static [u8],
}

pub(super) const LANDING_ASSETS: &[LandingAsset] = &[
    LandingAsset {
        key: "_shimesu/index.html",
        bytes: include_bytes!("../../../static/index.html"),
    },
    LandingAsset {
        key: "_shimesu/404.html",
        bytes: include_bytes!("../../../static/404.html"),
    },
];

pub(super) async fn upload_landing_assets(client: &S3Client, bucket_name: &str) -> Result<()> {
    for asset in LANDING_ASSETS {
        let sha256 = hex::encode(Sha256::digest(asset.bytes));

        client
            .put_object()
            .bucket(bucket_name)
            .key(asset.key)
            .content_type("text/html")
            .cache_control("no-cache")
            .metadata("sha256", &sha256)
            .body(ByteStream::from_static(asset.bytes))
            .send()
            .await
            .map_err(|error| map_sdk_error("Failed to upload installation page", &error))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::LANDING_ASSETS;

    #[test]
    fn landing_assets_cover_index_and_error_pages() {
        let keys: Vec<&str> = LANDING_ASSETS.iter().map(|asset| asset.key).collect();

        assert_eq!(keys, vec!["_shimesu/index.html", "_shimesu/404.html"]);
        for asset in LANDING_ASSETS {
            assert!(!asset.bytes.is_empty());
        }
    }
}
