//! Local sensitive storage policy (PRIVATEFILES-1..3).
//!
//! `~/.bourso/settings.json` can contain a plaintext password: do not share it.
//! No password is collected or persisted automatically by these helpers.
//! Managed directories are 0700 and managed regular files are 0600 on Unix,
//! including existing entries (permissions are tightened before use). Symlinks,
//! special files and Unix multiply-linked files are refused, never overwritten.
//! Explicit `save_settings` replaces settings content; loading creates defaults
//! only when absent, never on permission/parse errors. User-supplied credentials
//! loaded via `Settings::load` are read-only; their permissions are not changed.
//!
//! Logs remain sensitive despite diagnostic filtering. `bourso.log` and its sole
//! archive `bourso.log.1` are each limited to 10 MiB (20 MiB total). Rotation is
//! checked on every write, not just startup; the previous archive alone is replaced.
//! Oversized individual records and pre-existing oversized logs are refused rather
//! than silently truncated. Settings and unrelated files are never retention targets.
//! A persistent empty `bourso.log.lock` serializes logger processes: a second active
//! logger returns an error. This avoids stale descriptors writing into old archives.
//! Remove logs manually when no longer needed; settings have no automatic expiry.
//!
//! On Windows, Unix modes do not establish ACL protection: the user's profile and
//! inherited ACLs must restrict access. ACLs are neither configured nor tested here.
//! BOURSO_DATA_DIR optionally selects an absolute managed directory instead of
//! ~/.bourso; the same private-storage policy applies. The selected directory
//! and its ancestors must be trusted; this is not a sandbox
//! against a malicious process running as the same user and racing path operations.

use anyhow::{Context, Result};
use directories::UserDirs;
use serde::{Deserialize, Serialize};
use std::io::prelude::*;
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

// Resolve before consulting UserDirs: HOME does not override Windows Known Folders.
// The existing managed-directory/file routines validate and protect the result.
fn storage_directory() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("BOURSO_DATA_DIR") {
        return validate_storage_override(Path::new(&path));
    }
    let user_dirs = UserDirs::new().context("Failed to get user directories")?;
    Ok(user_dirs.home_dir().join(".bourso"))
}

// Pure validation keeps path boundary tests independent of process-global env.
fn validate_storage_override(path: &Path) -> Result<PathBuf> {
    anyhow::ensure!(
        !path.as_os_str().is_empty()
            && path.is_absolute()
            && !path.as_os_str().as_encoded_bytes().contains(&0),
        "BOURSO_DATA_DIR must be a nonempty absolute directory path"
    );
    Ok(path.to_owned())
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Settings {
    #[serde(rename = "clientId")]
    pub customer_id: Option<String>,
    #[serde(rename = "password")]
    pub password: Option<String>,
}

#[cfg(not(tarpaulin_include))]
impl Settings {
    pub fn load(path: &str) -> Result<Settings> {
        let mut file = open_existing_regular(Path::new(path), false)
            .context("Failed to read settings file")?;
        let mut file_content = String::new();
        file.read_to_string(&mut file_content)
            .context("Failed to read settings file")?;

        let settings: Settings = serde_json::from_str(&file_content).map_err(|e| {
            anyhow::anyhow!(
                "Failed to deserialize settings: {:?} at line {}, column {}. Please make sure the settings file is valid.",
                e.classify(), e.line(), e.column()
            )
        })?;

        Ok(settings)
    }
}

#[cfg(not(tarpaulin_include))]
pub fn get_settings() -> Result<Settings> {
    load_settings_at(&storage_directory()?)
}

#[cfg(not(tarpaulin_include))]
fn load_settings_at(directory: &Path) -> Result<Settings> {
    private_directory(directory)?;
    let path = directory.join("settings.json");
    let mut file = match open_existing_regular(&path, true) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let settings = Settings {
                customer_id: None,
                password: None,
            };
            // create_new refuses an entry appearing after the absence check.
            let mut file = create_private_file(&path)?;
            file.write_all(serde_json::to_string_pretty(&settings)?.as_bytes())?;
            return Ok(settings);
        }
        Err(error) => return Err(error).context("Failed to read settings file"),
    };
    let mut content = String::new();
    file.read_to_string(&mut content)
        .context("Failed to read settings file")?;
    serde_json::from_str(&content).map_err(|e| anyhow::anyhow!(
        "Failed to deserialize settings: {:?} at line {}, column {}. Please make sure the settings file is valid.",
        e.classify(), e.line(), e.column()
    ))
}

/// Explicitly replace managed settings, creating a private file if absent.
#[cfg(not(tarpaulin_include))]
pub fn save_settings(settings: &Settings) -> Result<()> {
    save_settings_at(&storage_directory()?, settings)
}

#[cfg(not(tarpaulin_include))]
fn save_settings_at(directory: &Path, settings: &Settings) -> Result<()> {
    let json = serde_json::to_string_pretty(settings).context("Failed to serialize settings")?;
    private_directory(directory)?;
    let mut file = open_private_file(&directory.join("settings.json"))?;
    // Do not truncate until the file type, identity and permissions were checked.
    file.set_len(0)
        .context("Failed to truncate settings file")?;
    file.write_all(json.as_bytes())
        .context("Failed to write settings file")?;
    Ok(())
}

