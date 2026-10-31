//! Лок-файл: не даём запустить второй экземпляр трея.
//!
//! Файл лежит в `$XDG_RUNTIME_DIR` и содержит PID. Если процесс с таким PID
//! уже не существует (например, после `kill -9`), файл считается устаревшим
//! и перезаписывается.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct InstanceLock {
    path: PathBuf,
}

impl InstanceLock {
    /// Занимает лок в стандартном месте:
    /// `$XDG_RUNTIME_DIR/cr-tray.lock`, иначе `$TMPDIR/cr-tray-<uid>.lock`.
    pub fn acquire() -> Result<Self, String> {
        Self::acquire_at(default_path())
    }

    /// Занимает лок по указанному пути (для тестов и отладки).
    pub fn acquire_at(path: PathBuf) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }

        match try_create(&path) {
            Ok(()) => Ok(Self { path }),
            Err(TryCreate::Busy(pid)) => Err(format!(
                "cr-tray уже запущен (pid {pid}), второй экземпляр не нужен"
            )),
            Err(TryCreate::Other(e)) => Err(format!("{}: {e}", path.display())),
        }
    }

    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

enum TryCreate {
    Busy(String),
    Other(String),
}

fn try_create(path: &Path) -> Result<(), TryCreate> {
    for _ in 0..2 {
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(mut file) => {
                let pid = std::process::id().to_string();
                return file
                    .write_all(pid.as_bytes())
                    .map_err(|e| TryCreate::Other(e.to_string()));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let pid = fs::read_to_string(path).unwrap_or_default();
                let pid = pid.trim().to_string();
                if !pid.is_empty() && process_alive(&pid) {
                    return Err(TryCreate::Busy(pid));
                }
                // Устаревший лок: процесс уже умер, забираем файл себе.
                let _ = fs::remove_file(path);
            }
            Err(e) => return Err(TryCreate::Other(e.to_string())),
        }
    }
    Err(TryCreate::Other("не удалось занять лок-файл".into()))
}

fn process_alive(pid: &str) -> bool {
    pid.parse::<u32>().is_ok() && Path::new(&format!("/proc/{pid}")).exists()
}

fn default_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("cr-tray.lock");
    }
    let uid = std::env::var("UID").unwrap_or_else(|_| "user".into());
    std::env::temp_dir().join(format!("cr-tray-{uid}.lock"))
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cr-tray-lock-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("cr-tray.lock")
    }

    #[test]
    fn second_instance_is_rejected() {
        let path = temp_path("busy");
        let first = InstanceLock::acquire_at(path.clone()).unwrap();
        let err = InstanceLock::acquire_at(path.clone()).unwrap_err();
        assert!(err.contains("уже запущен"), "{err}");
        drop(first);
        // После освобождения лок снова свободен.
        InstanceLock::acquire_at(path.clone()).unwrap();
    }

    #[test]
    fn stale_lock_is_taken_over() {
        let path = temp_path("stale");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        // PID 0 точно не существует.
        fs::write(&path, "0").unwrap();
        let lock = InstanceLock::acquire_at(path.clone()).unwrap();
        assert_eq!(lock.path(), path);
    }

    #[test]
    fn empty_lock_file_is_taken_over() {
        let path = temp_path("empty");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "").unwrap();
        InstanceLock::acquire_at(path).unwrap();
    }

    #[test]
    fn lock_is_removed_on_drop() {
        let path = temp_path("drop");
        {
            let _lock = InstanceLock::acquire_at(path.clone()).unwrap();
            assert!(path.exists());
        }
        assert!(!path.exists());
    }
}