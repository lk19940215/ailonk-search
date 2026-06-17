use crate::browser::manager::find_chrome_path;

fn default_chrome_profile_dir() -> Option<std::path::PathBuf> {
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

fn setup_profile_dir() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".ailonk-search-profile")
}

/// Files safe to symlink (no SQLite locking issues).
const SYMLINK_FILES: &[&str] = &[
    "Bookmarks", "Bookmarks.bak",
    "Preferences", "Secure Preferences",
];

/// SQLite databases — must be COPIED (symlink causes WAL lock conflicts
/// between normal Chrome and debug Chrome).
const COPY_FILES: &[&str] = &[
    "Cookies", "Cookies-journal",
    "Login Data", "Login Data-journal",
    "Login Data For Account", "Login Data For Account-journal",
    "Web Data", "Web Data-journal",
    "History", "History-journal",
    "Favicons", "Favicons-journal",
    "Top Sites", "Top Sites-journal",
    "Shortcuts", "Shortcuts-journal",
];

/// Directories safe to symlink (read-only or regeneratable).
const SYMLINK_DIRS: &[&str] = &[
    "Extensions", "Extension State", "Extension Rules",
    "Local Extension Settings", "Sync Extension Settings",
    "Managed Extension Settings",
];

fn run_setup_profile(orig: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
    let profile_dir = setup_profile_dir();

    if profile_dir.exists() {
        let bookmarks = profile_dir.join("Default/Bookmarks");
        if bookmarks.is_symlink() && bookmarks.exists() {
            println!("调试 profile 已存在且书签链接有效: {}", profile_dir.display());
            return Ok(profile_dir);
        }
        println!("调试 profile 存在但数据链接异常, 正在修复...");
        std::fs::remove_dir_all(&profile_dir)?;
    }

    std::fs::create_dir_all(profile_dir.join("Default"))?;

    let local_state_src = orig.join("Local State");
    if local_state_src.exists() {
        std::fs::copy(&local_state_src, profile_dir.join("Local State"))?;
        println!("  复制: Local State");
    }

    let orig_default = orig.join("Default");
    let dst_default = profile_dir.join("Default");

    for name in SYMLINK_FILES {
        let src = orig_default.join(name);
        if src.exists() {
            symlink_item(&src, &dst_default.join(name))?;
            println!("  链接: {}", name);
        }
    }

    for name in COPY_FILES {
        let src = orig_default.join(name);
        if src.exists() {
            std::fs::copy(&src, dst_default.join(name))?;
            println!("  复制: {}", name);
        }
    }

    for name in SYMLINK_DIRS {
        let src = orig_default.join(name);
        if src.is_dir() {
            symlink_item(&src, &dst_default.join(name))?;
            println!("  目录链接: {}", name);
        }
    }

    println!("\n书签/扩展通过 symlink 实时同步, Cookie/登录态通过复制保留。");
    println!("如需更新登录态, 请重新运行 setup。");
    Ok(profile_dir)
}

fn symlink_item(src: &std::path::Path, dst: &std::path::Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(src, dst)?;
    }
    #[cfg(windows)]
    {
        if src.is_dir() {
            std::os::windows::fs::symlink_dir(src, dst)?;
        } else {
            std::os::windows::fs::symlink_file(src, dst)?;
        }
    }
    Ok(())
}

fn print_setup_instructions(chrome_path: Option<&str>, profile_dir: &std::path::Path) {
    let dir = profile_dir.display();

    #[cfg(target_os = "macos")]
    {
        let path = chrome_path.unwrap_or(
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        );
        let escaped = path.replace(' ', "\\ ");
        println!(
            "\nChrome Search MCP — 一次性配置\n\n\
在终端运行以下命令启动调试 Chrome:\n\n\
  {escaped} --remote-debugging-port=19222 --user-data-dir=\"{dir}\" --no-first-run --no-default-browser-check &\n\n\
  无需关闭已打开的 Chrome — 调试实例使用独立目录, 可与正常 Chrome 并行运行。\n\
  书签/扩展通过 symlink 实时同步, Cookie/登录态通过复制保留。\n\n\
永久生效(可选):\n\
  添加以下 alias 到 ~/.zshrc 或 ~/.bashrc:\n\n\
  alias ailonk-debug='{path} --remote-debugging-port=19222 --user-data-dir=\"{dir}\" --no-first-run --no-default-browser-check &'\n\n\
  之后只需运行 ailonk-debug 即可。\n\n\
完成后, ailonk-search 会自动连接调试 Chrome (无需任何额外参数)。"
        );
    }

    #[cfg(target_os = "linux")]
    {
        let cmd = chrome_path.unwrap_or("google-chrome");
        println!(
            "\nChrome Search MCP — 一次性配置\n\n\
在终端运行以下命令启动调试 Chrome:\n\n\
  {cmd} --remote-debugging-port=19222 --user-data-dir=\"{dir}\" --no-first-run --no-default-browser-check &\n\n\
  无需关闭已打开的 Chrome — 调试实例使用独立目录, 可与正常 Chrome 并行运行。\n\n\
永久生效(可选):\n\
  添加 alias 到 ~/.bashrc:\n\n\
  alias ailonk-debug='{cmd} --remote-debugging-port=19222 --user-data-dir=\"{dir}\" --no-first-run --no-default-browser-check &'\n\n\
完成后, ailonk-search 会自动连接 (无需额外参数)。"
        );
    }

    #[cfg(target_os = "windows")]
    {
        let path = chrome_path
            .unwrap_or(r"C:\Program Files\Google\Chrome\Application\chrome.exe");
        println!(
            "\nChrome Search MCP — 一次性配置\n\n\
在命令行运行以下命令启动调试 Chrome:\n\n\
  \"{path}\" --remote-debugging-port=19222 --user-data-dir=\"{dir}\" --no-first-run --no-default-browser-check\n\n\
  无需关闭已打开的 Chrome — 调试实例使用独立目录, 可与正常 Chrome 并行运行。\n\n\
永久生效(可选):\n\
  右键 Chrome 快捷方式 → 属性 → 目标 后追加:\n\
  --remote-debugging-port=19222 --user-data-dir=\"{dir}\" --no-first-run --no-default-browser-check"
        );
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        println!(
            "\nChrome Search MCP — 一次性配置\n\n\
请启动 Chrome 并开启远程调试:\n\
  chrome --remote-debugging-port=19222 --user-data-dir=\"{dir}\""
        );
    }
}

pub fn run(args: &crate::cli::Args) -> anyhow::Result<()> {
    let chrome_path = find_chrome_path(&args.chrome_path);
    if let Some(ref path) = chrome_path {
        println!("检测到 Chrome: {}", path);
    } else {
        println!("未检测到 Chrome 安装路径, 使用默认路径。");
    }

    let orig = default_chrome_profile_dir()
        .ok_or_else(|| anyhow::anyhow!("未找到 Chrome 默认 profile 目录"))?;
    println!("Chrome 原始目录: {}", orig.display());

    let profile_dir = run_setup_profile(&orig)?;
    print_setup_instructions(chrome_path.as_deref(), &profile_dir);
    Ok(())
}
