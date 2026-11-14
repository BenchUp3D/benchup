//! `.bup` project files, shared by the app and the agent tools. A project is
//! JSON of a [`Document`], stored in one of three ways, detected by content:
//!
//! * a PNG viewport preview with the gzip-compressed JSON in a private `buPd`
//!   chunk before `IEND` (what the app saves when it has a GPU preview);
//! * gzip-compressed JSON (headless saves, the CLI);
//! * plain JSON (older files, recovery snapshots).
use benchup_core::{Document, tr};
use flate2::{Compression, read::MultiGzDecoder, write::GzEncoder};
use std::io::{Read, Write};

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
const DOCUMENT_CHUNK: &[u8; 4] = b"buPd";
/// Temporary JSON field the app uses to hand its preview image to [`encode`].
pub const PREVIEW_FIELD: &str = "viewport_preview_png";

/// Serialize `doc` as the app does when saving, then compress it.
pub fn encode_document(doc: &Document) -> Result<Vec<u8>, String> {
    let json = serde_json::to_string(doc).map_err(|error| error.to_string())?;
    encode(&json)
}

/// Gzip-compressed JSON; a PNG with the document chunk when the JSON carries a
/// preview image in [`PREVIEW_FIELD`] (the field itself is not stored).
pub fn encode(json: &str) -> Result<Vec<u8>, String> {
    let mut value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let preview = value.as_object_mut().and_then(|v| v.remove(PREVIEW_FIELD));
    let json = if preview.is_some() { serde_json::to_string(&value).map_err(|e| e.to_string())? } else { json.to_owned() };
    let compressed = gzip(&json)?;
    match preview {
        Some(preview) => {
            let png: Vec<u8> = serde_json::from_value(preview).map_err(|e| e.to_string())?;
            with_document_chunk(&png, &compressed)
        }
        None => Ok(compressed),
    }
}

/// Encode `doc` in the same container as `original`: a PNG keeps its preview
/// image (now stale) with the document chunk replaced; anything else becomes
/// gzip-compressed JSON. Lets a tool edit a file without dropping its preview.
pub fn reencode_like(original: &[u8], doc: &Document) -> Result<Vec<u8>, String> {
    let json = serde_json::to_string(doc).map_err(|error| error.to_string())?;
    let compressed = gzip(&json)?;
    if !original.starts_with(PNG_SIGNATURE) {
        return Ok(compressed);
    }
    let mut image = Vec::with_capacity(original.len());
    image.extend_from_slice(PNG_SIGNATURE);
    for chunk in chunks(original) {
        let chunk = chunk?;
        if &chunk[4..8] != DOCUMENT_CHUNK {
            image.extend_from_slice(chunk);
        }
    }
    with_document_chunk(&image, &compressed)
}

pub fn decode(bytes: &[u8]) -> Result<Document, String> {
    let bytes = project_payload(bytes)?;
    // Detect the content rather than the extension so all project extensions
    // accept compressed files and older, plain JSON files unchanged.
    let mut decompressed = Vec::new();
    let bytes: &[u8] = if bytes.starts_with(&[0x1f, 0x8b]) {
        MultiGzDecoder::new(bytes)
            .read_to_end(&mut decompressed)
            .map_err(|error| tr!("project_files.could_not_decompress_project", error = error))?;
        &decompressed
    } else {
        bytes
    };
    let json = std::str::from_utf8(bytes).map_err(|_| tr!("project_files.the_project_is_not_utf_8_text"))?;
    Document::from_json(json)
}

/// Whether `bytes` is a PNG preview file (it may still lack a project).
pub fn has_preview(bytes: &[u8]) -> bool {
    bytes.starts_with(PNG_SIGNATURE)
}

fn gzip(json: &str) -> Result<Vec<u8>, String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(json.as_bytes())
        .and_then(|_| encoder.finish())
        .map_err(|error| tr!("project_files.could_not_compress_project", error = error))
}