fn unsafe_entry() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "Refusing non-regular, linked or changed private storage entry",
    )
}

fn private_directory(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => (),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error),
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(unsafe_entry());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn regular_metadata(path: &Path) -> io::Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(unsafe_entry());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(unsafe_entry());
        }
    }
    Ok(metadata)
}

fn verify_file(path: &Path, file: &fs::File) -> io::Result<()> {
    let metadata = regular_metadata(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() {
        return Err(unsafe_entry());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.dev() != opened.dev() || metadata.ino() != opened.ino() || opened.nlink() != 1 {
            return Err(unsafe_entry());
        }
    }
    #[cfg(not(unix))]
    let _ = metadata;
    Ok(())
}

fn private_mode(file: &fs::File) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}

fn create_private_file(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    // fchmod also restores owner access under restrictive umasks such as 0777.
    private_mode(&file)?;
    Ok(file)
}

fn open_existing_regular(path: &Path, managed: bool) -> io::Result<fs::File> {
    regular_metadata(path)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(managed)
        .open(path)?;
    verify_file(path, &file)?;
    if managed {
        private_mode(&file)?;
    }
    Ok(file)
}

fn open_private_file(path: &Path) -> io::Result<fs::File> {
    match open_existing_regular(path, true) {
        Ok(file) => Ok(file),
        Err(error) if error.kind() == io::ErrorKind::NotFound => create_private_file(path),
        Err(error) => Err(error),
    }
}

// Conservative, fixed retention: one active file and one backup, never a glob.
const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;

struct RotatingLog {
    directory: std::path::PathBuf,
    file: Option<fs::File>,
    // Keep the stable lock inode open across rotations (std file locking, Rust 1.89+).
    _lock: fs::File,
}

impl RotatingLog {
    fn open(directory: &Path) -> io::Result<Self> {
        private_directory(directory)?;
        let lock = open_private_file(&directory.join("bourso.log.lock"))?;
        lock.try_lock()
            .map_err(|_| io::Error::other("Log storage is already locked or cannot be locked"))?;
        Self::check_archive(directory)?;
        let mut file = open_private_file(&directory.join("bourso.log"))?;
        Self::check_size(&file)?;
        file.seek(io::SeekFrom::End(0))?;
        Ok(Self {
            directory: directory.to_owned(),
            file: Some(file),
            _lock: lock,
        })
    }

    fn check_size(file: &fs::File) -> io::Result<()> {
        if file.metadata()?.len() > MAX_LOG_BYTES {
            return Err(io::Error::other(
                "Existing log exceeds 10 MiB; move it aside manually before logging",
            ));
        }
        Ok(())
    }

    fn check_archive(directory: &Path) -> io::Result<()> {
        match open_existing_regular(&directory.join("bourso.log.1"), true) {
            Ok(file) => Self::check_size(&file),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn append(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() as u64 > MAX_LOG_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Log record exceeds 10 MiB",
            ));
        }
        let path = self.directory.join("bourso.log");
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("Log unavailable after rotation failure"))?;
        verify_file(&path, file)?;
        if file.metadata()?.len().saturating_add(bytes.len() as u64) > MAX_LOG_BYTES {
            Self::check_archive(&self.directory)?;
            file.flush()?;
            fs::rename(&path, self.directory.join("bourso.log.1"))?;
            // Never keep writing through a descriptor now referring to the archive.
            self.file = None;
            self.file = Some(create_private_file(&path)?);
        }
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("Log unavailable"))?;
        file.write_all(bytes)?;
        Ok(bytes.len())
    }
}

#[derive(Clone)]
struct LogWriter(Arc<Mutex<RotatingLog>>);

impl Write for LogWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("Log lock poisoned"))?
            .append(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut log = self
            .0
            .lock()
            .map_err(|_| io::Error::other("Log lock poisoned"))?;
        log.file
            .as_mut()
            .ok_or_else(|| io::Error::other("Log unavailable"))?
            .flush()
    }
}

pub fn init_logger() -> Result<()> {
    use std::io;
    use std::io::IsTerminal;
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let log =
        RotatingLog::open(&storage_directory()?).context("Failed to open private log storage")?;
    let writer = LogWriter(Arc::new(Mutex::new(log)));

    // Pretty console (stderr), filtered by RUST_LOG
    let console_layer = fmt::layer()
        .with_writer(io::stderr)
        .with_ansi(IsTerminal::is_terminal(&io::stderr()))
        .with_level(true)
        .with_target(true)
        .without_time()
        .compact()
        .fmt_fields({
            fmt::format::debug_fn(move |writer, field, value| {
                if field.name() == "message" {
                    write!(writer, "{:?}", value)?;
                }
                Ok(())
            })
        })
        .with_filter(env_filter.clone())
        .with_filter(tracing_subscriber::filter::filter_fn(
            application_diagnostic,
        ));

    // JSON file retains reviewed application diagnostics at TRACE, independently
    // of RUST_LOG. The shared writer owns the file and rotates before each write.
    let json_layer = fmt::layer()
        .with_writer(move || writer.clone())
        .json()
        .with_target(true)
        .with_level(true)
        .flatten_event(true)
        .with_filter(LevelFilter::TRACE)
        .with_filter(tracing_subscriber::filter::filter_fn(
            application_diagnostic,
        ));

    tracing_subscriber::registry()
        .with(console_layer)
        .with(json_layer)
        .try_init()
        .context("Failed to install logger")?;

    Ok(())
}

