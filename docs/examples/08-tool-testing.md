# 工具测试报告：web_search / read_page / batch_read

本文档记录 ailonk-search 各独立工具的测试情况。

---

## 工具参数总览

### web_search

仅搜索，不读取页面全文。

```json
{
  "tool": "web_search",
  "arguments": {
    "query": "搜索关键词",
    "count": 10,
    "engine": "bing"
  }
}
```

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `query` | string | 必填 | 搜索关键词 |
| `count` | uint | 10 | 返回结果数（1-20） |
| `engine` | enum | auto | 搜索引擎：auto / google / bing / duckduckgo |

**返回**：标题 + URL + 摘要的列表（不读取全文）

**适用场景**：
- 只需要搜索结果列表，不需要页面全文
- 需要指定搜索引擎
- 快速获取 URL 列表供后续 `read_page` / `batch_read` 使用

### read_page

读取单个 URL 的全文内容。

```json
{
  "tool": "read_page",
  "arguments": {
    "url": "https://example.com/article",
    "max_length": 15000,
    "include_links": true
  }
}
```

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `url` | string | 必填 | 目标页面 URL |
| `max_length` | uint | 15000 | 提取内容最大字符数 |
| `include_links` | bool | true | 是否保留超链接 |

**返回**：页面主要内容的 Markdown 格式提取

**适用场景**：
- 已知特定 URL，需要深入阅读
- 需要自定义 max_length（如全文提取设 20000+，摘要设 3000）
- 处理 JS 渲染页面和 Cookie 弹窗

### batch_read

批量并发读取多个 URL。

```json
{
  "tool": "batch_read",
  "arguments": {
    "urls": [
      "https://example.com/page1",
      "https://example.com/page2",
      "https://example.com/page3"
    ],
    "concurrency": 5,
    "max_length_per_page": 5000
  }
}
```

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `urls` | string[] | 必填 | URL 列表（最多 10 个） |
| `concurrency` | uint | 5 | 并发 Tab 数（最大 10） |
| `max_length_per_page` | uint | 5000 | 每页最大字符数 |

**返回**：每个页面的提取内容 + 成功/失败统计

**适用场景**：
- 已有多个 URL 需要同时读取
- 对比多个信源的内容
- 搜索后批量深读特定结果

### search_and_read（组合工具）

搜索 + 自动读取 = `web_search` + `read_page` 的一站式组合。

```json
{
  "tool": "search_and_read",
  "arguments": {
    "query": "搜索关键词",
    "search_count": 10,
    "read_count": 3,
    "max_length_per_page": 5000
  }
}
```

**关系**：`search_and_read` ≈ `web_search(count=search_count)` + `read_page(url=top_N, max_length=max_length_per_page)` × `read_count`

---

## 测试结果

### 测试日期

2026-06-18

### search_and_read — 已完成（7 个场景）

在 [01-07 场景示例](./README.md) 中，`search_and_read` 已完成大量测试，覆盖：
- 中文搜索（股票、产品、行业报告）
- 英文搜索（技术文档）
- 多轮搜索（CJ-1000A 深度调研）
- 多公司对比（美光/闪迪/SpaceX/英伟达）

所有测试结果已保存在对应场景文件中。

### web_search / read_page / batch_read — 测试中断

**问题**：Chrome 实例断开连接

```
MCP error -32603: Failed to create new tab: Transport error: 
CDP reader thread has exited — WebSocket connection is dead
```

**测试调用记录**：

#### web_search 测试

```json
{
  "tool": "web_search",
  "arguments": {
    "query": "SpaceX IPO 2026 上市 股价 首日表现",
    "count": 10,
    "engine": "bing"
  }
}
```

结果：❌ Chrome CDP 连接已断开

#### read_page 测试

```json
{
  "tool": "read_page",
  "arguments": {
    "url": "https://blogs.nvidia.cn/blog/nvidia-announces-financial-results-for-fourth-quarter-and-fiscal-2026/",
    "max_length": 8000
  }
}
```

结果：❌ Chrome CDP 连接已断开

#### batch_read 测试

```json
{
  "tool": "batch_read",
  "arguments": {
    "urls": [
      "https://tokio.rs/tokio/tutorial",
      "https://www.rust-lang.org/",
      "https://github.com/nickel-org/nickel.rs"
    ],
    "concurrency": 3,
    "max_length_per_page": 3000
  }
}
```

结果：❌ 0/3 成功，全部因 Chrome CDP 连接断开失败

**根因分析**：

ailonk-search 通过 CDP（Chrome DevTools Protocol）连接到 Chrome 实例。设计上工具会自动启动 Chrome 服务，但存在以下问题：

1. **`BrowserManager` 使用 `OnceCell` 单次初始化**：Chrome 启动后连接信息被缓存，当 Chrome 进程退出后，`OnceCell` 不会重新初始化，导致后续调用使用已死的 WebSocket 连接
2. **Cursor 显示 MCP 可用**：MCP 进程本身存活（PID 26629），但内部的 Chrome CDP 连接已断开。Cursor 的 MCP 状态检测无法感知内部连接健康状态
3. **缺少健康检查**：没有 WebSocket 连接存活检测和自动重连/重启机制

**Bug 记录**：

- **问题**：Chrome 进程退出后 ailonk-search 无法自动恢复
- **影响**：长时间空闲或 Chrome 被外部关闭后，所有工具调用失败
- **临时解决**：在 Cursor 中重启 ailonk-search MCP 服务
- **建议修复**：在 `BrowserManager` 中增加连接健康检查，检测到 WebSocket 断开时自动重启 Chrome 并重建连接

`search_and_read` 在此前的测试中运行正常（累计执行 15+ 次调用无失败），说明工具在 Chrome 存活期间稳定性良好。此次断连发生在长时间不活动之后。

---

## 工具对比与选型建议

| 工具 | 输入 | 输出 | Token 消耗 | 适用场景 |
|------|------|------|-----------|---------|
| `web_search` | 关键词 | 标题+URL+摘要 | 低 | 快速获取搜索结果列表 |
| `read_page` | 单个 URL | 全文 Markdown | 中 | 精读已知页面 |
| `batch_read` | 多个 URL | 多页 Markdown | 高 | 批量对比多源内容 |
| `search_and_read` | 关键词 | 搜索列表+全文 | 中-高 | **推荐**：一站式搜索+阅读 |

### 推荐工作流

```
场景1: 快速概览
  → web_search（只看标题和摘要）

场景2: 常规调研
  → search_and_read（搜索+自动读取前3条）

场景3: 深度调研
  → search_and_read（第1轮广泛搜索）
  → search_and_read（第2轮针对性搜索）
  → batch_read（精读第1-2轮发现的高价值URL）

场景4: 已知URL的内容提取
  → read_page（单个URL）
  → batch_read（多个URL并发读取）
```

---

## 待补充测试

Chrome 连接恢复后，需补充以下测试：

- [ ] `web_search` 不同引擎对比（bing vs google vs duckduckgo）
- [ ] `read_page` 长页面提取（max_length=20000）
- [ ] `read_page` JS 渲染页面（SPA 应用）
- [ ] `batch_read` 10 个 URL 并发读取性能
- [ ] `batch_read` 部分 URL 失败时的容错行为
- [ ] 各工具的响应时间基准测试
