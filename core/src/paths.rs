use std::path::PathBuf;

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

pub fn runtime_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_dir())
}

pub fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/state"))
        .join("sigil")
}

pub fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".cache"))
        .join("sigil")
}

/// Where a kept file lands: `user-dirs.dirs` knows the real name, which is not always "Downloads".
pub fn download_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("XDG_DOWNLOAD_DIR").map(PathBuf::from).filter(|p| p.is_dir()) {
        return d;
    }
    let cfg = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"));
    if let Ok(text) = std::fs::read_to_string(cfg.join("user-dirs.dirs")) {
        for line in text.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("XDG_DOWNLOAD_DIR=") else { continue };
            let rest = rest.trim().trim_matches('"');
            let expanded = match rest.strip_prefix("$HOME/") {
                Some(tail) => home().join(tail),
                None => PathBuf::from(rest),
            };
            if expanded.is_dir() { return expanded }
        }
    }
    let fallback = home().join("Downloads");
    let _ = std::fs::create_dir_all(&fallback);
    fallback
}

/// Socket in $XDG_RUNTIME_DIR (0700 tmpfs), else the state dir. Never /tmp: another user could pre-create it.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("SIGIL_SOCKET") {
        return PathBuf::from(p);
    }
    match runtime_dir() {
        Some(d) => d.join("sigil.sock"),
        None => state_dir().join("sigil.sock"),
    }
}

/// Directory holding per-track video shared-memory files.
pub fn shm_dir() -> PathBuf {
    match runtime_dir() {
        Some(d) => d.join("sigil"),
        None => state_dir().join("shm"),
    }
}

pub fn ensure_private_dir(p: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    if !p.exists() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(p)?;
    }
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700))
}

/// Clamp the sqlite stores to 0700 dirs / 0600 files; matrix-sdk creates them with the umask.
pub fn tighten_store_permissions() {
    use std::os::unix::fs::PermissionsExt;
    for dir in [state_dir().join("store"), cache_dir().join("store")] {
        if !dir.exists() { continue }
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            if e.path().is_file() {
                let _ = std::fs::set_permissions(e.path(), std::fs::Permissions::from_mode(0o600));
            }
        }
    }
}
