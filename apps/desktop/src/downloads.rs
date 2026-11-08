//! Explicit, user-selected file exports. Renderer input never contains a path.
use base64::{Engine, prelude::BASE64_STANDARD};
use zeroize::Zeroizing;

const MAX_ENCODED_BYTES: usize = 40 * 1024 * 1024;
const MAX_FILE_BYTES: usize = 27 * 1024 * 1024;

pub(super) fn validate_input(filename: &str, encoded_len: usize) -> Result<(), String> {
    if filename.trim().is_empty()
        || filename.len() > 180
        || matches!(filename, "." | "..")
        || filename
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
    {
        return Err("Choose a valid file name".into());
    }
    if encoded_len > MAX_ENCODED_BYTES {
        return Err("Download is too large".into());
    }
    Ok(())
}

fn decode(data: &str) -> Result<Zeroizing<Vec<u8>>, String> {
    let bytes = Zeroizing::new(
        BASE64_STANDARD
            .decode(data)
            .map_err(|_| "Invalid download encoding".to_string())?,
    );
    if bytes.len() > MAX_FILE_BYTES {
        return Err("Download is too large".into());
    }
    Ok(bytes)
}

#[cfg(target_os = "macos")]
pub(super) fn save(filename: String, data: String) -> Result<bool, String> {
    use std::{fs::OpenOptions, io::Write, os::unix::fs::OpenOptionsExt};
    validate_input(&filename, data.len())?;
    let data = Zeroizing::new(data);
    let bytes = decode(&data)?;
    // RFD dispatches the native panel to AppKit's thread in this windowed app.
    // The OS asks before replacing an existing file. No path comes from IPC.
    let Some(destination) = rfd::FileDialog::new()
        .set_title("Save file")
        .set_file_name(filename)
        .save_file()
    else {
        return Ok(false);
    };
    if std::fs::symlink_metadata(&destination)
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err("Choose a regular file destination".into());
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(destination)
        .map_err(|_| "Unable to save this file".to_string())?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| "Unable to save this file".to_string())?;
    Ok(true)
}

#[cfg(not(target_os = "macos"))]
pub(super) fn save(_filename: String, _data: String) -> Result<bool, String> {
    Err("File export requires the supported macOS desktop build".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_paths_and_unbounded_inputs_without_opening_dialog() {
        for name in [
            "",
            " ",
            ".",
            "..",
            "../secret",
            "/tmp/file",
            "folder\\file",
            "file\nname",
            "file\0name",
            "disk:file",
            &"x".repeat(181),
        ] {
            assert!(validate_input(name, 0).is_err(), "{name:?}");
        }
        for name in [
            "photo.png",
            "cirra-history.crkbackup",
            "report final.pdf",
            "📎.txt",
        ] {
            assert!(validate_input(name, MAX_ENCODED_BYTES).is_ok());
        }
        assert!(validate_input("file.bin", MAX_ENCODED_BYTES + 1).is_err());
    }

    #[test]
    fn decoding_preserves_exact_binary_bytes_and_checks_decoded_limit() {
        let binary = [0, 1, 255, 0, 128];
        assert_eq!(&**decode(&BASE64_STANDARD.encode(binary)).unwrap(), &binary);
        assert!(decode("invalid!").is_err());
        let encoded = BASE64_STANDARD.encode(vec![0; MAX_FILE_BYTES + 1]);
        assert!(validate_input("file.bin", encoded.len()).is_ok());
        assert!(decode(&encoded).is_err());
    }
}
