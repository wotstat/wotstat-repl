//! Local screenshot files shared by the game process and desktop MCP adapter.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tokio::time::Instant;

const FILE_PREFIX: &str = "wotstat-repl-";
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const STABLE_FOR: Duration = Duration::from_millis(250);
const STALE_AFTER: Duration = Duration::from_secs(5 * 60);
pub(crate) const MAX_SCREENSHOT_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct CapturedScreenshot {
    pub bytes: Vec<u8>,
    pub mime_type: &'static str,
}

/// Owns one capture prefix. Dropping it removes any matching temporary file.
pub(crate) struct PendingScreenshot {
    directory: PathBuf,
    extension: &'static str,
    mime_type: &'static str,
    prefix: String,
}

impl PendingScreenshot {
    pub fn new(game_dir: &Path, capture_id: &str, format: &str) -> Result<Self, String> {
        if capture_id.len() != 32
            || !capture_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err("invalid screenshot capture id".to_string());
        }
        let (extension, mime_type) = match format {
            "jpg" => ("jpg", "image/jpeg"),
            "png" => ("png", "image/png"),
            _ => return Err("format must be jpg or png".to_string()),
        };
        let directory = game_dir.join("screenshots");
        fs::create_dir_all(&directory).map_err(|error| {
            format!(
                "cannot prepare game screenshot directory {}: {error}",
                directory.display()
            )
        })?;
        cleanup_stale(&directory);
        let pending = Self {
            directory,
            extension,
            mime_type,
            prefix: format!("{FILE_PREFIX}{capture_id}"),
        };
        pending.cleanup_matches();
        Ok(pending)
    }

    pub async fn read(&self, deadline: Instant) -> Result<CapturedScreenshot, String> {
        let mut observed: Option<(PathBuf, u64, Instant)> = None;
        loop {
            if Instant::now() >= deadline {
                return Err("screenshot did not complete before its timeout".to_string());
            }
            match self.find_candidate().await? {
                Some(path) => {
                    let metadata = match tokio::fs::metadata(&path).await {
                        Ok(metadata) if metadata.is_file() => metadata,
                        Ok(_) => return Err("game screenshot path is not a file".to_string()),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            observed = None;
                            tokio::time::sleep(POLL_INTERVAL).await;
                            continue;
                        }
                        Err(error) => {
                            return Err(format!(
                                "cannot inspect game screenshot {}: {error}",
                                path.display()
                            ));
                        }
                    };
                    let size = metadata.len();
                    if size > MAX_SCREENSHOT_BYTES {
                        return Err(format!(
                            "screenshot is {size} bytes, above the {MAX_SCREENSHOT_BYTES}-byte limit"
                        ));
                    }
                    let now = Instant::now();
                    match &mut observed {
                        Some((observed_path, observed_size, observed_at))
                            if *observed_path == path && *observed_size == size =>
                        {
                            if size > 0 && now.duration_since(*observed_at) >= STABLE_FOR {
                                if let Some(bytes) = self.read_stable(&path, size).await? {
                                    if validate_image(&bytes, self.extension)? {
                                        return Ok(CapturedScreenshot {
                                            bytes,
                                            mime_type: self.mime_type,
                                        });
                                    }
                                }
                                *observed_at = now;
                            }
                        }
                        _ => observed = Some((path, size, now)),
                    }
                }
                None => observed = None,
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    async fn find_candidate(&self) -> Result<Option<PathBuf>, String> {
        let mut entries = tokio::fs::read_dir(&self.directory)
            .await
            .map_err(|error| {
                format!(
                    "cannot inspect game screenshot directory {}: {error}",
                    self.directory.display()
                )
            })?;
        let suffix = format!(".{}", self.extension);
        let mut matches = Vec::new();
        while let Some(entry) = entries.next_entry().await.map_err(|error| {
            format!(
                "cannot inspect game screenshot directory {}: {error}",
                self.directory.display()
            )
        })? {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if is_capture_name(name, &self.prefix, &suffix) {
                let file_type = entry
                    .file_type()
                    .await
                    .map_err(|error| format!("cannot inspect screenshot {name}: {error}"))?;
                if file_type.is_file() {
                    matches.push(entry.path());
                }
            }
        }
        matches.sort();
        Ok(matches.pop())
    }

    async fn read_stable(
        &self,
        path: &Path,
        expected_size: u64,
    ) -> Result<Option<Vec<u8>>, String> {
        let bytes = match tokio::fs::read(path).await {
            Ok(bytes) => bytes,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                ) =>
            {
                return Ok(None);
            }
            Err(error) => {
                return Err(format!(
                    "cannot read game screenshot {}: {error}",
                    path.display()
                ));
            }
        };
        let final_size = match tokio::fs::metadata(path).await {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "cannot verify game screenshot {}: {error}",
                    path.display()
                ));
            }
        };
        if final_size != expected_size || bytes.len() as u64 != expected_size {
            return Ok(None);
        }
        Ok(Some(bytes))
    }

    fn cleanup_matches(&self) {
        let suffix = format!(".{}", self.extension);
        let Ok(entries) = fs::read_dir(&self.directory) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if is_capture_name(name, &self.prefix, &suffix) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

impl Drop for PendingScreenshot {
    fn drop(&mut self) {
        self.cleanup_matches();
    }
}

fn is_capture_name(name: &str, prefix: &str, suffix: &str) -> bool {
    name.ends_with(suffix)
        && (name.strip_suffix(suffix) == Some(prefix)
            || name
                .strip_suffix(suffix)
                .is_some_and(|stem| stem.starts_with(&format!("{prefix}_"))))
}

fn cleanup_stale(directory: &Path) {
    let now = SystemTime::now();
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(FILE_PREFIX) || !(name.ends_with(".jpg") || name.ends_with(".png")) {
            continue;
        }
        let stale = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= STALE_AFTER);
        if stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Returns false while a correctly-started image is still missing its end marker.
fn validate_image(bytes: &[u8], extension: &str) -> Result<bool, String> {
    match extension {
        "jpg" if bytes.starts_with(&[0xff, 0xd8, 0xff]) => Ok(bytes.ends_with(&[0xff, 0xd9])),
        "png" if bytes.starts_with(b"\x89PNG\r\n\x1a\n") => {
            Ok(bytes.ends_with(b"\x00\x00\x00\x00IEND\xaeB\x60\x82"))
        }
        "jpg" | "png" => Err(format!("game returned invalid {extension} screenshot data")),
        _ => Err("unsupported screenshot format".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_game() -> PathBuf {
        std::env::temp_dir().join(format!("wms_screenshot_{}", uuid::Uuid::new_v4()))
    }

    #[tokio::test]
    async fn waits_for_a_growing_file_and_removes_it_after_reading() {
        let game = temp_game();
        let capture_id = "0123456789abcdef0123456789abcdef";
        let pending = PendingScreenshot::new(&game, capture_id, "jpg").unwrap();
        let path = game
            .join("screenshots")
            .join(format!("{FILE_PREFIX}{capture_id}_001.jpg"));
        let writer_path = path.clone();
        tokio::spawn(async move {
            fs::write(&writer_path, [0xff, 0xd8, 0xff, 0xe0]).unwrap();
            // Longer than STABLE_FOR: the missing JPEG end marker must keep the
            // reader waiting even while the file size is temporarily stable.
            tokio::time::sleep(Duration::from_millis(350)).await;
            let mut file = fs::OpenOptions::new()
                .append(true)
                .open(&writer_path)
                .unwrap();
            file.write_all(b"complete-jpeg\xff\xd9").unwrap();
        });

        let started = Instant::now();
        let capture = pending
            .read(Instant::now() + Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(capture.mime_type, "image/jpeg");
        assert_eq!(capture.bytes, b"\xff\xd8\xff\xe0complete-jpeg\xff\xd9");
        assert!(started.elapsed() >= Duration::from_millis(600));
        drop(pending);
        assert!(!path.exists());
        let _ = fs::remove_dir_all(game);
    }

    #[tokio::test]
    async fn rejects_a_file_with_the_wrong_image_signature() {
        let game = temp_game();
        let capture_id = "fedcba9876543210fedcba9876543210";
        let pending = PendingScreenshot::new(&game, capture_id, "png").unwrap();
        let path = game
            .join("screenshots")
            .join(format!("{FILE_PREFIX}{capture_id}_001.png"));
        fs::write(&path, b"not-a-png").unwrap();

        let error = pending
            .read(Instant::now() + Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(error.contains("invalid png"));
        drop(pending);
        assert!(!path.exists());
        let _ = fs::remove_dir_all(game);
    }
}
