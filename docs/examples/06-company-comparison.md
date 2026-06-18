# 场景 6：跨行业公司对比（美光 · 闪迪 · SpaceX · 英伟达）

展示 **多轮搜索** 策略：单次宽泛查询容易被某一话题主导，拆分后效果更好。

## 搜索查询

**第一轮（宽泛）**：美光科技 Micron 闪迪 SanDisk SpaceX 英伟达 NVIDIA 2026 最新动态 财报 对比

**第二轮（SpaceX 专项）**：SpaceX 2026 星舰 Starship 发射 估值 IPO

**第三轮（英伟达专项）**：NVIDIA 英伟达 2026 最新财报 Blackwell 芯片 市值

**web_search（对比）**：美光 闪迪 SpaceX 英伟达 2026 最新动态 对比

---

## ailonk-search 结果

### 第一轮：宽泛搜索

#### 搜索参数

```json
{
  "tool": "search_and_read",
  "arguments": {
    "query": "美光科技 Micron 闪迪 SanDisk SpaceX 英伟达 NVIDIA 2026 最新动态 财报 对比",
    "search_count": 10,
    "read_count": 3
  }
}
```

#### 搜索结果列表

1. **[存储芯片股持续飙升_新浪财经](https://finance.sina.com.cn/stock/marketresearch/20260509/storage-chip-rally.html)** — 2026年5月9日 — 美光 +15.5%，闪迪 +16.6%，五强格局
2. **[美光 vs 闪迪 2026_edgen.tech](https://www.edgen.tech/analysis/micron-vs-sandisk-2026)** — 2026年4月14日 — MU HBM 领先（$2.18B/Q），SNDK BiCS10 NAND
3. **[AI引爆存储芯片狂潮_腾讯新闻](https://news.qq.com/rain/a/20260602A08KAG00)** — 2026年6月2日 — 美光市值破万亿（50 天翻倍），闪迪一年涨 45 倍
4. **[存储芯片超级周期_Odaily](https://www.odaily.news/post/storage-supercycle-2026)** — 2026年5月 — AI 需求驱动存储芯片超级周期
5. **[美光科技48天市值翻倍_网易](https://www.163.com/dy/article/MU-48days-double.html)** — 2026年5月 — 美光 48 天市值翻倍分析
6. **[闪迪分拆后一年涨45倍_腾讯新闻](https://news.qq.com/rain/a/sndk-45x-2026)** — 2026年4月 — 闪迪从西部数据分拆后的资本表现
7. **[美光 vs 闪迪：分析师目标价对比_雪球](https://xueqiu.com/S/MU)** — 2026年5月 — 美光 $465 目标 vs 公允 $423；闪迪 $944 目标 vs 公允 $698
8. **[存储芯片五强格局_新浪财经](https://finance.sina.com.cn/stock/storage-five-giants-2026)** — 2026年5月 — 美光、闪迪、三星、SK 海力士、铠侠
9. **[美光FY2026营收预测_东方财富](https://data.eastmoney.com/report/MU-FY2026.html)** — 2026年5月 — FY2026E 营收 $400 亿+
10. **[SNDK BiCS10 NAND技术突破_电子工程专辑](https://www.eet-china.com/mp/sndk-bics10-nvidia.html)** — 2026年4月 — BiCS10 332 层 NAND，与 NVIDIA 联合开发

#### 全文阅读内容

**存储芯片双雄对比摘要**

| 指标 | 美光 (MU) | 闪迪 (SNDK) |
|------|-----------|-------------|
| HBM 营收 | Q2 $2.18B（+22%） | — |
| NAND 技术 | HBM3E 领先 | BiCS10 332 层（NVIDIA 联合开发） |
| FY2026E 营收 | $400 亿+ | — |
| 分析师目标价 | $465（公允 $423） | $944（公允 $698） |
| 结论 | **买入**（合理估值 + HBM 可见度） | **持有**（等回调） |

**问题**：本轮结果几乎全部被存储芯片话题占据，SpaceX 和英伟达信息缺失 → 需拆分专项搜索。

---

### 第二轮：SpaceX 专项

#### 搜索参数

```json
{
  "tool": "search_and_read",
  "arguments": {
    "query": "SpaceX 2026 星舰 Starship 发射 估值 IPO",
    "search_count": 10,
    "read_count": 3
  }
}
```

#### 搜索结果列表

1. **[SpaceX正式提交IPO申请_cislunarspace](https://www.cislunarspace.cn/spacex-ipo-s1-2026)** — 2026年5月20日 — 估值 1.75–2 万亿，星舰研发累计 >150 亿
2. **[从星舰V3到万亿美元IPO_网易](https://www.163.com/dy/article/KT83TB1B05198UNI.html)** — 2026年5月18日 — Starship V3 试飞，商业航天投资狂潮
3. **[SpaceX IPO时间表：最早6月12日_36kr](https://36kr.com/p/spacex-ipo-timeline-2026)** — 2026年5月 — 纳斯达克代码 SPCX，最早 6/12 上市
4. **[SpaceX捆绑Grok AI上市方案_彭博](https://www.bloomberg.com/news/spacex-grok-ai-merger-2026)** — 2026年5月 — 与 xAI/Grok 捆绑估值方案
5. **[星链年营收113亿美元_SpaceNews](https://spacenews.com/starlink-revenue-113b-2026/)** — 2026年Q1 — 星链年化营收 $113 亿
6. **[SpaceX S-1招股书曝光_Financial Times](https://www.ft.com/spacex-s1-filing-2026)** — 2026年5月20日 — 募资 $750–800 亿
7. **[Starship V3试飞成功_SpaceX官网](https://www.spacex.com/launches/starship-v3-2026)** — 2026年5月 — V3 版本试飞进展
8. **[SpaceX航天业务Q1亏损_Reuters](https://www.reuters.com/spacex-q1-loss-2026/)** — 2026年Q1 — 航天发射业务 Q1 亏损 $6.62 亿
9. **[SpaceX估值对比分析_雪球](https://xueqiu.com/S/SPACEX-IPO)** — 2026年5月 — 估值 1.75–2 万亿 vs 特斯拉
10. **[商业航天投资狂潮2026_财新](https://www.caixin.com/2026/spacex-commercial-space.html)** — 2026年5月 — 商业航天板块整体分析

#### 全文阅读内容

**SpaceX IPO 核心数据**

| 项目 | 数据 |
|------|------|
| S-1 提交日期 | 2026 年 5 月 20 日 |
| 上市代码 | SPCX / 纳斯达克 |
| 募资规模 | $750–800 亿 |
| 估值 | $1.75–2 万亿 |
| 星链年化营收 | $113 亿 |
| 航天业务 Q1 亏损 | $6.62 亿 |
| 星舰研发累计投入 | >$150 亿 |
| 最早上市日期 | 2026 年 6 月 12 日 |

---

### 第三轮：英伟达专项

#### 搜索参数

```json
{
  "tool": "search_and_read",
  "arguments": {
    "query": "NVIDIA 英伟达 2026 最新财报 Blackwell 芯片 市值",
    "search_count": 10,
    "read_count": 3
  }
}
```

#### 搜索结果列表

1. **[NVIDIA 2026财年Q4及全年财务报告_nvidia.cn](https://blogs.nvidia.cn/blog/fy2026-q4-earnings/)** — 2026年2月25日 — Q4 $681 亿（+73%），FY2026 $2,159 亿（+65%）
2. **[英伟达2026财报大超预期_雪球](https://xueqiu.com/S/NVDA)** — 2026年5月21日 — 最新 Q 营收 $816 亿，数据中心 $752 亿，Q2 指引 $910 亿
3. **[Blackwell GB200放量分析_SemiAnalysis](https://semianalysis.com/blackwell-gb200-ramp-2026)** — 2026年5月 — GB200/GB300 NVL72 占高端 71%+
4. **[NVIDIA Rubin平台发布_The Verge](https://www.theverge.com/nvidia-rubin-platform-2026)** — 2026年3月 — Rubin 平台接棒 Blackwell
5. **[英伟达股息提升_雅虎财经](https://finance.yahoo.com/nvda-dividend-increase-2026)** — 2026年2月 — 股息 $0.01 → $0.25
6. **[数据中心营收623亿_彭博](https://www.bloomberg.com/nvda-datacenter-q4-2026)** — 2026年2月 — 数据中心 Q4 $623 亿（+75%）
7. **[Blackwell vs Hopper性能对比_AnandTech](https://www.anandtech.com/nvidia-blackwell-vs-hopper)** — 2026年4月 — Blackwell 推理成本降至 Hopper 1/4
8. **[英伟达FY2026股东回报411亿_SEC Filing](https://www.sec.gov/nvda-fy2026-10k)** — 2026年2月 — 股东回报 $411 亿
9. **[Rubin推理成本降至Blackwell 1/10_Tom's Hardware](https://www.tomshardware.com/nvidia-rubin-inference-cost)** — 2026年3月 — Rubin 推理成本优化
10. **[英伟达市值3万亿_华尔街见闻](https://wallstreetcn.com/nvda-3trillion-2026)** — 2026年5月 — 市值突破 3 万亿美元

#### 全文阅读内容

**NVIDIA FY2026 财务摘要**

| 指标 | 数值 |
|------|------|
| FY2026 营收 | $2,159 亿（+65%） |
| Q4 营收 | $681 亿（+73%） |
| 数据中心 Q4 | $623 亿（+75%） |
| 最新 Q 营收 | $816 亿 |
| 数据中心最新 Q | $752 亿 |
| Q2 指引 | $910 亿 |
| 毛利率 | 75% |
| 股东回报 | $411 亿 |
| 股息 | $0.01 → $0.25 |

**产品路线图**

- Blackwell：GB200/GB300 NVL72 占高端市场 71%+
- Rubin：推理成本降至 Blackwell 的 1/10

---

## web_search 结果

### 搜索查询

美光 闪迪 SpaceX 英伟达 2026 最新动态 对比

### 搜索结果列表

1. **[SpaceX值疯了？拆解2万亿市值_OFweek](https://semi.ofweek.com/spacex-2trillion-valuation-2026)** — SpaceX 6/12 上市 135$/股，市值冲 2.1 万亿，太空 AI 数据中心
2. **[SpaceX夜盘续涨近2%_今日美股网](https://www.todayusstock.com/spacex-premarket-2026)** — 上市累涨超 49%，美光/闪迪/迈威尔齐涨 2%
3. **[SpaceX 1.25万亿IPO前夜_36kr](https://36kr.com/p/spacex-xai-merger-2026)** — 收购 xAI 合并估值 1.25 万亿，获 600 亿 Cursor 收购选择权
4. **[英伟达Vera Rubin放量引爆全球存储荒_TradingKey](https://www.tradingkey.com/nvidia-rubin-storage-shortage-2026)** — 闪迪即将突破 $2000，瑞穗目标 $2200
5. **[美光闪迪大反弹_FX168](https://news.fx168news.com/micron-sandisk-rally-2026)** — 大摩揭秘：英伟达减配非需求下滑而是 DRAM 供应不足，黄仁勋称需求热潮延续数年

### Synthesis（AI自动摘要）

SpaceX 市值 2.1 万亿，英伟达 Vera Rubin 放量，美光绑定英伟达，闪迪年内涨 50 倍。「英伟达提供计算 + 美光 DRAM/HBM + 闪迪 NAND/SSD = AI 基础设施底层」

### 全文内容

**产业链关系（来自 synthesis 与 TradingKey）**

```
NVIDIA（计算/GPU）
    ↓ 需求传导
Micron（DRAM/HBM 供应）
    ↓ 配套
SanDisk（NAND/SSD 存储）
```

- 英伟达 Rubin 放量 → DRAM 供应不足 → 美光/闪迪齐涨
- 大摩：英伟达「减配」系供应瓶颈，非需求下滑
- 黄仁勋：AI 需求热潮将延续数年

---

## 对比分析

| 维度 | ailonk-search（3 轮） | web_search（1 次） |
|------|----------------------|-------------------|
| 宽泛搜索 | 被存储芯片话题主导，SpaceX/NVIDIA 缺失 | 一次搜索覆盖 4 家公司 |
| SpaceX 深度 | 第二轮专搜：S-1、估值、星链营收、Q1 亏损 | synthesis 提及 2.1 万亿市值 |
| 英伟达深度 | 第三轮专搜：FY2026 完整财报、Blackwell/Rubin | synthesis 提及 Rubin 放量 |
| 存储芯片 | 第一轮：HBM $2.18B/Q、BiCS10、分析师目标价 | 闪迪 $2000、年内涨 50 倍 |
| 跨公司洞察 | 无（各公司独立分析） | 「英伟达计算 + 美光 DRAM + 闪迪 NAND = AI 基础设施」 |
| 总耗时 | 3 轮 × ~30s ≈ 90s | ~2s + synthesis |
| Token 消耗 | 高（3× 全文） | 低（摘要优先） |

**四公司汇总**

| Company | Key Data | Status |
|---------|----------|--------|
| Micron (美光) | 市值破万亿，HBM Q2 $2.18B，FY2026E 营收 $400 亿+ | 存储芯片 AI 超级周期，**买入** |
| SanDisk (闪迪) | 分拆后一年涨 45 倍，BiCS10 332 层 NAND | 与 NVIDIA 联合开发，**持有** |
| SpaceX | 估值 $1.75–2 万亿，IPO 计划 6/12，星链年营收 $113 亿 | 史上最大 IPO |
| NVIDIA (英伟达) | FY2026 营收 $2,159 亿（+65%），数据中心 $623 亿/Q | Blackwell 放量，Rubin 接棒 |

**洞察**

- 宽泛多公司查询时，`web_search` 的 synthesis 效率更高，还能提炼跨公司产业链关系。
- `ailonk-search` 需多轮拆分，但每轮返回完整页面原文，单公司深度更足（如 SpaceX S-1 细节、NVIDIA 完整财报表）。
- 推荐组合：先用 `web_search` 快速横览建立框架，再用 `ailonk-search` 逐家深挖。

---

## 后续查询建议

1. `美光 HBM3E 2026 产能 市场份额 vs SK海力士`
2. `SpaceX Starship V3 试飞 2026 时间表 月球任务`
3. `NVIDIA Rubin GPU 2026 发布 推理成本 Blackwell 对比`
