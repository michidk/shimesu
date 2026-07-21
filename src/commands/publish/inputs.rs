//! Prepares publish inputs: slug derivation, path safety, hashing, and bounded zip extraction.

use crate::core::{
    normalize_slug, validate_path, validate_slug, validate_zip, MAX_DEPLOYMENT_BYTES,
    MAX_DEPLOYMENT_FILES,
};
use crate::error::{Result, ShimesuError};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use tempfile::TempDir;
use walkdir::WalkDir;
use zip::ZipArchive;

#[derive(Debug)]
pub(crate) struct PreparedFile {
    pub relative_path: PathBuf,
    pub object_key: String,
    pub content_type: String,
    pub sha256: String,
    pub size: u64,
    pub source: PathBuf,
}

/// Files are hashed up front but streamed from `source` at upload time, so a
/// deployment never has to fit in memory. `staging` keeps extracted zip
/// contents alive until the upload finishes.
#[derive(Debug)]
pub(super) struct PreparedDeployment {
    pub slug: String,
    pub files: Vec<PreparedFile>,
    pub total_bytes: u64,
    _staging: Option<TempDir>,
}

impl PreparedDeployment {
    pub fn file_count(&self) -> usize {
        self.files.len()
    }
}

pub(super) fn derive_slug(path: &Path, site: Option<&str>) -> Result<String> {
    let candidate = match site {
        Some(slug) => slug,
        None if path.is_dir() => {
            path.file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    ShimesuError::Validation(
                        "Could not derive a site slug from the directory name".into(),
                    )
                })?
        }
        None => path
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                ShimesuError::Validation("Could not derive a site slug from the file name".into())
            })?,
    };
    let candidate = normalize_slug(candidate);
    validate_slug(&candidate)?;
    Ok(candidate)
}

pub(super) fn prepare_deployment(path: &Path, site: Option<&str>) -> Result<PreparedDeployment> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        ShimesuError::Validation(format!(
            "Cannot access deployment path '{}': {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() {
        return Err(ShimesuError::Validation(
            "Deployment path cannot be a symlink".into(),
        ));
    }

    let slug = derive_slug(path, site)?;
    let (files, staging) = if metadata.is_dir() {
        (prepare_directory(path, &slug)?, None)
    } else if metadata.is_file() && path.extension().is_some_and(|extension| extension == "zip") {
        let (files, staging) = prepare_zip(path, &slug)?;
        (files, Some(staging))
    } else if metadata.is_file() {
        (
            vec![prepare_file(path, Path::new("index.html"), &slug)?],
            None,
        )
    } else {
        return Err(ShimesuError::Validation(format!(
            "Deployment path '{}' is not a file or directory",
            path.display()
        )));
    };

    if files.is_empty() {
        return Err(ShimesuError::Validation(
            "Cannot deploy an empty deployment".into(),
        ));
    }
    let total_bytes = files.iter().try_fold(0_u64, |total, file| {
        total
            .checked_add(file.size)
            .ok_or_else(|| ShimesuError::Validation("Deployment size overflow".into()))
    })?;
    enforce_deployment_limits(files.len(), total_bytes)?;

    Ok(PreparedDeployment {
        slug,
        files,
        total_bytes,
        _staging: staging,
    })
}

pub(super) fn enforce_deployment_limits(file_count: usize, total_bytes: u64) -> Result<()> {
    if file_count > MAX_DEPLOYMENT_FILES {
        return Err(ShimesuError::Validation(format!(
            "Deployment contains too many files ({file_count} > {MAX_DEPLOYMENT_FILES})"
        )));
    }
    if total_bytes > MAX_DEPLOYMENT_BYTES {
        return Err(ShimesuError::Validation(format!(
            "Deployment size exceeds limit ({total_bytes} bytes > {MAX_DEPLOYMENT_BYTES} bytes)"
        )));
    }
    Ok(())
}

fn prepare_directory(root: &Path, slug: &str) -> Result<Vec<PreparedFile>> {
    let mut files = Vec::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| ShimesuError::Validation(error.to_string()))?;
        if entry.file_type().is_symlink() {
            return Err(ShimesuError::Validation(format!(
                "Deployment contains a symlink: {}",
                entry.path().display()
            )));
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry.path().strip_prefix(root).map_err(|error| {
            ShimesuError::Validation(format!("Failed to scope deployment path: {error}"))
        })?;
        validate_path(relative)?;
        files.push(prepare_file(entry.path(), relative, slug)?);
    }
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(files)
}

fn prepare_zip(path: &Path, slug: &str) -> Result<(Vec<PreparedFile>, TempDir)> {
    prepare_zip_with_limit(path, slug, MAX_DEPLOYMENT_BYTES)
}

