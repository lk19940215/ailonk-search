# ailonk-search 使用场景示例

本目录收录 **ailonk-search** 在实际使用中的典型场景示例。所有示例均来自真实搜索调用结果，展示 AI 助手如何通过 MCP 工具完成信息搜集、技术调研、新闻追踪等任务。

每个示例文件均包含 **ailonk-search** 与 Cursor **web_search** 的完整搜索结果（标题、URL、摘要、全文摘录）及 **对比分析** 章节，可作为两种工具能力的参考文档，用于后续横向比较与方法评估。

---

## 示例列表

| 文件 | 场景 | 说明 |
|------|------|------|
| [01-stock-research.md](./01-stock-research.md) | 股票信息搜集 | 查询上市公司财务数据、业绩预告、社区分析 |
| [02-tech-docs.md](./02-tech-docs.md) | 技术文档搜索 | 搜索编程框架文档，自动获取中英文教程 |
| [03-product-research.md](./03-product-research.md) | 产品/行业调研 | 调研国产 GPU 厂商及 IPO 状态 |
| [04-news-tracking.md](./04-news-tracking.md) | AI/科技新闻追踪 | 追踪大模型最新发布与定价动态 |
| [05-industry-report.md](./05-industry-report.md) | 行业报告 / 市场研究 | eVTOL 市场规模与主要玩家 |
| [06-company-comparison.md](./06-company-comparison.md) | 跨行业公司对比 | 多轮搜索对比美光、闪迪、SpaceX、英伟达 |
| [07-deep-research.md](./07-deep-research.md) | 专题深度调研 | CJ-1000A 多轮搜索 + 交叉验证 |
| [08-tool-testing.md](./08-tool-testing.md) | 工具测试报告 | web_search / read_page / batch_read 参数与测试 |

---

## 工具选择指南

| 工具 | 适用场景 | 推荐度 |
|------|---------|--------|
| `search_and_read` | 搜索 + 自动读取 top 结果全文，**大多数调研任务的首选** | ⭐ 推荐 |
| `web_search` | 仅需搜索结果列表（标题、URL、摘要），不需要全文 | 按需 |
| `read_page` | 已有明确 URL，需要深入阅读单页内容 | 按需 |
| `batch_read` | 已有多个 URL，需要并发批量读取 | 按需 |
| `screenshot` | 需要页面视觉内容（布局、图表截图），文本请用 `read_page` | 特殊场景 |

---

## 搜索技巧

1. **加入时间词** — 在查询中加入「2026」「最新」「Q1」等时间限定词，过滤过时信息。
2. **中英混搜** — 技术类查询可同时使用中文和英文关键词（如 `Rust tokio async`），工具会自动匹配中英文内容。
3. **多轮搜索** — 复杂主题拆分为多轮针对性查询，比单次宽泛搜索效果更好（参见 [06-company-comparison.md](./06-company-comparison.md)）。
4. **调整 `read_count`** — 默认读取前 3 条结果全文；需要更多深度内容时可设为 4–5，仅需概览时设为 1–2。
5. **指定 `engine`** — 中国区可设 `"engine": "bing"` 获取更稳定的中文结果；英文技术文档可设 `"engine": "google"`。
6. **与 web_search 对比** — 每个示例均包含与 Cursor `web_search` 的完整搜索结果及「对比分析」章节（含对比表格与洞察），帮助判断不同工具的适用场景；详见各场景文件末尾的 **对比分析** 部分。

---

## 参数说明

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `search_count` | 10 | 搜索返回的结果条数（标题 + URL + 摘要） |
| `read_count` | 3 | 从前 N 条结果中选取并读取全文的页数（1–5） |
| `max_length_per_page` | 5000 | 每页提取内容的最大字符数 |

**为什么 `read_count` < `search_count`？**

这是有意为之的设计。`search_count` 提供完整的搜索结果列表供 AI 浏览和筛选，而 `read_count` 只对最相关的几条进行深度阅读。若对全部结果都读取全文，会导致响应极慢且消耗大量 Token。AI 会先查看 10 条摘要，再挑选 3 条最相关的深入阅读——兼顾广度与深度。
