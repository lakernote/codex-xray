# Codex X-Ray 产品方向与技术架构

## 一句话定位

**Codex X-Ray 是 Codex 的本地可视化助手与诊断工作台：看清用量、看懂执行、正确配置。**

Codex 官方客户端已经在持续补齐账户用量、Profile、插件、Skills、模型和配置。Codex X-Ray 不追着复制这些页面，而是把官方能力、本机执行事实和开发者工作流连接起来，回答“钱和 Token 花在哪、每一步做了什么、当前环境到底怎样、怎样安全修改配置”。

## 官方做什么，Codex X-Ray 做什么

| 用户问题 | Codex 官方能力 | Codex X-Ray 的增量 |
| --- | --- | --- |
| 我用了多少 | 官方额度、Profile、Token 活动 | 本地输入/缓存/输出账本、逐模型成本、官方与本地口径对照 |
| 它现在在干什么 | 任务和对话界面 | 跨项目任务状态、审批/失败聚合、系统托盘提醒、直接跳回任务 |
| 为什么这一轮很慢或很贵 | 单个对话的结果 | 每轮 LLM、上下文变化、缓存、压缩、工具/MCP/Skill 调用和耗时 |
| 哪些扩展真正有用 | 安装与基础管理 | 按项目统计实际调用、Token 影响、失败率、延迟和使用趋势 |
| 怎样切换模型或环境 | `config.toml`、模型选择、Provider 配置 | 面向普通用户的配置解释、Profile 预览、校验、备份、切换和恢复；不触碰 `auth.json` |
| Codex 环境为什么异常 | 分散的设置与错误提示 | 版本、配置层、MCP、Skill、Plugin、Session 和 App Server 一站式诊断 |

## 产品信息架构

### 1. 用量与成本

不是再做一个官方 Profile。

- 官方额度和账户统计只作为可信对照。
- 本地 Session 提供今日实时、按日、按月、模型、缓存和 API 等价成本。
- 所有数字显示来源、公式、更新时间和是否完整。

### 2. 执行解剖

这是 Codex X-Ray 的核心能力。

- 目录遵循 Codex 的“项目 → 对话”结构。
- 用户点选后才解析单个 Session，并持久化结果。
- 逐回合展示 LLM 调用、Token 结算、上下文变化、压缩、MCP、CLI、Skill、插件和子 Agent。
- 不做模糊的“浪费分数”；展示可核对事实，让用户自己判断并学习 Codex。

### 3. 任务看守

- 汇总运行中、等待审批、等待输入、失败、完成。
- 托盘只显示最需要关注的状态。
- 点击回到对应 Codex 任务。
- 跨进程状态不可见时明确标记为本地推断，不伪装成官方实时状态。

### 4. 增强洞察

官方插件页回答“装了什么”，Codex X-Ray 回答“实际有没有用”。

- Skills、Plugins、MCP、Hooks 的本地清单。
- 按项目、对话和回合统计调用次数、失败率、耗时和 Token 影响。
- 发现未使用扩展、重复能力、配置冲突和启动失败。
- 官方仍标记为开发中的 App Server 方法不进入生产路径；先使用稳定接口和本地结构化事实。

### 5. 环境与 Provider Profile

这一模块是 Codex 的可解释控制台，也包含类似 CCSwitch 的安全 Provider 子集，但不是本地反向代理。

- 读取 `config/read`、`model/list`、用户级 `config.toml` 和命名 Profile。
- 展示当前 Provider、模型、Endpoint、认证变量名和生效范围。
- 把模型行为、权限审批、上下文与 Memory、工具能力转换为带说明和推荐值的 GUI，不要求用户先理解 TOML 键名。
- 切换前生成差异预览，写入时优先使用官方 `config/batchWrite`。
- 每次写入都备份、校验并支持一键恢复。
- API Key 只引用 `env_key`；首版不保存密钥，不读写 `auth.json`。
- 不做 OAuth 转发、额度绕过或自动把请求导向第三方。

## 技术栈

当前的 **Tauri 2 + Rust + React 19 + TypeScript + Vite** 是合适的，不需要重写。