fn prepare_zip_with_limit(
    path: &Path,
    slug: &str,
    max_bytes: u64,
) -> Result<(Vec<PreparedFile>, TempDir)> {
    let archive_file = fs::File::open(path)?;
    let mut archive = ZipArchive::new(archive_file)
        .map_err(|error| ShimesuError::Validation(format!("Invalid zip archive: {error}")))?;
    validate_zip(&mut archive)?;
    let staging = tempfile::tempdir()?;

    let mut files = Vec::new();
    let mut extracted_total: u64 = 0;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            ShimesuError::Validation(format!("Failed to read zip entry: {error}"))
        })?;
        if entry.is_dir() {
            continue;
        }
        let relative = entry
            .enclosed_name()
            .ok_or_else(|| ShimesuError::Validation(format!("Unsafe zip entry: {}", entry.name())))?
            .to_path_buf();
        validate_path(&relative)?;
        let destination = staging.path().join(&relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut extracted = fs::File::create(&destination)?;
        // Bound the copy by the actual decompressed byte count instead of the
        // declared entry sizes checked by validate_zip: a hostile archive can
        // understate its sizes, and an unbounded copy would fill the disk.
        let budget = max_bytes.saturating_sub(extracted_total).saturating_add(1);
        let copied = std::io::copy(&mut (&mut entry).take(budget), &mut extracted)?;
        extracted_total = extracted_total.saturating_add(copied);
        if extracted_total > max_bytes {
            return Err(ShimesuError::Validation(format!(
                "Zip decompressed size exceeds limit ({extracted_total} bytes > {max_bytes} bytes)"
            )));
        }
        files.push(prepare_file(&destination, &relative, slug)?);
    }
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok((files, staging))
}

fn prepare_file(source: &Path, relative: &Path, slug: &str) -> Result<PreparedFile> {
    let relative_path = relative.to_path_buf();
    let relative_key = relative_path
        .components()
        .map(|component| match component {
            Component::Normal(segment) => segment.to_str().ok_or_else(|| {
                ShimesuError::Validation("Deployment paths must be valid UTF-8".into())
            }),
            _ => Err(ShimesuError::Validation(
                "Deployment contains an unsafe path".into(),
            )),
        })
        .collect::<Result<Vec<_>>>()?
        .join("/");
    let content_type = mime_guess::from_path(&relative_path)
        .first_or_octet_stream()
        .essence_str()
        .to_string();
    let (sha256, size) = hash_file(source)?;
    Ok(PreparedFile {
        relative_path,
        object_key: format!("{slug}/{relative_key}"),
        content_type,
        sha256,
        size,
        source: source.to_path_buf(),
    })
}

fn hash_file(path: &Path) -> Result<(String, u64)> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut size = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }
    Ok((hex::encode(hasher.finalize()), size))
}

#[cfg(test)]
mod tests {
    use super::{derive_slug, prepare_deployment};
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    fn zip_options() -> SimpleFileOptions {
        SimpleFileOptions::default().compression_method(CompressionMethod::Stored)
    }

    #[test]
    fn derive_slug_normalizes_uppercase_explicit_site() {
        let slug = derive_slug(Path::new("pitch.html"), Some("MyReport"))
            .unwrap_or_else(|error| panic!("expected normalization: {error}"));
        assert_eq!(slug, "myreport");
    }

    #[test]
    fn derive_slug_normalizes_uppercase_file_stem() {
        let slug = derive_slug(Path::new("Pitch.html"), None)
            .unwrap_or_else(|error| panic!("expected normalization: {error}"));
        assert_eq!(slug, "pitch");
    }

    #[test]
    fn derive_slug_prefers_explicit_site() {
        let slug = derive_slug(Path::new("pitch.html"), Some("demo-site"))
            .unwrap_or_else(|error| panic!("expected explicit site slug: {error}"));

        assert_eq!(slug, "demo-site");
    }

    #[test]
    fn derive_slug_uses_file_stem_for_single_file() {
        let slug = derive_slug(Path::new("pitch.html"), None)
            .unwrap_or_else(|error| panic!("expected file stem slug: {error}"));

        assert_eq!(slug, "pitch");
    }

    #[test]
    fn derive_slug_uses_directory_name() {
        let slug = derive_slug(Path::new("site-output"), None)
            .unwrap_or_else(|error| panic!("expected directory slug: {error}"));

        assert_eq!(slug, "site-output");
    }

    #[test]
    fn prepare_deployment_maps_single_file_to_index_html() {
        let tempdir = tempdir().expect("tempdir should exist");
        let file_path = tempdir.path().join("pitch.html");
        fs::write(&file_path, "<html>pitch</html>").expect("fixture file should write");

        let prepared = prepare_deployment(&file_path, None)
            .unwrap_or_else(|error| panic!("expected single-file deployment: {error}"));

        assert_eq!(prepared.slug, "pitch");
        assert_eq!(prepared.file_count(), 1);
        assert_eq!(prepared.files[0].relative_path, PathBuf::from("index.html"));
        assert_eq!(prepared.files[0].object_key, "pitch/index.html");
        assert_eq!(prepared.files[0].content_type, "text/html");
    }

