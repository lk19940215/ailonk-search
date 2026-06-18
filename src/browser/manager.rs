use std::sync::Arc;
use eoka::{Browser, StealthConfig};

use super::pool::TabPool;

const DEBUG_PORT: u16 = 19222;

pub struct LazyBrowserManager {
    args: crate::cli::Args,
    inner: tokio::sync::OnceCell<Arc<BrowserManager>>,
}

impl LazyBrowserManager {
    pub fn new(args: &crate::cli::Args) -> Arc<Self> {
        Arc::new(Self {
            args: args.clone(),
            inner: tokio::sync::OnceCell::new(),
        })
    }

    pub async fn get(&self) -> anyhow::Result<&Arc<BrowserManager>> {
        self.inner
            .get_or_try_init(|| async {
                tracing::info!("First tool call — initializing Chrome...");
                let bm = BrowserManager::new(&self.args).await?;
                Ok(Arc::new(bm))
            })
            .await
    }

    pub async fn shutdown(&self) {
        if let Some(bm) = self.inner.get() {
            bm.shutdown().await;
        }
    }
}

pub enum ConnectionMode {
    UserChrome,
    Headless,
}

pub struct BrowserManager {
    #[allow(dead_code)]
    browser: Arc<Browser>,
    mode: ConnectionMode,
    tab_pool: TabPool,
    chrome_child: std::sync::Mutex<Option<std::process::Child>>,
}

impl BrowserManager {
    pub async fn new(args: &crate::cli::Args) -> anyhow::Result<Self> {
        if let Some(ref url) = args.remote_url {
            Self::connect_remote(url, args.max_tabs).await
        } else if args.headless {
            Self::launch_headless(args).await
        } else {
            Self::launch_user_chrome(args).await
        }
    }

    async fn connect_remote(url: &str, max_tabs: usize) -> anyhow::Result<Self> {
        let mut config = StealthConfig::live();
        config.cdp_timeout = 60;
        let browser = Browser::connect_with_config(url, config).await
            .map_err(|e| anyhow::anyhow!("Failed to connect to Chrome at {}: {}", url, e))?;
        tracing::info!("Connected to existing Chrome at {}", url);
        Ok(Self::from_browser(browser, ConnectionMode::UserChrome, max_tabs, None))
    }

    async fn launch_user_chrome(args: &crate::cli::Args) -> anyhow::Result<Self> {
        let profile_dir = dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(".ailonk-search-profile");

        if !profile_dir.exists() {
            tracing::warn!(
                "Profile 目录不存在: {}. 将使用 headless 模式。\n\
                运行 `ailonk-search setup` 创建 profile。",
                profile_dir.display()
            );
            return Self::launch_headless(args).await;
        }

        let chrome_path = find_chrome_path(&args.chrome_path)
            .ok_or_else(|| anyhow::anyhow!("未找到 Chrome，请使用 --chrome-path 指定"))?;

        let port = DEBUG_PORT;

        if let Ok(ws_url) = wait_for_debug_port(port, 1).await {
            tracing::info!("Detected existing Chrome on port {}, connecting...", port);
            let mut config = StealthConfig::live();
            config.cdp_timeout = 60;
            let browser = Browser::connect_with_config(&ws_url, config).await
                .map_err(|e| anyhow::anyhow!("Failed to connect to existing Chrome: {}", e))?;
            return Ok(Self::from_browser(browser, ConnectionMode::UserChrome, args.max_tabs, None));
        }

        let mut cmd = std::process::Command::new(&chrome_path);
        cmd.arg(format!("--remote-debugging-port={}", port))
            .arg(format!("--user-data-dir={}", profile_dir.display()))
            .arg("--profile-directory=Default")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-background-timer-throttling")
            .arg("--disable-backgrounding-occluded-windows")
            .arg("--disable-renderer-backgrounding")
            .arg("--disable-popup-blocking")
            .arg("--disable-hang-monitor")
            .arg("--disable-prompt-on-repost")
            .arg("--disable-sync")
            .arg("--disable-translate")
            .arg("--disable-infobars");

        if let Some(ref extra) = args.chrome_args {
            for a in extra.split(',') {
                let a = a.trim();
                if !a.is_empty() {
                    cmd.arg(a);
                }
            }
        }

        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        tracing::info!("Launching Chrome with user profile: {}", profile_dir.display());
        let child = cmd.spawn()
            .map_err(|e| anyhow::anyhow!("Failed to launch Chrome at {}: {}", chrome_path, e))?;

        let ws_url = wait_for_debug_port(port, 10).await
            .map_err(|e| anyhow::anyhow!("Chrome 启动超时 (port {}): {}", port, e))?;

        tracing::info!(ws_url = %ws_url, "Chrome ready, connecting via WebSocket...");

        let mut config = StealthConfig::live();
        config.cdp_timeout = 60;
        let browser = Browser::connect_with_config(ws_url.trim(), config).await
            .map_err(|e| anyhow::anyhow!("Failed to connect to Chrome (url={}): {}", ws_url.trim(), e))?;

        tracing::info!("Connected to user Chrome (with login state)");
        Ok(Self::from_browser(browser, ConnectionMode::UserChrome, args.max_tabs, Some(child)))
    }