/// Dependency diagnostics may contain raw cookies, headers and challenge URLs.
/// Permit only reviewed application targets on both persistent and console sinks.
fn application_diagnostic(metadata: &tracing::Metadata<'_>) -> bool {
    let target = metadata.target();
    ["bourso_cli", "bourso_api"].iter().any(|root| {
        target == *root
            || target
                .strip_prefix(root)
                .is_some_and(|suffix| suffix.starts_with("::"))
    })
}

#[cfg(test)]
mod private_storage_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Temp(std::path::PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "bourso-settings-unit-{}-{unique}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn directory(&self) -> std::path::PathBuf {
            self.0.join("private")
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn storage_override_requires_a_nonempty_absolute_path() {
        for invalid in ["", ".", "..", "relative", "~/private", "$HOME/private"] {
            assert!(
                validate_storage_override(Path::new(invalid)).is_err(),
                "{invalid:?}"
            );
        }
        let temp = Temp::new();
        let absolute = temp.0.join("private storage");
        assert_eq!(validate_storage_override(&absolute).unwrap(), absolute);
        assert!(
            !absolute.exists(),
            "pure validation must not create storage"
        );
        assert!(validate_storage_override(&temp.0.join("invalid\0path")).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn storage_override_rejects_windows_drive_relative_and_root_relative_paths() {
        for invalid in [r"C:private", r"\private"] {
            assert!(validate_storage_override(Path::new(invalid)).is_err());
        }
        assert!(validate_storage_override(Path::new(r"C:\private")).is_ok());
    }

    #[test]
    fn log_boundary_and_oversized_record_are_lossless_or_explicit_error() {
        let temp = Temp::new();
        let directory = temp.directory();
        let mut log = RotatingLog::open(&directory).unwrap();
        let exact = vec![b'a'; MAX_LOG_BYTES as usize];
        assert_eq!(log.append(&exact).unwrap(), exact.len());
        assert!(!directory.join("bourso.log.1").exists());
        assert!(log.append(&vec![b'b'; MAX_LOG_BYTES as usize + 1]).is_err());
        assert_eq!(fs::read(directory.join("bourso.log")).unwrap(), exact);
        log.append(b"next").unwrap();
        assert_eq!(fs::read(directory.join("bourso.log.1")).unwrap(), exact);
        assert_eq!(fs::read(directory.join("bourso.log")).unwrap(), b"next");
        log.append(&exact).unwrap();
        assert_eq!(fs::read(directory.join("bourso.log.1")).unwrap(), b"next");
        assert_eq!(fs::read(directory.join("bourso.log")).unwrap(), exact);
    }

    #[test]
    fn log_lock_prevents_stale_descriptor_writers_and_releases_on_drop() {
        let temp = Temp::new();
        let directory = temp.directory();
        let log = RotatingLog::open(&directory).unwrap();
        assert!(RotatingLog::open(&directory).is_err());
        drop(log);
        assert!(RotatingLog::open(&directory).is_ok());
    }

    #[test]
    fn rotation_error_keeps_current_file_and_does_not_touch_user_directory() {
        let temp = Temp::new();
        let directory = temp.directory();
        let mut log = RotatingLog::open(&directory).unwrap();
        log.file.as_ref().unwrap().set_len(MAX_LOG_BYTES).unwrap();
        let backup = directory.join("bourso.log.1");
        fs::create_dir(&backup).unwrap();
        fs::write(backup.join("userdata"), b"keep").unwrap();
        assert!(log.append(b"cannot rotate").is_err());
        assert_eq!(
            fs::metadata(directory.join("bourso.log")).unwrap().len(),
            MAX_LOG_BYTES
        );
        assert_eq!(fs::read(backup.join("userdata")).unwrap(), b"keep");
    }

    #[cfg(not(tarpaulin_include))]
    #[test]
    fn missing_settings_are_created_once_and_explicit_saves_can_update() {
        let temp = Temp::new();
        let directory = temp.directory();
        let initial = load_settings_at(&directory).unwrap();
        assert!(initial.customer_id.is_none() && initial.password.is_none());
        save_settings_at(
            &directory,
            &Settings {
                customer_id: Some("local".into()),
                password: None,
            },
        )
        .unwrap();
        assert_eq!(
            load_settings_at(&directory).unwrap().customer_id.as_deref(),
            Some("local")
        );
        save_settings_at(&directory, &initial).unwrap();
        assert!(load_settings_at(&directory).unwrap().customer_id.is_none());
    }
}
