use std::{
    collections::HashSet,
    env, fs,
    fs::OpenOptions,
    io::{self, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

const STORAGE_VERSION: u32 = 1;
const MAX_CAMERAS: usize = 512;
const MAX_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedCamera {
    pub id: u64,
    pub name: String,
    pub rtsp_address: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CameraFile {
    version: u32,
    cameras: Vec<SavedCamera>,
}

pub fn camera_file_path() -> Result<PathBuf, String> {
    let base = if cfg!(windows) {
        env::var_os("APPDATA").map(PathBuf::from)
    } else {
        env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    }
    .ok_or_else(|| "Could not determine the user's configuration directory".to_string())?;

    Ok(base.join("vermouth-frontend").join("cameras.json"))
}

pub fn load() -> Result<Vec<SavedCamera>, String> {
    let path = camera_file_path()?;
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Cannot inspect saved cameras: {error}")),
    };
    if metadata.len() > MAX_FILE_BYTES {
        return Err("Saved camera file exceeds its size limit".into());
    }
    let bytes = fs::read(path).map_err(|error| format!("Cannot read saved cameras: {error}"))?;
    decode(&bytes)
}

pub fn save(cameras: &[SavedCamera]) -> Result<(), String> {
    validate(cameras)?;
    let path = camera_file_path()?;
    let parent = path.parent().ok_or("Invalid camera file path")?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Cannot create application data directory: {error}"))?;

    let data = serde_json::to_vec_pretty(&CameraFile {
        version: STORAGE_VERSION,
        cameras: cameras.to_vec(),
    })
    .map_err(|error| format!("Cannot serialize cameras: {error}"))?;
    if data.len() as u64 > MAX_FILE_BYTES {
        return Err("Saved camera file exceeds its size limit".into());
    }

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp = parent.join(format!(".cameras-{}-{nanos}.tmp", std::process::id()));
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|error| format!("Cannot create temporary camera file: {error}"))?;

    let write_result = (|| -> io::Result<()> {
        output.write_all(&data)?;
        output.sync_all()?;
        drop(output);
        fs::rename(&temp, &path)
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp);
        return Err(format!("Cannot commit saved cameras: {error}"));
    }
    Ok(())
}

fn decode(bytes: &[u8]) -> Result<Vec<SavedCamera>, String> {
    let document: CameraFile = serde_json::from_slice(bytes)
        .map_err(|error| format!("Invalid saved camera file: {error}"))?;
    if document.version != STORAGE_VERSION {
        return Err(format!(
            "Unsupported camera storage version: {}",
            document.version
        ));
    }
    validate(&document.cameras)?;
    Ok(document.cameras)
}

fn validate(cameras: &[SavedCamera]) -> Result<(), String> {
    if cameras.len() > MAX_CAMERAS {
        return Err("Too many saved cameras".into());
    }
    let mut ids = HashSet::new();
    for camera in cameras {
        if camera.id == 0 || !ids.insert(camera.id) {
            return Err("Camera IDs must be unique and nonzero".into());
        }
        if camera.name.trim().is_empty() || camera.name.len() > 128 {
            return Err("Camera names must be between 1 and 128 bytes".into());
        }
        validate_address(&camera.rtsp_address)?;
    }
    Ok(())
}

fn validate_address(address: &str) -> Result<(), String> {
    let rest = address
        .strip_prefix("rtsp://")
        .ok_or("Only rtsp:// camera addresses can currently be saved")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return Err(
            "RTSP URLs with embedded usernames/passwords cannot be saved until secure credential storage is implemented"
                .into(),
        );
    }
    if address.contains('?') || address.contains('#') {
        return Err("Token-bearing RTSP query/fragment URLs cannot yet be persisted safely".into());
    }
    if address.len() > 2048 || address.contains('\n') || address.contains('\r') {
        return Err("Invalid RTSP address".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> SavedCamera {
        SavedCamera {
            id: 4,
            name: "Front gate".into(),
            rtsp_address: "rtsp://192.168.1.20:554/live/ch00_1".into(),
        }
    }

    #[test]
    fn round_trip_preserves_only_configuration() {
        let file = CameraFile {
            version: STORAGE_VERSION,
            cameras: vec![camera()],
        };
        let encoded = serde_json::to_vec(&file).unwrap();
        let restored = decode(&encoded).unwrap();
        assert_eq!(restored, vec![camera()]);
        assert!(!String::from_utf8(encoded).unwrap().contains("connected"));
    }

    #[test]
    fn corrupt_file_does_not_become_an_empty_success() {
        assert!(decode(b"broken").is_err());
        assert!(decode(br#"{"version":99,"cameras":[]}"#).is_err());
    }

    #[test]
    fn rejects_duplicate_ids_and_embedded_secrets() {
        assert!(validate(&[camera(), camera()]).is_err());
        let mut secret = camera();
        secret.rtsp_address = "rtsp://admin:password@192.168.1.20/live".into();
        assert!(validate(&[secret]).is_err());
    }
}
