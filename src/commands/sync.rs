use std::path::Path;
use super::setup::{COPY_FILES, default_chrome_profile_dir, setup_profile_dir};

/// Directories that store auth tokens (LevelDB format, must be copied entirely).
const COPY_DIRS: &[&str] = &[
    "Local Storage",
    "Session Storage",
];

/// Sync login state (Cookies, Login Data, localStorage, sessionStorage)
/// from main Chrome to debug profile.
/// Returns (synced_count, skipped_count).
pub fn sync_login_files() -> anyhow::Result<(usize, usize)> {
    let orig = default_chrome_profile_dir()
        .ok_or_else(|| anyhow::anyhow!("未找到 Chrome 默认 profile 目录"))?;
    let profile_dir = setup_profile_dir();
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

pub fn run() -> anyhow::Result<()> {
    println!("正在同步登录态（含 Local Storage / Session Storage）...");

    let (synced, skipped) = sync_login_files()?;

    println!(
        "同步完成: {} 项已更新, {} 项跳过",
        synced, skipped
    );

    if skipped > 0 {
        println!("提示: 跳过的项可能被 Chrome 锁定。如有需要，请关闭调试 Chrome 后重试。");
    }

    println!("\n如果调试 Chrome 正在运行，请重启以使新登录态生效。");
    Ok(())
}
