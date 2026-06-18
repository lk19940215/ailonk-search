# Bug #001: Chrome CDP 连接断开后无法自动恢复

**状态：已修复** | 修复日期: 2026-06-18

## 问题描述

ailonk-search 在 Chrome 进程退出或 WebSocket 连接断开后，所有工具调用失败，且无法自动恢复，需要手动重启 MCP 服务。

## 复现步骤

1. 启动 ailonk-search（在 Cursor 中配置为 MCP 服务）
2. 成功执行若干次 `search_and_read` 调用（验证工具正常）
3. 等待一段时间（Chrome 可能被系统回收或 WebSocket 超时）
4. 再次调用任何工具 → 失败

## 错误信息

```
MCP error -32603: Failed to create new tab: Transport error: 
CDP reader thread has exited — WebSocket connection is dead
```

## 现象

- **MCP 进程存活**：`ps aux | grep ailonk-search` 可以看到进程在运行（PID 26629）
- **Cursor 显示可用**：MCP 面板显示 ailonk-search 为可用状态
- **所有工具都失败**：`web_search`、`read_page`、`batch_read`、`search_and_read` 全部返回相同错误
- **无法自恢复**：反复调用仍然失败，必须重启 MCP 服务

## 根因分析

### 1. `LazyBrowserManager` 使用 `OnceCell` 单次初始化

```rust
// src/browser/manager.rs:8-11
pub struct LazyBrowserManager {
    args: crate::cli::Args,
    inner: tokio::sync::OnceCell<Arc<BrowserManager>>,
}
```

`tokio::sync::OnceCell` 一旦初始化成功，值就被永久缓存，即使内部的 Chrome 连接已经死亡，也不会重新初始化。

### 2. `get()` 方法没有健康检查

```rust
// src/browser/manager.rs:21-29
pub async fn get(&self) -> anyhow::Result<&Arc<BrowserManager>> {
    self.inner
        .get_or_try_init(|| async {
            tracing::info!("First tool call — initializing Chrome...");
            let bm = BrowserManager::new(&self.args).await?;
            Ok(Arc::new(bm))
        })
        .await
}
```

当 `OnceCell` 已初始化后，`get_or_try_init` 直接返回缓存值，不会检查 Chrome 连接是否还活着。

### 3. 错误发生位置

```rust
// src/browser/pool.rs:34-35
let page = self.browser.new_blank_page().await
    .map_err(|e| anyhow::anyhow!("Failed to create new tab: {}", e))?;
```

`browser.new_blank_page()` 尝试通过已死的 WebSocket 创建新 Tab，触发 `Transport error: CDP reader thread has exited — WebSocket connection is dead`。

## 建议修复方案

### 方案 A：在 `LazyBrowserManager::get()` 中加入健康检查 + 重建

将 `OnceCell` 替换为 `tokio::sync::RwLock<Option<Arc<BrowserManager>>>`，在每次 `get()` 时检查连接健康状态。

```rust
pub struct LazyBrowserManager {
    args: crate::cli::Args,
    inner: tokio::sync::RwLock<Option<Arc<BrowserManager>>>,
}

impl LazyBrowserManager {
    pub async fn get(&self) -> anyhow::Result<Arc<BrowserManager>> {
        // 快速路径：读锁检查
        {
            let guard = self.inner.read().await;
            if let Some(bm) = guard.as_ref() {
                if bm.is_healthy().await {
                    return Ok(bm.clone());
                }
                tracing::warn!("Chrome connection dead, will reinitialize...");
            }
        }
        // 慢速路径：写锁重建
        let mut guard = self.inner.write().await;
        // Double-check（可能其他任务已重建）
        if let Some(bm) = guard.as_ref() {
            if bm.is_healthy().await {
                return Ok(bm.clone());
            }
            // 关闭旧实例
            bm.shutdown().await;
        }
        tracing::info!("Reinitializing Chrome...");
        let bm = Arc::new(BrowserManager::new(&self.args).await?);
        *guard = Some(bm.clone());
        Ok(bm)
    }
}
```

### 方案 B：在 `BrowserManager` 中加入 `is_healthy()` 方法

```rust
impl BrowserManager {
    pub async fn is_healthy(&self) -> bool {
        // 尝试创建一个页面来验证连接
        match self.browser.new_blank_page().await {
            Ok(page) => {
                let target_id = page.target_id().to_string();
                self.browser.close_tab(&target_id).await.ok();
                true
            }
            Err(_) => false,
        }
    }
}
```

或者更轻量的方式 — 检查 eoka 的底层 WebSocket 状态（如果 eoka 暴露了此 API）。

### 方案 C：在 `TabPool::acquire()` 中捕获连接错误并触发重建

```rust
// 在 pool.rs 中，如果 new_blank_page 失败且错误含 "WebSocket" 或 "CDP reader"
// 则向上层返回特定错误类型，由 server/mod.rs 的工具处理器触发重建
```

## 推荐

**方案 A + B 组合**：用 `RwLock<Option<...>>` 替代 `OnceCell`，配合 `is_healthy()` 检查。这样：
- 正常情况下走读锁快速路径，性能无损
- Chrome 断开时自动重建，用户无感知
- 避免了重启 MCP 服务的操作

## 额外建议

1. **Chrome 进程监控**：对 `launch_user_chrome()` 启动的 Chrome 子进程，可以在后台 spawn 一个 task 等待其退出（`child.wait()`），触发主动重连
2. **MCP 健康检查响应**：在 MCP server 层面暴露连接状态，使 Cursor 的 MCP 面板能准确反映工具可用性
3. **日志增强**：在 Chrome 连接断开时输出 WARN 级别日志，便于调试

## 测试影响

此 bug 导致本次测试中 `web_search`、`read_page`、`batch_read` 三个工具无法独立测试。`search_and_read` 在 Chrome 存活期间完成了 15+ 次成功调用。

## 修复方案（已实施）

采用 **方案 A + B + C 组合**：

### 核心改动

| 文件 | 改动 |
|------|------|
| `src/browser/manager.rs` | `OnceCell` → `RwLock<Option<...>>`，双重检查锁定，`is_healthy()` 检查子进程 + AtomicBool 标志 |
| `src/browser/pool.rs` | `TabPool::acquire()` 在 CDP 失败时设置 `healthy=false` |
| `src/server/mod.rs` | `check_cdp_error()` 在工具层检测 transport 错误并标记 unhealthy |

### 恢复流程

1. Chrome 死亡 → 首次工具调用 → `acquire()` 或 `navigate()` 失败 → 标记 `healthy=false`
2. AI 重试 → `get()` 检测到 unhealthy → 写锁关闭旧实例 → 重新启动 Chrome → 返回新连接
3. 后续调用正常

### 已知限制

- 首次失败不可避免（Chrome 已死，需一次调用触发检测），AI 重试后恢复
- Headless/Remote 模式无子进程监控，依赖 CDP 错误触发重建

## 环境信息

- macOS (darwin 24.6.0)
- ailonk-search: 本地编译版本
- eoka: 当前版本（通过 Cargo.toml 管理）
- Chrome: v149.0.7827.114
- 发现日期: 2026-06-18
- 修复日期: 2026-06-18
