use std::sync::Arc;
use eoka::{Browser, Page, StealthConfig, TabInfo};
use tokio::sync::RwLock;

use super::pool::TabPool;

const DEBUG_PORT: u16 = 19222;

pub struct LazyBrowserManager {
    args: crate::cli::Args,
    inner: RwLock<Option<Arc<BrowserManager>>>,
    /// Cached WebSocket URL from a successful connection (AutoConnect or UserChrome),
    /// reused on reconnect to avoid re-scanning DevToolsActivePort and Chrome auth popups.
    cached_ws_url: RwLock<Option<String>>,
    /// Serializes reconnection attempts so only one task reconnects at a time.
    /// Unlike the RwLock on `inner`, this is only held during reconnection and
    /// does NOT block read access to the current BrowserManager.
    reconnect_mutex: tokio::sync::Mutex<()>,
}

impl LazyBrowserManager {
    pub fn new(args: &crate::cli::Args) -> Arc<Self> {
        Arc::new(Self {
            args: args.clone(),
            inner: RwLock::new(None),
            cached_ws_url: RwLock::new(None),
            reconnect_mutex: tokio::sync::Mutex::new(()),
        })
    }

    /// Reconnection timeout: bounds the total time `new_with_cache` can spend trying
    /// sequential connection methods. Set long enough for users to notice and respond to
    /// Chrome's CDP authorization popup. Fast rejection comes from eoka detecting
    /// TCP close (not from short timeouts). `spawn_blocking` prevents runtime starvation.
    const RECONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

    pub async fn get(&self) -> anyhow::Result<Arc<BrowserManager>> {
        // Fast path: return healthy cached connection (read lock only, no contention).
        {
            let guard = self.inner.read().await;
            if let Some(bm) = guard.as_ref() {
                if bm.is_healthy() {
                    return Ok(bm.clone());
                }
                tracing::warn!("Chrome connection unhealthy, will reconnect...");
            }
        }

        // Slow path: acquire reconnect_mutex to ensure only one task reconnects.
        // This is a tokio::Mutex (async), so waiting tasks yield properly and
        // browser()'s spawn-isolated timeout can fire even while we wait here.
        let _reconnect_guard = self.reconnect_mutex.lock().await;

        // Double-check: another task may have reconnected while we were waiting.
        {
            let guard = self.inner.read().await;
            if let Some(bm) = guard.as_ref() {
                if bm.is_healthy() {
                    return Ok(bm.clone());
                }
            }
        }

        // Take old BM out (brief write lock, released immediately).
        let old_bm = {
            let mut guard = self.inner.write().await;
            guard.take()
        };

        // Shutdown outside the write lock — even if eoka blocks, the write lock is free.
        if let Some(old) = old_bm {
            tracing::info!("Shutting down unhealthy Chrome instance...");
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(3),
                old.shutdown(),
            ).await;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }

        // Reconnect outside the write lock — eoka's blocking I/O won't hold any lock.
        tracing::info!("Initializing Chrome...");
        let cached_url = self.cached_ws_url.read().await.clone();
        let bm = tokio::time::timeout(
            Self::RECONNECT_TIMEOUT,
            BrowserManager::new_with_cache(&self.args, cached_url.as_deref()),
        )
        .await
        .map_err(|_| {
            tracing::error!("Chrome reconnection timed out ({}s)", Self::RECONNECT_TIMEOUT.as_secs());
            anyhow::anyhow!(
                "Chrome reconnection timed out after {}s. Chrome may be unresponsive or rejecting connections.",
                Self::RECONNECT_TIMEOUT.as_secs()
            )
        })??;

        // Store new BM (brief write lock).
        {
            let mut cache = self.cached_ws_url.write().await;
            if let Some(url) = &bm.connected_ws_url {
                *cache = Some(url.clone());
            } else if cached_url.is_some() {
                *cache = None;
                tracing::debug!("Cleared stale cached WS URL");
            }
        }

        let bm = Arc::new(bm);
        {
            let mut guard = self.inner.write().await;
            *guard = Some(bm.clone());
        }
        Ok(bm)
    }

    pub async fn shutdown(&self) {
        let mut guard = self.inner.write().await;
        if let Some(bm) = guard.take() {
            bm.shutdown().await;
        }
    }
}

pub enum ConnectionMode {
    UserChrome,
    Headless,
}

pub struct BrowserManager {
    browser: Arc<Browser>,
    mode: ConnectionMode,
    tab_pool: TabPool,
    chrome_child: std::sync::Mutex<Option<std::process::Child>>,
    healthy: Arc<std::sync::atomic::AtomicBool>,
    /// The WebSocket URL used for this connection (AutoConnect, UserChrome, or cached reconnect).
    pub(crate) connected_ws_url: Option<String>,
}

