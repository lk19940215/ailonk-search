pub fn run() -> anyhow::Result<()> {
    let profile_dir = dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".ailonk-search-profile");

    if profile_dir.exists() {
        std::fs::remove_dir_all(&profile_dir)?;
        println!("已删除调试 profile: {}", profile_dir.display());
    } else {
        println!("调试 profile 不存在, 无需清理。");
    }

    println!();
    println!("恢复步骤:");
    println!("  1. 关闭所有通过命令行启动的 Chrome 窗口");
    println!("  2. 正常方式打开 Chrome (点击 Dock/桌面图标)");
    println!("  3. 你的书签/登录态/扩展会自动恢复 (它们始终保存在 Chrome 原始目录中)");
    println!();
    println!("说明: cleanup 只删除了调试用的 symlink 目录,");
    println!("你的原始 Chrome 数据从未被修改, 所以「恢复」就是正常打开 Chrome。");

    Ok(())
}