/// Insert the document as a private ancillary chunk immediately before IEND.
/// Standard PNG readers ignore it and display the saved viewport thumbnail.
fn with_document_chunk(png: &[u8], compressed: &[u8]) -> Result<Vec<u8>, String> {
    let end = png.len().checked_sub(12).ok_or(tr!("project_files.invalid_preview"))?;
    let mut chunk = Vec::with_capacity(compressed.len() + 12);
    chunk.extend_from_slice(&(u32::try_from(compressed.len()).map_err(|e| e.to_string())?).to_be_bytes());
    chunk.extend_from_slice(DOCUMENT_CHUNK);
    chunk.extend_from_slice(compressed);
    chunk.extend_from_slice(&png_crc(&chunk[4..]).to_be_bytes());
    let mut out = png.to_vec();
    out.splice(end..end, chunk);
    Ok(out)
}

/// The PNG's chunks (length, type, data and CRC), after the signature, up to
/// and including `IEND`. Bytes after `IEND` are not part of the image (tools
/// sometimes append some) and are ignored.
fn chunks(bytes: &[u8]) -> impl Iterator<Item = Result<&[u8], String>> {
    let mut position = PNG_SIGNATURE.len();
    let mut done = false;
    std::iter::from_fn(move || {
        if done {
            return None;
        }
        let header = bytes.get(position..position.saturating_add(8))?;
        let length = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
        let chunk = position
            .checked_add(12)
            .and_then(|v| v.checked_add(length))
            .ok_or_else(|| tr!("project_files.invalid_chunk_size").to_string())
            .and_then(|end| {
                let chunk = bytes.get(position..end).ok_or_else(|| tr!("project_files.truncated_chunk").to_string())?;
                position = end;
                Ok(chunk)
            });
        done = chunk.is_err() || &header[4..8] == b"IEND";
        Some(chunk)
    })
}

