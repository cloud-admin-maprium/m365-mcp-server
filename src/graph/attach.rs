//! Local attachment handling — inline (base64) path only.
//!
//! The upstream JS also implements the `createUploadSession` chunked path for
//! 3-25MB combined. That is deliberately not ported yet: this build attaches up
//! to 3MB inline and returns a clear error above it.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};

use super::GraphError;

pub const MAX_DIRECT_ATTACH_BYTES: u64 = 3 * 1024 * 1024;
pub const MAX_UPLOAD_SESSION_BYTES: u64 = 25 * 1024 * 1024;

const MIME_TYPES: &[(&str, &str)] = &[
    (".pdf", "application/pdf"),
    (".doc", "application/msword"),
    (
        ".docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ),
    (".xls", "application/vnd.ms-excel"),
    (
        ".xlsx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ),
    (".ppt", "application/vnd.ms-powerpoint"),
    (
        ".pptx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ),
    (".png", "image/png"),
    (".jpg", "image/jpeg"),
    (".jpeg", "image/jpeg"),
    (".gif", "image/gif"),
    (".csv", "text/csv"),
    (".txt", "text/plain"),
    (".zip", "application/zip"),
];

#[derive(Debug, Clone)]
pub struct ResolvedFile {
    pub path: PathBuf,
    pub size: u64,
}

/// Canonicalise each path, then refuse anything outside the configured root.
/// `Path::starts_with` compares whole components, which is stricter — and safer —
/// than the upstream string-prefix check.
pub fn resolve(paths: &[String], root: &Path) -> Result<Vec<ResolvedFile>, GraphError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }

    let root = std::fs::canonicalize(root).map_err(|err| {
        GraphError::message(format!(
            "Attachment root '{}' is not usable: {err}",
            root.display()
        ))
    })?;

    paths
        .iter()
        .map(|requested| {
            let resolved = std::fs::canonicalize(requested).map_err(|_| {
                GraphError::message(format!("Attachment file not found: {requested}"))
            })?;
            if !resolved.starts_with(&root) {
                return Err(GraphError::message(format!(
                    "Attachment path '{requested}' is outside the shared data directory ({}) — refusing to read it.",
                    root.display()
                )));
            }
            let size = std::fs::metadata(&resolved)
                .map_err(|err| {
                    GraphError::message(format!(
                        "Attachment file '{}' is not readable: {err}",
                        resolved.display()
                    ))
                })?
                .len();
            Ok(ResolvedFile { path: resolved, size })
        })
        .collect()
}

pub fn total_bytes(files: &[ResolvedFile]) -> u64 {
    files.iter().map(|file| file.size).sum()
}

/// Rejects the combined size up front — the same combined-total rule as upstream
/// (`graphClient.js:12-19`), not a per-file check.
pub fn check_limits(files: &[ResolvedFile]) -> Result<(), GraphError> {
    let total = total_bytes(files);
    let mb = total as f64 / 1024.0 / 1024.0;
    if total > MAX_UPLOAD_SESSION_BYTES {
        return Err(GraphError::message(format!(
            "Combined attachment size is {mb:.1}MB — over the {}MB ceiling for this tool.",
            MAX_UPLOAD_SESSION_BYTES / 1024 / 1024
        )));
    }
    if total > MAX_DIRECT_ATTACH_BYTES {
        return Err(GraphError::message(format!(
            "Combined attachment size is {mb:.1}MB — this build attaches up to {}MB inline. \
             Chunked upload (3-25MB via createUploadSession) is not implemented yet.",
            MAX_DIRECT_ATTACH_BYTES / 1024 / 1024
        )));
    }
    Ok(())
}

pub async fn build_inline(files: &[ResolvedFile]) -> Result<Vec<Value>, GraphError> {
    let mut attachments = Vec::with_capacity(files.len());
    for file in files {
        let bytes = tokio::fs::read(&file.path).await.map_err(|err| {
            GraphError::message(format!(
                "Attachment file '{}' could not be read: {err}",
                file.path.display()
            ))
        })?;
        let name = file
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "attachment".to_string());
        attachments.push(json!({
            "@odata.type": "#microsoft.graph.fileAttachment",
            "name": name,
            "contentType": content_type_for(&file.path),
            "contentBytes": BASE64.encode(bytes),
        }));
    }
    Ok(attachments)
}

fn content_type_for(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .map(|ext| format!(".{}", ext.to_string_lossy().to_lowercase()))
        .unwrap_or_default();
    MIME_TYPES
        .iter()
        .find(|(known, _)| *known == extension)
        .map(|(_, mime)| *mime)
        .unwrap_or("application/octet-stream")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_lookup_falls_back_to_octet_stream() {
        assert_eq!(
            content_type_for(Path::new("/tmp/report.PDF")),
            "application/pdf"
        );
        assert_eq!(
            content_type_for(Path::new("/tmp/thing.unknown")),
            "application/octet-stream"
        );
        assert_eq!(
            content_type_for(Path::new("/tmp/noext")),
            "application/octet-stream"
        );
    }

    #[test]
    fn paths_outside_the_root_are_refused() {
        let root = std::env::temp_dir().join("m365-attach-root");
        std::fs::create_dir_all(&root).unwrap();
        let inside = root.join("ok.txt");
        std::fs::write(&inside, b"hi").unwrap();
        let outside = std::env::temp_dir().join("m365-attach-outside.txt");
        std::fs::write(&outside, b"no").unwrap();

        let good = resolve(&[inside.display().to_string()], &root).unwrap();
        assert_eq!(good.len(), 1);

        let err = resolve(&[outside.display().to_string()], &root).unwrap_err();
        assert!(
            err.to_string()
                .contains("outside the shared data directory")
        );
    }

    #[test]
    fn combined_size_drives_the_limit() {
        let one = ResolvedFile {
            path: PathBuf::from("/tmp/a"),
            size: 2 * 1024 * 1024,
        };
        let two = ResolvedFile {
            path: PathBuf::from("/tmp/b"),
            size: 2 * 1024 * 1024,
        };
        // Each file passes on its own; the combined total must not.
        assert!(check_limits(std::slice::from_ref(&one)).is_ok());
        assert!(check_limits(&[one, two]).is_err());
    }
}