impl BrowserManager {
    /// Spawn-isolated WebSocket connect: runs eoka's blocking connect on a dedicated
    /// blocking thread pool (`spawn_blocking`) so leaked connections never starve
    /// the async runtime — rmcp can always write responses to stdout.
    async fn spawn_connect(ws_url: &str, config: StealthConfig, timeout_secs: u64) -> anyhow::Result<Browser> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let ws = ws_url.to_string();
        tokio::task::spawn_blocking(move || {
            let handle = tokio::runtime::Handle::current();
            let result = handle.block_on(Browser::connect_with_config(&ws, config));
            let _ = tx.send(result);
        });
        match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), rx).await {
            Ok(Ok(Ok(browser))) => Ok(browser),
            Ok(Ok(Err(e))) => Err(anyhow::anyhow!("WebSocket connect failed: {}", e)),
            Ok(Err(_)) => Err(anyhow::anyhow!("WebSocket connect task ended without result (panicked or dropped)")),
            Err(_) => {
                tracing::warn!(ws_url, timeout_secs, "WebSocket connect timed out (spawn-isolated)");
                Err(anyhow::anyhow!("WebSocket connection timed out after {}s — Chrome may be showing authorization popup", timeout_secs))
            }
        }
    }

    pub async fn new(args: &crate::cli::Args) -> anyhow::Result<Self> {
        Self::new_with_cache(args, None).await
    }

    pub async fn new_with_cache(args: &crate::cli::Args, cached_ws_url: Option<&str>) -> anyhow::Result<Self> {
        if let Some(ref url) = args.remote_url {
            Self::connect_remote(url, args.max_tabs).await
        } else if args.headless {
            Self::launch_headless(args).await
        } else {
            // On reconnect, try cached WS URL first to avoid triggering Chrome's auth popup
            if let Some(url) = cached_ws_url {
                if let Some(bm) = Self::try_cached_connect(url, args.max_tabs).await {
                    return Ok(bm);
                }
                tracing::debug!("Cached WS URL reconnect failed, falling back to full connect");
            }

            if args.no_auto_connect {
                tracing::info!("--no-auto-connect: skipping auto-connect, launching UserChrome");
                return Self::launch_user_chrome(args).await;
            }

            // If debug profile exists, probe port 19222 before scanning DevToolsActivePort
            if super::profile::debug_profile_dir().exists() {
                if let Some(bm) = Self::try_debug_port_connect(DEBUG_PORT, args.max_tabs).await {
                    return Ok(bm);
                }
            }

            if let Some(bm) = Self::try_auto_connect(args).await {
                return Ok(bm);
            }
            Self::launch_user_chrome(args).await
        }
    }

    /// Reconnect using a previously successful WebSocket URL (skips DevToolsActivePort scan).
    async fn try_cached_connect(ws_url: &str, max_tabs: usize) -> Option<Self> {
        tracing::info!("Attempting reconnect via cached WS URL...");

        let mut config = StealthConfig::live();
        config.cdp_timeout = 5;

        match Self::spawn_connect(ws_url, config, 5).await {
            Ok(browser) => {
                tracing::info!("Reconnected via cached WS URL");
                Some(Self::from_browser(browser, ConnectionMode::UserChrome, max_tabs, None, Some(ws_url.to_string())))
            }
            Err(e) => {
                tracing::debug!("Cached WS URL reconnect failed: {}", e);
                None
            }
        }
    }

    /// Fast probe: try connecting to an existing debug Chrome on a known port.
    async fn try_debug_port_connect(port: u16, max_tabs: usize) -> Option<Self> {
        let ws_url = wait_for_debug_port(port, 1).await.ok()?;
        let mut config = StealthConfig::live();
        config.cdp_timeout = 5;
        let browser = Self::spawn_connect(&ws_url, config, 15).await.ok()?;
        tracing::info!(port, "Connected via debug port (fast probe)");
        Some(Self::from_browser(browser, ConnectionMode::UserChrome, max_tabs, None, Some(ws_url)))
    }

    /// Try connecting via `chrome://inspect#remote-debugging` (Chrome/Edge 144+).
    /// Reads `DevToolsActivePort` from the browser's default user data directory.
    async fn try_auto_connect(args: &crate::cli::Args) -> Option<Self> {
        for (name, dir) in Self::browser_data_dirs() {
            let port_file = dir.join("DevToolsActivePort");
            if !port_file.exists() {
                continue;
            }

            let content = std::fs::read_to_string(&port_file).ok()?;
            let mut lines = content.lines();
            let port: u16 = match lines.next().and_then(|l| l.trim().parse().ok()) {
                Some(p) if p > 0 => p,
                _ => continue,
            };
            let ws_path = match lines.next() {
                Some(p) if !p.trim().is_empty() => p.trim(),
                _ => continue,
            };

            // Quick TCP check to see if the port is actually alive
            if tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port)).await.is_err() {
                tracing::debug!("{}: DevToolsActivePort exists but port {} is not listening (stale file)", name, port);
                continue;
            }

            let ws_url = format!("ws://127.0.0.1:{}{}", port, ws_path);
            tracing::info!(browser = name, port, "Found DevToolsActivePort, attempting auto-connect...");

            let mut config = StealthConfig::live();
            config.cdp_timeout = 5;

            match Self::spawn_connect(&ws_url, config, 15).await {
                Ok(browser) => {
                    tracing::info!("Auto-connected to {} via DevToolsActivePort (port {})", name, port);
                    return Some(Self::from_browser(browser, ConnectionMode::UserChrome, args.max_tabs, None, Some(ws_url)));
                }
                Err(e) => {
                    tracing::debug!("{}: auto-connect failed: {}", name, e);
                }
            }
        }
        None
    }

    /// Returns candidate browser data directories (Chrome first, then Edge).
    fn browser_data_dirs() -> Vec<(&'static str, std::path::PathBuf)> {
        let mut dirs = Vec::new();
        if let Some(home) = dirs::home_dir() {
            #[cfg(target_os = "macos")]
            {
                let chrome = home.join("Library/Application Support/Google/Chrome");
                if chrome.exists() { dirs.push(("Chrome", chrome)); }
                let edge = home.join("Library/Application Support/Microsoft Edge");
                if edge.exists() { dirs.push(("Edge", edge)); }
            }
            #[cfg(target_os = "linux")]
            {
                let chrome = home.join(".config/google-chrome");
                if chrome.exists() { dirs.push(("Chrome", chrome)); }
                let edge = home.join(".config/microsoft-edge");
                if edge.exists() { dirs.push(("Edge", edge)); }
            }
            #[cfg(target_os = "windows")]
            {
                if let Some(local) = dirs::data_local_dir() {
                    let chrome = local.join(r"Google\Chrome\User Data");
                    if chrome.exists() { dirs.push(("Chrome", chrome)); }
                    let edge = local.join(r"Microsoft\Edge\User Data");
                    if edge.exists() { dirs.push(("Edge", edge)); }
                }
            }
        }
        dirs
    }

    async fn connect_remote(url: &str, max_tabs: usize) -> anyhow::Result<Self> {
        let mut config = StealthConfig::live();
        config.cdp_timeout = 10;
        let browser = Self::spawn_connect(url, config, 10).await
            .map_err(|e| anyhow::anyhow!("Failed to connect to Chrome at {}: {}", url, e))?;
        tracing::info!("Connected to existing Chrome at {}", url);
        Ok(Self::from_browser(browser, ConnectionMode::UserChrome, max_tabs, None, None))
    }

    async fn launch_user_chrome(args: &crate::cli::Args) -> anyhow::Result<Self> {
        let profile_dir = super::profile::debug_profile_dir();

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
            config.cdp_timeout = 10;
            match Self::spawn_connect(&ws_url, config, 10).await {
                Ok(browser) => {
                    return Ok(Self::from_browser(browser, ConnectionMode::UserChrome, args.max_tabs, None, Some(ws_url)));
                }
                Err(e) => {
                    tracing::warn!("Orphan Chrome on port {} is unresponsive ({}), killing...", port, e);
                    kill_process_on_port(port).await;
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
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
        let mut child = cmd.spawn()
            .map_err(|e| anyhow::anyhow!("Failed to launch Chrome at {}: {}", chrome_path, e))?;

        let ws_url = match wait_for_debug_port(port, 10).await {
            Ok(url) => url,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(anyhow::anyhow!("Chrome 启动超时 (port {}): {}", port, e));
            }
        };

        tracing::info!(ws_url = %ws_url, "Chrome ready, connecting via WebSocket...");

        let mut config = StealthConfig::live();
        config.cdp_timeout = 10;
        let browser = match Self::spawn_connect(ws_url.trim(), config, 10).await {
            Ok(browser) => browser,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(anyhow::anyhow!(
                    "Failed to connect to Chrome (url={}): {}",
                    ws_url.trim(),
                    e
                ));
            }
        };

        tracing::info!("Connected to user Chrome (with login state)");
        Ok(Self::from_browser(browser, ConnectionMode::UserChrome, args.max_tabs, Some(child), Some(ws_url.trim().to_string())))
    }

    async fn launch_headless(args: &crate::cli::Args) -> anyhow::Result<Self> {
        let mut config = StealthConfig {
            headless: true,
            patch_binary: true,
            human_mouse: true,
            human_typing: true,
            cdp_timeout: 10,
            ..Default::default()
        };

        if let Some(ref path) = args.chrome_path {
            config.chrome_path = Some(path.clone());
        }

        let browser = Browser::launch_with_config(config).await
            .map_err(|e| anyhow::anyhow!("Failed to launch headless Chrome: {}", e))?;
        tracing::info!("Launched headless Chrome with stealth");
        Ok(Self::from_browser(browser, ConnectionMode::Headless, args.max_tabs, None, None))
    }

    fn from_browser(
        browser: Browser,
        mode: ConnectionMode,
        max_tabs: usize,
        chrome_child: Option<std::process::Child>,
        ws_url: Option<String>,
    ) -> Self {
        let browser = Arc::new(browser);
        let healthy = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let tab_pool = TabPool::new(browser.clone(), max_tabs, healthy.clone());
        tracing::info!("Tab pool ready (max {} tabs)", max_tabs);
        Self {
            browser,
            mode,
            tab_pool,
            chrome_child: std::sync::Mutex::new(chrome_child),
            healthy,
            connected_ws_url: ws_url,
        }
    }

    pub fn is_healthy(&self) -> bool {
        if !self.healthy.load(std::sync::atomic::Ordering::Relaxed) {
            return false;
        }

        let mut guard = self.chrome_child.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(child) = guard.as_mut() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    tracing::warn!("Chrome process exited: {}", status);
                    self.healthy.store(false, std::sync::atomic::Ordering::Relaxed);
                    return false;
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!("Failed to check Chrome process: {}", e);
                    self.healthy.store(false, std::sync::atomic::Ordering::Relaxed);
                    return false;
                }
            }
        }
        true
    }

    pub fn mark_unhealthy(&self) {
        self.healthy.store(false, std::sync::atomic::Ordering::Relaxed);
        tracing::warn!("Browser marked as unhealthy — will reconnect on next call");
    }

    pub fn mode(&self) -> &ConnectionMode {
        &self.mode
    }

    pub fn tab_pool(&self) -> &TabPool {
        &self.tab_pool
    }

    pub async fn list_tabs(&self) -> anyhow::Result<Vec<TabInfo>> {
        self.browser.tabs().await
            .map_err(|e| anyhow::anyhow!("Failed to list tabs: {}", e))
    }

    pub async fn attach_tab(&self, target_id: &str) -> anyhow::Result<Page> {
        self.browser.attach_page(target_id).await
            .map_err(|e| anyhow::anyhow!("Failed to attach to tab {}: {}", target_id, e))
    }

    pub async fn shutdown(&self) {
        if let Some(mut child) = self.chrome_child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
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

/// Kill processes listening on a port. Sends SIGTERM first, waits, then SIGKILL if needed.
pub async fn kill_process_on_port(port: u16) {
    #[cfg(unix)]
    {
        let output = tokio::process::Command::new("lsof")
            .args(["-ti", &format!(":{}", port)])
            .output()
            .await;
        let pids: Vec<String> = match output {
            Ok(out) => String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            Err(e) => {
                tracing::warn!(port, error = %e, "lsof failed, cannot find processes on port");
                return;
            }
        };
        if pids.is_empty() {
            return;
        }

        for pid in &pids {
            let _ = tokio::process::Command::new("kill")
                .args(["-TERM", pid])
                .output()
                .await;
            tracing::info!(pid, port, "Sent SIGTERM to process on debug port");
        }

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        if tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port)).await.is_ok() {
            for pid in &pids {
                let _ = tokio::process::Command::new("kill")
                    .args(["-9", pid])
                    .output()
                    .await;
                tracing::warn!(pid, port, "SIGTERM insufficient, sent SIGKILL");
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
    #[cfg(windows)]
    {
        let output = tokio::process::Command::new("netstat")
            .args(["-aon"])
            .output()
            .await;
        if let Ok(out) = output {
            let text = String::from_utf8_lossy(&out.stdout);
            let needle = format!(":{}", port);
            for line in text.lines() {
                if line.contains(&needle) && line.contains("LISTENING") {
                    if let Some(pid) = line.split_whitespace().last() {
                        let _ = tokio::process::Command::new("taskkill")
                            .args(["/F", "/PID", pid])
                            .output()
                            .await;
                        tracing::info!(pid, port, "Killed process on debug port (Windows)");
                    }
                }
            }
        }
    }
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