pub fn png_crc(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320u32 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

/// The stored document: the `buPd` chunk of a PNG, or the bytes themselves.
pub fn project_payload(bytes: &[u8]) -> Result<&[u8], String> {
    if !bytes.starts_with(PNG_SIGNATURE) {
        return Ok(bytes);
    }
    // Stops at the document chunk, so nothing after it needs to be readable.
    for chunk in chunks(bytes) {
        let chunk = chunk?;
        if &chunk[4..8] == DOCUMENT_CHUNK {
            let length = chunk.len() - 12;
            let checksum = u32::from_be_bytes(chunk[8 + length..].try_into().unwrap());
            if png_crc(&chunk[4..8 + length]) != checksum {
                return Err(tr!("project_files.invalid_checksum").into());
            }
            return Ok(&chunk[8..8 + length]);
        }
    }
    Err(tr!("project_files.png_has_no_project").into())
}

/// Write `bytes` to `path` through a temporary file next to the real file and
/// a rename, so a crash never leaves a half-written project. A symlink is
/// followed (the link stays, its target is replaced) and an existing file's
/// permissions are kept; a new file gets the usual ones (0o666 less the umask).
#[cfg(not(target_arch = "wasm32"))]
pub fn save_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    let existed = path.exists();
    let real = if existed { path.canonicalize().map_err(|e| e.to_string())? } else { path.to_path_buf() };
    let parent = real
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    if !existed {
        use std::os::unix::fs::PermissionsExt;
        // The temporary file is created with these, less the umask, as
        // `std::fs::write` would create the project.
        builder.permissions(std::fs::Permissions::from_mode(0o666));
    }
    let mut temporary = builder.tempfile_in(parent).map_err(|e| e.to_string())?;
    temporary
        .write_all(bytes)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    if existed {
        let permissions = std::fs::metadata(&real).map_err(|e| e.to_string())?.permissions();
        std::fs::set_permissions(temporary.path(), permissions).map_err(|e| e.to_string())?;
    }
    temporary.persist(&real).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    if let Ok(dir) = std::fs::File::open(parent) {
        let _ = dir.sync_all();
    }
    Ok(())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn documents_round_trip_compressed_and_plain() {
        let mut doc = Document::default();
        doc.name = "Bench".into();
        let bytes = encode_document(&doc).unwrap();
        assert!(bytes.starts_with(&[0x1f, 0x8b]));
        assert_eq!(decode(&bytes).unwrap().name, "Bench");
        let plain = serde_json::to_vec(&doc).unwrap();
        assert_eq!(decode(&plain).unwrap().name, "Bench");
    }

    /// A minimal valid PNG: signature, IHDR, IDAT and IEND.
    fn tiny_png() -> Vec<u8> {
        let chunk = |kind: &[u8; 4], data: &[u8]| -> Vec<u8> {
            let mut c = (data.len() as u32).to_be_bytes().to_vec();
            c.extend_from_slice(kind);
            c.extend_from_slice(data);
            let crc = png_crc(&c[4..]);
            c.extend_from_slice(&crc.to_be_bytes());
            c
        };
        let mut png = PNG_SIGNATURE.to_vec();
        png.extend(chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0]));
        png.extend(chunk(b"IDAT", &[120, 156, 99, 96, 0, 0, 0, 2, 0, 1]));
        png.extend(chunk(b"IEND", &[]));
        png
    }

    #[test]
    fn png_previews_carry_the_project_and_survive_an_edit() {
        let mut doc = Document::default();
        doc.name = "Bench".into();
        let mut value = serde_json::to_value(&doc).unwrap();
        value[PREVIEW_FIELD] = serde_json::to_value(tiny_png()).unwrap();
        let saved = encode(&value.to_string()).unwrap();
        assert!(has_preview(&saved));
        assert!(saved.ends_with(&tiny_png()[tiny_png().len() - 12..]), "IEND stays last");
        assert_eq!(decode(&saved).unwrap().name, "Bench");

        doc.name = "Edited".into();
        let edited = reencode_like(&saved, &doc).unwrap();
        assert!(has_preview(&edited), "the preview image is kept");
        assert_eq!(decode(&edited).unwrap().name, "Edited");
        assert_eq!(edited.windows(4).filter(|w| *w == DOCUMENT_CHUNK).count(), 1, "one document chunk");
        // Files without a preview stay gzip JSON.
        assert!(reencode_like(&encode_document(&doc).unwrap(), &doc).unwrap().starts_with(&[0x1f, 0x8b]));
        // A preview without a project is reported, not misread.
        assert!(decode(&tiny_png()).is_err());

        // Bytes appended after IEND are ignored when reading and when editing.
        let mut padded = edited.clone();
        padded.extend_from_slice(b"trailing metadata bytes");
        assert_eq!(decode(&padded).unwrap().name, "Edited");
        let reedited = reencode_like(&padded, &doc).unwrap();
        assert_eq!(decode(&reedited).unwrap().name, "Edited");
        assert!(reedited.ends_with(&tiny_png()[tiny_png().len() - 12..]), "IEND stays last");
    }

    #[test]
    fn corrupt_input_is_an_error_not_a_panic() {
        assert!(decode(&[0x1f, 0x8b, 1, 2, 3]).is_err());
        assert!(decode(&[0xff, 0xfe]).is_err());
        assert!(decode(b"{}").is_err());
    }

    #[test]
    fn save_atomic_replaces_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bup");
        save_atomic(&path, b"one").unwrap();
        save_atomic(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn save_atomic_follows_symlinks_and_keeps_mode() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.bup");
        std::fs::write(&real, b"old").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o640)).unwrap();
        let link = dir.path().join("link.bup");
        symlink(&real, &link).unwrap();
        save_atomic(&link, b"new").unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read(&real).unwrap(), b"new");
        assert_eq!(std::fs::metadata(&real).unwrap().permissions().mode() & 0o777, 0o640);
        // A new file gets the mode std::fs::write gives it under this umask.
        let fresh = dir.path().join("fresh.bup");
        save_atomic(&fresh, b"x").unwrap();
        let usual = dir.path().join("usual");
        std::fs::write(&usual, b"x").unwrap();
        let mode = |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&fresh), mode(&usual));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 4);
    }
}
