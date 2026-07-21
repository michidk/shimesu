//! Synchronizes prepared files to S3: uploads changes, skips unchanged objects, prunes stale keys.

use crate::cli::{Output, OutputFormat};
use crate::commands::content_store::ContentStore;
use crate::commands::publish::inputs::PreparedFile;
use crate::commands::support::marker_key;
use crate::error::{Result, ShimesuError};
use futures::stream::{self, StreamExt, TryStreamExt};
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::BTreeSet;

const UPLOAD_CONCURRENCY: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SyncSummary {
    pub uploaded: usize,
    pub skipped: usize,
    pub deleted: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileOutcome {
    Uploaded,
    Skipped,
}

struct UploadProgress {
    bar: Option<ProgressBar>,
}

impl UploadProgress {
    fn new(output: &Output, total: u64) -> Self {
        let bar = (output.format() == OutputFormat::Human).then(|| {
            let bar = ProgressBar::new(total);
            bar.set_style(
                ProgressStyle::with_template("{msg} [{bar:30}] {pos}/{len}")
                    .unwrap_or_else(|_| ProgressStyle::default_bar()),
            );
            bar.set_message("Uploading");
            bar
        });
        Self { bar }
    }

    fn tick(&self) {
        if let Some(bar) = &self.bar {
            bar.inc(1);
        }
    }

    fn finish(&self) {
        if let Some(bar) = &self.bar {
            bar.finish_and_clear();
        }
    }
}

pub(super) fn should_skip_upload(existing_sha256: Option<&str>, desired_sha256: &str) -> bool {
    existing_sha256.is_some_and(|existing| existing == desired_sha256)
}

pub(super) fn stale_keys(
    existing_keys: &[String],
    desired_keys: &BTreeSet<String>,
    marker: &str,
) -> Vec<String> {
    existing_keys
        .iter()
        .filter(|key| key.as_str() != marker && !desired_keys.contains(key.as_str()))
        .cloned()
        .collect()
}

pub(super) async fn sync_site_files<S: ContentStore>(
    store: &S,
    slug: &str,
    files: &[PreparedFile],
    output: &Output,
) -> Result<SyncSummary> {
    let marker = marker_key(slug);
    let existing_keys = store.list_keys(&marker).await?;
    let existing: BTreeSet<&str> = existing_keys.iter().map(String::as_str).collect();
    let desired_keys: BTreeSet<String> = files.iter().map(|file| file.object_key.clone()).collect();

    let progress = UploadProgress::new(output, files.len() as u64);
    let outcomes: Vec<FileOutcome> = stream::iter(files.iter().map(|file| {
        let exists = existing.contains(file.object_key.as_str());
        let progress = &progress;
        async move {
            let outcome = sync_one_file(store, file, exists).await?;
            progress.tick();
            Ok::<FileOutcome, ShimesuError>(outcome)
        }
    }))
    .buffer_unordered(UPLOAD_CONCURRENCY)
    .try_collect()
    .await?;
    progress.finish();

    if !existing.contains(marker.as_str()) {
        store.put_marker(&marker).await?;
    }

    let stale = stale_keys(&existing_keys, &desired_keys, &marker);
    store.delete_keys(&stale).await?;

    let uploaded = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, FileOutcome::Uploaded))
        .count();
    Ok(SyncSummary {
        uploaded,
        skipped: outcomes.len() - uploaded,
        deleted: stale.len(),
    })
}

async fn sync_one_file<S: ContentStore>(
    store: &S,
    file: &PreparedFile,
    exists: bool,
) -> Result<FileOutcome> {
    if exists {
        let existing_sha256 = store.head_sha256(&file.object_key).await?;
        if should_skip_upload(existing_sha256.as_deref(), &file.sha256) {
            return Ok(FileOutcome::Skipped);
        }
    }
    store.upload(file).await?;
    Ok(FileOutcome::Uploaded)
}