    async fn launch_headless(args: &crate::cli::Args) -> anyhow::Result<Self> {
        let mut config = StealthConfig {
            headless: true,
            patch_binary: true,
            human_mouse: true,
            human_typing: true,
            cdp_timeout: 60,
            ..Default::default()
        };

        if let Some(ref path) = args.chrome_path {
            config.chrome_path = Some(path.clone());
        }

        let browser = Browser::launch_with_config(config).await
            .map_err(|e| anyhow::anyhow!("Failed to launch headless Chrome: {}", e))?;
        tracing::info!("Launched headless Chrome with stealth");
        Ok(Self::from_browser(browser, ConnectionMode::Headless, args.max_tabs, None))
    }

    fn from_browser(
        browser: Browser,
        mode: ConnectionMode,
        max_tabs: usize,
        chrome_child: Option<std::process::Child>,
    ) -> Self {
        let browser = Arc::new(browser);
        let tab_pool = TabPool::new(browser.clone(), max_tabs);
        tracing::info!("Tab pool ready (max {} tabs)", max_tabs);
        Self { browser, mode, tab_pool, chrome_child: std::sync::Mutex::new(chrome_child) }
    }

    pub fn mode(&self) -> &ConnectionMode {
        &self.mode
    }

    pub fn tab_pool(&self) -> &TabPool {
        &self.tab_pool
    }

    pub async fn shutdown(&self) {
        if let Some(mut child) = self.chrome_child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
            tracing::info!("Chrome process terminated");
        }

        match self.mode {
            ConnectionMode::UserChrome => {
                tracing::info!("UserChrome session ended");
            }
            ConnectionMode::Headless => {
                tracing::info!("Headless Chrome closed");
            }
        }
    }
}

pub fn find_chrome_path(cli_path: &Option<String>) -> Option<String> {
    if let Some(path) = cli_path {
        if std::path::Path::new(path).exists() {
            return Some(path.clone());
        }
    }

    #[cfg(target_os = "macos")]
    {
        let candidates = [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ];
        for path in &candidates {
            if std::path::Path::new(path).exists() {
                return Some(path.to_string());
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        let commands = [
            "google-chrome", "google-chrome-stable",
            "microsoft-edge", "microsoft-edge-stable",
            "chromium", "chromium-browser",
        ];
        for cmd in &commands {
            if let Ok(output) = std::process::Command::new("which").arg(cmd).output() {
                if output.status.success() {
                    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !path.is_empty() {
                        return Some(path);
                    }
                }
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        let paths = [
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        ];
        for path in &paths {
            if std::path::Path::new(path).exists() {
                return Some(path.to_string());
            }
        }
    }
    None
}

async fn wait_for_debug_port(port: u16, timeout_secs: u64) -> anyhow::Result<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);

    loop {
        if tokio::time::Instant::now() > deadline {
            anyhow::bail!("Chrome debug port {} not ready after {}s", port, timeout_secs);
        }

        match tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port)).await {
            Ok(mut stream) => {
                let req = format!(
                    "GET /json/version HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
                    port
                );
                if stream.write_all(req.as_bytes()).await.is_ok() {
                    let mut buf = Vec::with_capacity(4096);
                    let mut tmp = [0u8; 1024];
                    let read_result = tokio::time::timeout(
                        std::time::Duration::from_secs(3),
                        async {
                            loop {
                                let n = stream.read(&mut tmp).await?;
                                if n == 0 { break; }
                                buf.extend_from_slice(&tmp[..n]);
                                let text = String::from_utf8_lossy(&buf);
                                if text.contains("webSocketDebuggerUrl") {
                                    break;
                                }
                            }
                            Ok::<_, std::io::Error>(())
                        }
                    ).await;

                    if read_result.is_ok() {
                        let text = String::from_utf8_lossy(&buf);
                        if let Some(start) = text.find('{') {
                            let body = &text[start..];
                            if let Ok(info) = serde_json::from_str::<serde_json::Value>(body.trim()) {
                                if let Some(ws_url) = info["webSocketDebuggerUrl"].as_str() {
                                    return Ok(ws_url.to_string());
                                }
                            }
                        }
                    }
                }
            }
            Err(_) => {}
        }

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}
