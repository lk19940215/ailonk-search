use std::path::Path;

/// SQLite databases that must be copied (symlink causes WAL lock conflicts).
pub const COPY_FILES: &[&str] = &[
    "Cookies", "Cookies-journal",
    "Login Data", "Login Data-journal",
    "Login Data For Account", "Login Data For Account-journal",
    "Web Data", "Web Data-journal",
    "History", "History-journal",
    "Favicons", "Favicons-journal",
    "Top Sites", "Top Sites-journal",
    "Shortcuts", "Shortcuts-journal",
];

/// Directories that store auth tokens (LevelDB format, must be copied entirely).
const COPY_DIRS: &[&str] = &[
    "Local Storage",
    "Session Storage",
];

pub fn default_chrome_profile_dir() -> Option<std::path::PathBuf> {
    let home = dirs::home_dir()?;
    #[cfg(target_os = "macos")]
    {
        let p = home.join("Library/Application Support/Google/Chrome");
        if p.exists() { return Some(p); }
    }
    #[cfg(target_os = "linux")]
    {
        let p = home.join(".config/google-chrome");
        if p.exists() { return Some(p); }
        let p = home.join(".config/chromium");
        if p.exists() { return Some(p); }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(local) = dirs::data_local_dir() {
            let p = local.join(r"Google\\Chrome\\User Data");
            if p.exists() { return Some(p); }
        }
    }
    None
}

pub fn debug_profile_dir() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".ailonk-search-profile")
}

/// Sync login state from main Chrome to debug profile.
/// Returns (synced_count, skipped_count).
pub fn sync_login_files() -> anyhow::Result<(usize, usize)> {
    let orig = default_chrome_profile_dir()
        .ok_or_else(|| anyhow::anyhow!("未找到 Chrome 默认 profile 目录"))?;
    let profile_dir = debug_profile_dir();
    let dst_default = profile_dir.join("Default");

    if !dst_default.exists() {
        anyhow::bail!(
            "调试 profile 不存在，请先运行 setup 命令: {}",
            profile_dir.display()
        );
    }

    let orig_default = orig.join("Default");
    let mut synced = 0;
    let mut skipped = 0;

    for name in COPY_FILES {
        let src = orig_default.join(name);
        let dst = dst_default.join(name);
        if src.exists() {
            match std::fs::copy(&src, &dst) {
                Ok(_) => { synced += 1; }
                Err(e) => {
                    tracing::warn!(file = name, error = %e, "file sync failed");
                    skipped += 1;
                }
            }
        }
    }

    for name in COPY_DIRS {
        let src = orig_default.join(name);
        let dst = dst_default.join(name);
        if src.is_dir() {
            match copy_dir_recursive(&src, &dst) {
                Ok(n) => {
                    tracing::debug!(dir = name, files = n, "dir synced");
                    synced += 1;
                }
                Err(e) => {
                    tracing::warn!(dir = name, error = %e, "dir sync failed");
                    skipped += 1;
                }
            }
        }
    }

    Ok((synced, skipped))
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<usize> {
    if dst.exists() {
        std::fs::remove_dir_all(dst)?;
    }
    std::fs::create_dir_all(dst)?;

    let mut count = 0;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            count += copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
            count += 1;
        }
    }
    Ok(count)
}