### 桌面壳：Tauri 2

- 适合系统托盘、原生窗口、自动更新和跨平台打包。
- 安装体积和常驻内存明显低于 Electron。
- Rust 后端可以直接处理文件、子进程和 App Server，不需要额外本地服务。

### 数据与协议：Rust

Rust 负责所有接近 Codex 和本地磁盘的工作：

- 通过 stdio JSON-RPC 连接独立 `codex app-server`。
- 只读增量解析 `$CODEX_HOME/sessions/**/*.jsonl`。
- 对 Token、工具调用、参数和结果做脱敏与聚合。
- 监听 Session 和配置变化，避免固定频率全量扫描。
- 对配置写入执行预览、原子替换、备份和恢复。

建议下一阶段增加：

- `rusqlite`：使用单一 SQLite + WAL 保存启动缓存；用关系表保存 Session、Turn、Token、阶段和工具调用，每个文件独立更新。Trace 额外保留逐 Session 结构快照，用于快速、无损地还原 Timeline。
- `notify`：监听 Session、配置和扩展目录变化，只处理发生变化的文件。
- `keyring`：仅在未来确实需要保存第三方凭据时接入系统钥匙串；默认仍优先 `env_key`。

### 界面：React 19 + TypeScript

- 保留当前报表、树形目录、筛选和详情工作台。
- 图表优先使用 CSS/SVG，数据量和交互复杂度需要时再引入轻量图表库。
- 所有 Rust IPC 结果保持显式 TypeScript 类型；后续可评估自动生成类型，避免两端结构漂移。

### 本地数据层：SQLite

建议的数据粒度：

- `projects`
- `sessions`
- `turns`
- `llm_calls`
- `tool_calls`
- `extension_usage`
- `daily_usage`
- `provider_profiles`
- `index_state`

正文、完整补丁、读取到的文件内容和完整工具输出不进入数据库。存储的是结构化元数据、限长脱敏摘要和来源指针。

## 数据流与边界

1. Official Adapter 从 App Server 读取账户、目录、配置和稳定扩展信息。
2. Session Adapter 只读解析本机 JSONL，生成可持久化的结构化事件。
3. Domain 层把数据组织成用量明细、执行追踪、任务状态和扩展洞察。
4. SQLite 保存索引和分析结果，React 只通过 Tauri command 读取视图模型。
5. Config Adapter 默认只读；用户在控制台修改设置并确认差异后，才允许带版本校验和恢复点的写入。

## 开发顺序

### P0：把当前两项做成可靠工具

- 用量汇总数字与官方 Profile 对齐。
- 执行目录完整分页、按需解剖、持久化稳定。
- 所有来源和边界可解释。

### P1：环境诊断已实现，继续补齐任务看守

- 运行/审批/失败聚合。
- 托盘提醒和跳回 Codex。
- App Server、配置层、MCP、Skill、Plugin、Session 诊断页（基础版已实现）。

### P2：扩展效果分析

- 基础版已按已解剖 Session 统计 Skill、MCP、CLI、浏览器、自动化和子 Agent 的真实调用。
- 已关联覆盖项目/Session/Turn、可观测耗时、失败、重复和结果记录体积。
- Session 没有把 Token 归因到单个工具调用；产品不会伪造“某扩展消耗 Token”，后续只在有真实因果字段时增加该指标。
- 支持导出脱敏执行报告。

### P3：Codex 控制台（常用配置与 Provider 基础版已实现）

- 用自然语言解释常用设置，只提交用户实际修改的配置键。
- Provider 与普通配置都带差异预览、版本校验、恢复点和一键恢复。
- Codex App 需要重启或新开任务才能生效时明确提示，不承诺热切换。

## 明确不做

- 不再做桌面宠物。
- 不复制 Codex 官方插件市场和 Profile 社交页。
- 不读取或替换 `auth.json`。
- 不修改 Codex Session、数据库或任务内容；用户配置只在控制台展示差异并确认后通过官方配置接口更新。
- 不做订阅额度绕过、OAuth 转发或隐式请求代理。
- 不以不可解释的综合分数替代真实事件和数据来源。
