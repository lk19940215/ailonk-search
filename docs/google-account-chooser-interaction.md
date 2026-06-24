# Google 账号选择弹窗交互方案

## 背景

2026-06-24 对照测试发现：Chrome DevTools MCP 可以通过 CDP 成功操作 Google 账号选择弹窗（网页版 `accounts.google.com/v3/signin/accountchooser`），完成 SSO 登录流程。

这意味着 ailonk-search 的 `click_authorize` 工具也可以（且已经）实现类似能力。

## 关键发现

### Google 账号选择弹窗是常规网页

Google SSO 有两种账号选择方式：

| 类型 | 技术实现 | CDP 可操作 | 示例 |
|------|----------|-----------|------|
| **网页版账号选择** | `accounts.google.com` 页面重定向/弹窗 | **是** | SSO 登录流中的新窗口 |
| **FedCM 浏览器原生弹窗** | Chrome 顶层 UI 浮层 | **否** | Chrome 内置账号选择器 |

网页版 `accounts.google.com` 账号选择器是标准 DOM 页面，可以通过 CDP 进行完整交互：
- 检测新 tab 打开
- 切换到弹窗 tab
- 读取账号列表（a11y tree / DOM）
- 点击目标账号
- 等待弹窗关闭 + 原页面 JWT token 回传

### Chrome DevTools MCP 测试记录

**测试流程**（Wiki SSO 授权）：

```
navigate_page → SSO 页面
take_snapshot  → 发现 Google "Sign in" 按钮
click          → 点击按钮，打开 accounts.google.com 弹窗
list_pages     → 检测到新 tab (accounts.google.com)
select_page    → 切换到弹窗 tab
take_snapshot  → 看到 "Choose an account" + 账号列表
click          → 选择 longkuo@akulaku.com
select_page    → 弹窗关闭，切回原 tab → Wiki 页面已登录
```

**结果**：成功完成 SSO 登录，共 8 步工具调用。

### ailonk-search 已有能力

`click_authorize` 已封装完整流程：
- 自动检测 SSO 页面类型（custom_sso / generic_login / google_saml）
- 自动点击 Google 登录按钮
- 自动处理 popup 弹窗（检测新 tab → 切换 → 选择账号 → 等待关闭）
- 自动等待重定向完成
- 单次调用完成，AI 无需逐步编排

## 对照测试数据

### 测试场景：TAPD + Wiki + Yapi 内容读取

| 维度 | ailonk-search | Chrome DevTools MCP |
|------|---------------|---------------------|
| **Wiki 读取** | |
| 工具调用次数 | 6 次（含 TAPD） | 12 次 |
| 成功读取页面 | 3 个（TAPD + 2 Wiki） | 1 个（Wiki a11y 快照） |
| 授权方式 | 1 次 `click_authorize` | 6 步手动编排 |
| **Yapi 读取** | |
| 工具调用次数 | 3 次 | 10+ 次 |
| 结果 | **成功**（接口列表 Markdown） | **失败**（SPA session 丢失） |
| **输出格式** | 干净 Markdown 正文 | a11y 树快照 |
| **AI 编排复杂度** | 低（工具自治） | 高（每步需判断） |

### Chrome DevTools MCP 失败原因（Yapi）

Yapi 是 SPA 应用。Chrome DevTools 的 `navigate_page` 触发全页面加载，导致：
1. SSO 在页面 A 完成登录
2. `navigate_page` 跳转到目标 URL 时创建新的页面加载上下文
3. SPA 客户端路由被刷新，session 状态丢失
4. 重定向回未登录的着陆页

ailonk-search 在同一 tab 内完成授权后直接读取，避免了这个问题。

## CDP 原生能力分析

Chrome DevTools MCP 测试暴露的问题 **不是 CDP 能力不足**，而是 **CDP 能力未被充分利用**。

### CDP 具备但未充分利用的能力

| CDP 能力 | Chrome DevTools MCP 使用情况 | ailonk-search 使用情况 |
|----------|---------------------------|----------------------|
| `Target.getTargets` | 通过 `list_pages` 暴露，但需 AI 手动调用 | 自动监听新 target 弹出 |
| `Target.attachToTarget` | 通过 `select_page` 暴露 | 自动 attach 到弹窗 tab |
| `Runtime.evaluate` | 有 `evaluate_script`，但未用于内容提取 | 用于 DOM 查询、按钮识别、内容提取 |
| `Page.navigate` | 通过 `navigate_page` 暴露（全页面刷新） | 在同一 tab 内导航，保持 session |
| `DOM.querySelector` | 间接通过 a11y 树暴露 | 直接用于定位 SSO 按钮、账号选择器 |
| `Network.getCookies` | 未暴露 | 用于验证登录状态 |

### Yapi 失败的根因

Yapi 是 SPA（hash 路由），Chrome DevTools 的 `navigate_page` 触发了 **全页面服务端加载**，而非客户端路由跳转。

**正确做法**（Chrome DevTools 具备但未使用）：

```javascript
// 使用 evaluate_script 在客户端导航，保持 session
window.location.href = 'http://testyapi.akulaku.com/project/1902/interface/api/cat_61510'
// 或直接操作 SPA hash 路由
window.location.hash = '#/project/1902/interface/api/cat_61510'
```

ailonk-search 在同一 tab 内完成 SSO 后直接 `Page.navigate` 到目标 URL，由于 cookie 已设置且在同一域下，SPA 正常识别登录状态。

### 两个产品的定位差异

```
┌──────────────────────────────────────────────────┐
│              Chrome DevTools MCP                  │
│  定位：通用浏览器调试工具                           │
│  能力：CDP 原子操作（导航、点击、截图、执行脚本）     │
│  适用：前端调试、性能分析、自动化测试                 │
│  内容提取：a11y 树快照（需 AI 二次处理）             │
│  Auth 处理：无（需 AI 逐步编排 6+ 步）              │
├──────────────────────────────────────────────────┤
│              ailonk-search                        │
│  定位：AI Agent 专用搜索 & 内容提取 MCP             │
│  能力：在 CDP 之上封装高层抽象                       │
│  适用：网页搜索、内容读取、SSO 授权                  │
│  内容提取：ML 正文提取 + Markdown 输出              │
│  Auth 处理：一次 click_authorize 自动完成           │
└──────────────────────────────────────────────────┘
```

## 可能的增强方向

### 1. 多账号选择策略

当前 `click_authorize` 选择逻辑：自动选择第一个账号（或匹配企业域名的账号）。

可增强为：
- 支持 `preferred_account` 参数指定优先账号
- 自动匹配企业域名（如 `@akulaku.com`）
- 记忆上次选择的账号

### 2. FedCM 能力边界

FedCM 浏览器原生弹窗（Chrome 顶层 UI）：
- CDP 目前无法拦截或操作
- `FedCm.enable()` 等 CDP 命令仍为实验性
- 首次使用 FedCM 的站点：需用户手动完成一次授权
- 后续访问：Chrome auto-reauthn 可自动处理

### 3. SPA 深度链接支持

ailonk-search 已实现此模式：
- 授权完成后在同一 tab 内 `Page.navigate` 到目标 URL
- cookie 在同域下共享，SPA 正常识别登录状态
- 不触发全页面服务端加载
