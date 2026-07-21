//! Input validation: site slugs, publish paths, and zip archive safety limits.

use crate::error::{Result, ShimesuError};
use std::path::{Component, Path};
use zip::ZipArchive;

const RESERVED_SLUGS: &[&str] = &[
    "www",
    "admin",
    "api",
    "mail",
    "_acme-challenge",
    "ftp",
    "smtp",
    "pop",
    "imap",
    "ns1",
    "ns2",
];
pub const MAX_DEPLOYMENT_BYTES: u64 = 500 * 1024 * 1024;
pub const MAX_DEPLOYMENT_FILES: usize = 10_000;

/// Normalize a slug to lowercase before validation or storage.
/// DNS names and DynamoDB/S3 keys are case-sensitive in different layers;
/// normalizing at the input boundary makes them unambiguous everywhere.
pub fn normalize_slug(slug: &str) -> String {
    slug.to_ascii_lowercase()
}

pub fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty() || slug.len() > 63 {
        return Err(ShimesuError::Validation(format!(
            "Slug must be 1-63 characters, got {}",
            slug.len()
        )));
    }

    let valid_pattern = slug.chars().enumerate().all(|(index, character)| {
        if index == 0 || index == slug.len() - 1 {
            character.is_ascii_lowercase() || character.is_ascii_digit()
        } else {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        }
    });

    if !valid_pattern {
        return Err(ShimesuError::Validation(
            "Slug must be lowercase alphanumeric with hyphens (not at start/end)".into(),
        ));
    }

    if RESERVED_SLUGS.contains(&slug) {
        return Err(ShimesuError::Validation(format!(
            "'{}' is a reserved name",
            slug
        )));
    }

    Ok(())
}

pub fn validate_path(path: &Path) -> Result<()> {
    if path.is_absolute() {
        return Err(ShimesuError::Validation(
            "Absolute paths are not allowed".into(),
        ));
    }

    for (index, component) in path.components().enumerate() {
        match component {
            Component::ParentDir => {
                return Err(ShimesuError::Validation(
                    "Path traversal (..) is not allowed".into(),
                ));
            }
            Component::Normal(segment) => {
                let segment = segment.to_string_lossy();

                if index == 0 && segment.starts_with('.') {
                    return Err(ShimesuError::Validation(
                        "Hidden files at the path root are not allowed".into(),
                    ));
                }

                if segment.chars().any(char::is_control) {
                    return Err(ShimesuError::Validation(
                        "Control characters in paths are not allowed".into(),
                    ));
                }
            }
            _ => {}
        }
    }

    Ok(())
}

pub fn validate_zip<R: std::io::Read + std::io::Seek>(archive: &mut ZipArchive<R>) -> Result<()> {
    validate_zip_with_limits(archive, MAX_DEPLOYMENT_FILES, MAX_DEPLOYMENT_BYTES)
}

