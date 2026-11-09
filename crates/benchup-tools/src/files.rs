//! `.bup` project files: gzip-compressed JSON of a [`Document`]. Plain JSON
//! (older files, recovery snapshots) is accepted when reading.
use benchup_core::{Document, tr};
use flate2::{Compression, read::MultiGzDecoder, write::GzEncoder};
use std::io::{Read, Write};

/// Serialize `doc` as the app does when saving, then compress it.
pub fn encode_document(doc: &Document) -> Result<Vec<u8>, String> {
    let json = serde_json::to_string(doc).map_err(|error| error.to_string())?;
    encode(&json)
}

/// Project files use gzip-compressed JSON; recovery and save snapshots stay JSON.
pub fn encode(json: &str) -> Result<Vec<u8>, String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(json.as_bytes())
        .and_then(|_| encoder.finish())
        .map_err(|error| tr!("project_files.could_not_compress_project", error = error))
}

pub fn decode(bytes: &[u8]) -> Result<Document, String> {
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

/// Write `bytes` to `path` through a temporary file in the same directory and
/// a rename, so a crash never leaves a half-written project.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temporary
        .write_all(bytes)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    temporary.persist(path).map_err(|e| e.to_string())?;
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
}
