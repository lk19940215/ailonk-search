use crate::browser::profile;

pub use profile::sync_login_files;

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