fn validate_zip_with_limits<R: std::io::Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    max_files: usize,
    max_bytes: u64,
) -> Result<()> {
    let file_count = archive.len();

    if file_count > max_files {
        return Err(ShimesuError::Validation(format!(
            "Zip contains too many files ({} > {})",
            file_count, max_files
        )));
    }

    let mut total_size = 0_u64;

    for index in 0..file_count {
        let file = archive.by_index(index).map_err(|error| {
            ShimesuError::Generic(format!("Failed to read zip entry: {}", error))
        })?;

        let name = file.name();
        if name.starts_with('/') || name.starts_with('\\') {
            return Err(ShimesuError::Validation(format!(
                "Zip entry has absolute path: {}",
                name
            )));
        }
        if Path::new(name).is_absolute() {
            return Err(ShimesuError::Validation(format!(
                "Zip entry has absolute path: {}",
                name
            )));
        }
        if file.is_symlink() {
            return Err(ShimesuError::Validation(format!(
                "Zip entry is a symlink (not allowed): {}",
                name
            )));
        }

        validate_path(Path::new(name))?;

        total_size += file.size();
        if total_size > max_bytes {
            return Err(ShimesuError::Validation(format!(
                "Zip uncompressed size exceeds limit ({} bytes > {} bytes)",
                total_size, max_bytes
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    fn zip_options() -> SimpleFileOptions {
        SimpleFileOptions::default().compression_method(CompressionMethod::Stored)
    }

    fn archive_from_entries(entries: &[(&str, &[u8])]) -> ZipArchive<Cursor<Vec<u8>>> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip_options();

        for (name, content) in entries {
            writer
                .start_file(*name, options)
                .expect("test zip file entry should be created");
            writer
                .write_all(content)
                .expect("test zip file content should be written");
        }

        let cursor = writer.finish().expect("test zip archive should finish");
        ZipArchive::new(Cursor::new(cursor.into_inner())).expect("test zip archive should open")
    }

    #[test]
    fn normalize_slug_lowercases_ascii() {
        assert_eq!(normalize_slug("DEMO"), "demo");
        assert_eq!(normalize_slug("My-Site"), "my-site");
        assert_eq!(normalize_slug("Report123"), "report123");
        assert_eq!(normalize_slug("already-lower"), "already-lower");
    }

    #[test]
    fn test_valid_slugs() {
        assert!(validate_slug("demo").is_ok());
        assert!(validate_slug("my-site").is_ok());
        assert!(validate_slug("a").is_ok());
        assert!(validate_slug("a1").is_ok());
        assert!(validate_slug("site123").is_ok());
    }

    #[test]
    fn test_invalid_slugs() {
        assert!(validate_slug("DEMO").is_err());
        assert!(validate_slug("Demo").is_err());
        assert!(validate_slug("my_site").is_err());
        assert!(validate_slug("my.site").is_err());
        assert!(validate_slug("my site").is_err());
        assert!(validate_slug("-demo").is_err());
        assert!(validate_slug("demo-").is_err());
        assert!(validate_slug(&"a".repeat(64)).is_err());
        assert!(validate_slug("").is_err());
        assert!(validate_slug("../etc").is_err());
    }

    #[test]
    fn test_reserved_slugs() {
        assert!(validate_slug("www").is_err());
        assert!(validate_slug("admin").is_err());
        assert!(validate_slug("api").is_err());
        assert!(validate_slug("mail").is_err());
        assert!(validate_slug("ns1").is_err());
    }

    #[test]
    fn test_valid_paths() {
        assert!(validate_path(Path::new("index.html")).is_ok());
        assert!(validate_path(Path::new("assets/style.css")).is_ok());
        assert!(validate_path(Path::new("deep/nested/file.js")).is_ok());
    }

    #[test]
    fn test_invalid_paths() {
        assert!(validate_path(Path::new("../etc/passwd")).is_err());
        assert!(validate_path(Path::new("foo/../bar")).is_err());
        assert!(validate_path(Path::new("/etc/passwd")).is_err());
        assert!(validate_path(Path::new(".env")).is_err());
        assert!(validate_path(Path::new("dir/\u{0000}file.txt")).is_err());
    }

    #[test]
    fn test_valid_zip_archive() {
        let mut archive = archive_from_entries(&[("assets/index.html", b"<html></html>")]);

        assert!(validate_zip(&mut archive).is_ok());
    }

    #[test]
    fn test_zip_accepts_double_dots_inside_filename() {
        let mut archive = archive_from_entries(&[("notes..final.txt", b"safe")]);

        assert!(validate_zip(&mut archive).is_ok());
    }

    #[test]
    fn test_zip_rejects_path_traversal() {
        let mut archive = archive_from_entries(&[("../etc/passwd", b"oops")]);

        assert!(validate_zip(&mut archive).is_err());
    }

    #[test]
    fn test_zip_rejects_absolute_paths() {
        let mut archive = archive_from_entries(&[("/etc/passwd", b"oops")]);

        assert!(validate_zip(&mut archive).is_err());
    }

    #[test]
    fn test_zip_rejects_symlinks() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .add_symlink("current", "target", zip_options())
            .expect("test symlink should be created");
        let cursor = writer.finish().expect("test zip archive should finish");
        let mut archive = ZipArchive::new(Cursor::new(cursor.into_inner()))
            .expect("test zip archive should open");

        assert!(validate_zip(&mut archive).is_err());
    }

    #[test]
    fn test_zip_rejects_size_bomb() {
        let mut archive = archive_from_entries(&[("large.bin", b"eight by")]);

        assert!(validate_zip_with_limits(&mut archive, MAX_DEPLOYMENT_FILES, 4).is_err());
    }

    #[test]
    fn test_zip_accepts_size_at_limit() {
        let mut archive = archive_from_entries(&[("exact.bin", b"1234")]);

        assert!(validate_zip_with_limits(&mut archive, MAX_DEPLOYMENT_FILES, 4).is_ok());
    }

    #[test]
    fn test_zip_rejects_too_many_files() {
        let mut archive =
            archive_from_entries(&[("a.txt", b"a"), ("b.txt", b"b"), ("c.txt", b"c")]);

        assert!(validate_zip_with_limits(&mut archive, 2, MAX_DEPLOYMENT_BYTES).is_err());
    }
}