#[cfg(test)]
mod tests {
    use super::{should_skip_upload, stale_keys, sync_site_files};
    use crate::cli::{Output, OutputFormat};
    use crate::commands::content_store::testing::FakeContentStore;
    use crate::commands::publish::inputs::PreparedFile;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn prepared(object_key: &str, sha256: &str) -> PreparedFile {
        PreparedFile {
            relative_path: PathBuf::from(object_key),
            object_key: object_key.to_string(),
            content_type: "text/html".to_string(),
            sha256: sha256.to_string(),
            size: 1,
            source: PathBuf::from("/nonexistent"),
        }
    }

    fn json_output() -> Output {
        Output::new(OutputFormat::Json)
    }

    #[tokio::test]
    async fn sync_uploads_new_files_and_creates_marker() {
        let store = FakeContentStore::default();
        let files = [prepared("pitch/index.html", "abc")];

        let summary = sync_site_files(&store, "pitch", &files, &json_output())
            .await
            .expect("sync should succeed");

        assert_eq!(summary.uploaded, 1);
        assert_eq!(summary.skipped, 0);
        assert_eq!(summary.deleted, 0);
        assert_eq!(store.keys(), vec!["pitch/", "pitch/index.html"]);
    }

    #[tokio::test]
    async fn sync_skips_unchanged_files_by_sha256() {
        let store = FakeContentStore::with_objects(&[("pitch/", ""), ("pitch/index.html", "abc")]);
        let files = [prepared("pitch/index.html", "abc")];

        let summary = sync_site_files(&store, "pitch", &files, &json_output())
            .await
            .expect("sync should succeed");

        assert_eq!(summary.uploaded, 0);
        assert_eq!(summary.skipped, 1);
        assert!(store.uploads.lock().expect("fake lock").is_empty());
    }

    #[tokio::test]
    async fn sync_reuploads_changed_files_and_prunes_stale_keys() {
        let store = FakeContentStore::with_objects(&[
            ("pitch/", ""),
            ("pitch/index.html", "old"),
            ("pitch/stale.css", "gone"),
        ]);
        let files = [prepared("pitch/index.html", "new")];

        let summary = sync_site_files(&store, "pitch", &files, &json_output())
            .await
            .expect("sync should succeed");

        assert_eq!(summary.uploaded, 1);
        assert_eq!(summary.deleted, 1);
        assert_eq!(
            *store.deletes.lock().expect("fake lock"),
            vec!["pitch/stale.css".to_string()]
        );
        assert_eq!(store.keys(), vec!["pitch/", "pitch/index.html"]);
    }

    #[tokio::test]
    async fn sync_never_touches_other_site_prefixes() {
        let store = FakeContentStore::with_objects(&[("docs/index.html", "keep")]);
        let files = [prepared("pitch/index.html", "abc")];

        sync_site_files(&store, "pitch", &files, &json_output())
            .await
            .expect("sync should succeed");

        assert!(store.keys().contains(&"docs/index.html".to_string()));
        assert!(store.deletes.lock().expect("fake lock").is_empty());
    }

    #[test]
    fn stale_keys_excludes_marker_and_desired_objects() {
        let desired = BTreeSet::from([
            "pitch/index.html".to_string(),
            "pitch/assets/app.js".to_string(),
        ]);

        let stale = stale_keys(
            &[
                "pitch/".to_string(),
                "pitch/index.html".to_string(),
                "pitch/assets/app.js".to_string(),
                "pitch/old.css".to_string(),
            ],
            &desired,
            "pitch/",
        );

        assert_eq!(stale, vec!["pitch/old.css".to_string()]);
    }

    #[test]
    fn should_skip_upload_only_when_sha256_matches() {
        assert!(should_skip_upload(Some("abc123"), "abc123"));
        assert!(!should_skip_upload(Some("abc123"), "def456"));
        assert!(!should_skip_upload(None, "abc123"));
    }
}