    #[test]
    fn prepare_deployment_preserves_safe_relative_paths_for_directories() {
        let tempdir = tempdir().expect("tempdir should exist");
        let root = tempdir.path().join("assets-site");
        fs::create_dir(&root).expect("root directory should exist");
        fs::create_dir_all(root.join("css")).expect("nested directory should exist");
        fs::write(root.join("index.html"), "<html></html>").expect("index should write");
        fs::write(root.join("css/app.css"), "body{}").expect("nested file should write");

        let prepared = prepare_deployment(&root, None)
            .unwrap_or_else(|error| panic!("expected directory deployment: {error}"));

        let relative_paths: Vec<PathBuf> = prepared
            .files
            .iter()
            .map(|file| file.relative_path.clone())
            .collect();
        let object_keys: Vec<String> = prepared
            .files
            .iter()
            .map(|file| file.object_key.clone())
            .collect();

        assert_eq!(prepared.slug, "assets-site");
        assert_eq!(
            relative_paths,
            vec![PathBuf::from("css/app.css"), PathBuf::from("index.html")]
        );
        assert_eq!(
            object_keys,
            vec!["assets-site/css/app.css", "assets-site/index.html"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn prepare_deployment_rejects_directory_symlinks() {
        use std::os::unix::fs as unix_fs;

        let tempdir = tempdir().expect("tempdir should exist");
        let root = tempdir.path().join("site");
        fs::create_dir(&root).expect("root directory should exist");
        fs::write(root.join("index.html"), "<html></html>").expect("index should write");
        unix_fs::symlink(root.join("index.html"), root.join("linked.html"))
            .expect("symlink fixture should exist");

        let error = prepare_deployment(&root, None).expect_err("symlink should be rejected");

        assert!(error.to_string().contains("symlink"));
    }

    #[test]
    fn prepare_deployment_extracts_valid_zip_entries() {
        let tempdir = tempdir().expect("tempdir should exist");
        let zip_path = tempdir.path().join("pitch.zip");
        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(cursor);
        writer
            .start_file("index.html", zip_options())
            .expect("zip file should start");
        std::io::Write::write_all(&mut writer, b"<html>zip</html>").expect("zip file should write");
        writer
            .start_file("assets/app.js", zip_options())
            .expect("zip nested file should start");
        std::io::Write::write_all(&mut writer, b"console.log('zip');")
            .expect("zip nested file should write");
        let buffer = writer.finish().expect("zip should finish").into_inner();
        fs::write(&zip_path, buffer).expect("zip fixture should write");

        let prepared = prepare_deployment(&zip_path, None)
            .unwrap_or_else(|error| panic!("expected zip deployment: {error}"));

        let object_keys: Vec<String> = prepared
            .files
            .iter()
            .map(|file| file.object_key.clone())
            .collect();

        assert_eq!(prepared.slug, "pitch");
        assert_eq!(object_keys, vec!["pitch/assets/app.js", "pitch/index.html"]);
    }

    #[test]
    fn prepare_zip_bounds_extraction_by_actual_decompressed_bytes() {
        let tempdir = tempdir().expect("tempdir should exist");
        let zip_path = tempdir.path().join("pitch.zip");
        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(cursor);
        writer
            .start_file("index.html", zip_options())
            .expect("zip file should start");
        std::io::Write::write_all(&mut writer, b"<html>sixteen+b</html>")
            .expect("zip file should write");
        let buffer = writer.finish().expect("zip should finish").into_inner();
        fs::write(&zip_path, buffer).expect("zip fixture should write");

        let error = super::prepare_zip_with_limit(&zip_path, "pitch", 4)
            .expect_err("extraction beyond the byte limit should fail");

        assert!(error.to_string().contains("decompressed size"));
    }

    #[test]
    fn deployment_limits_apply_to_every_input_kind() {
        use crate::core::{MAX_DEPLOYMENT_BYTES, MAX_DEPLOYMENT_FILES};

        assert!(super::enforce_deployment_limits(1, 100).is_ok());
        assert!(super::enforce_deployment_limits(MAX_DEPLOYMENT_FILES + 1, 100).is_err());
        assert!(super::enforce_deployment_limits(1, MAX_DEPLOYMENT_BYTES + 1).is_err());
    }

    #[test]
    fn prepared_files_stream_from_their_source_path() {
        let tempdir = tempdir().expect("tempdir should exist");
        let file_path = tempdir.path().join("pitch.html");
        fs::write(&file_path, "<html>pitch</html>").expect("fixture file should write");

        let prepared = prepare_deployment(&file_path, None)
            .unwrap_or_else(|error| panic!("expected single-file deployment: {error}"));

        assert_eq!(prepared.files[0].source, file_path);
        assert_eq!(prepared.files[0].size, 18);
        assert_eq!(prepared.total_bytes, 18);
    }

    #[test]
    fn prepare_deployment_rejects_empty_directory() {
        let tempdir = tempdir().expect("tempdir should exist");
        let root = tempdir.path().join("empty-site");
        fs::create_dir(&root).expect("root directory should exist");

        let error = prepare_deployment(&root, None).expect_err("empty deployment should fail");

        assert!(error.to_string().contains("empty deployment"));
    }
}
