# Codex X-Ray 数据来源与指标口径

版本：0.1.0
原则：官方值原样展示，本地值明确标注为“推导”或“估算”，不把缺失数据补成看似精确的数字。

## 数据通路

Codex X-Ray 使用四类互不混淆的数据：

1. **官方账户通路**：启动当前安装的 `codex app-server`，通过本地 stdio JSONL 与 JSON-RPC 通信，只调用账户读取方法。App Server 是 Codex 官方为富客户端提供的接口。
2. **官方对话元数据**：调用只读 `thread/list`，只保留线程 ID、用户可见对话名、`cwd` 与结构化活动状态。不会把 `preview` 或对话正文写入 Codex X-Ray 索引。
3. **本机会话通路**：只读扫描 `$CODEX_HOME/sessions/**/*.jsonl` 中的结构化事件。默认的 `CODEX_HOME` 是 `~/.codex`。Codex X-Ray 不修改这些文件。
4. **Codex 配置通路**：调用官方 App Server 的 `config/read` 读取合并后的配置与用户层版本，并通过 `model/list` 获取当前可用的官方模型。只有用户在控制台查看变更预览并再次确认后，才调用 `config/batchWrite` 更新所选用户配置。

Codex X-Ray 给自己启动的 App Server 指定独立的 `sqlite_home`，因此不会和 Codex App 的 App Server 争用运行时数据库。`thread/list` 可从共享 session 记录修复 Codex X-Ray 自己的线程元数据副本，不写 Codex App 的数据库，也不启动模型回合。

## 环境诊断来源

环境诊断页组合三类只读事实：

- Codex X-Ray 启动的 CLI/App Server 提供当前可执行文件路径、版本，以及官方 `config/read` 返回的合并配置。
- `$CODEX_HOME`、`config.toml`、`sessions`、Skills 和 Plugin cache 只检查路径是否存在及顶层目录数量；不读取扩展正文。
- MCP 只读取配置名称、启用状态、传输类型和脱敏后的目标摘要；不展示 headers、环境变量值或任何凭据。

环境诊断中的“分析数据库”是 Codex X-Ray 自己的 `codex-xray.sqlite`，不是 Codex App 的 SQLite。X-Ray 启动的 App Server 另用隔离的 `sqlite_home` 保存其运行状态。环境诊断不读取 `auth.json`，也不声称能够附着到另一个 Codex App 进程。

## Provider 配置与国产 Responses

Codex 当前的自定义 Provider 使用 `wire_api = "responses"`。界面里的“OpenAI 兼容”不会自动视为可用：只有厂商明确实现 `/responses`、SSE 事件和函数工具调用时，才标记为原生或托管 Responses。

| 预设 | 类型 | Base URL | 默认环境变量 | 说明 |
|---|---|---|---|---|
| OpenAI | Codex 内置 | Codex 内置 | Codex 登录 | 不新增自定义 Provider |
| 阿里百炼 / Qwen | 厂商原生 Responses | `https://dashscope.aliyuncs.com/compatible-mode/v1` | `DASHSCOPE_API_KEY` | 现有公共域名仍可用，官方建议换成业务空间专属域名 |
| 火山方舟 / 豆包 | 厂商原生 Responses | `https://ark.cn-beijing.volces.com/api/v3` | `ARK_API_KEY` | 模型 ID 需与方舟控制台/文档一致 |
| 百度千帆 / GLM、DeepSeek | 云平台托管 Responses | `https://qianfan.baidubce.com/v2` | `QIANFAN_API_KEY` | 千帆把指定 GLM、DeepSeek、Qwen 模型暴露为 Responses |
| MiniMax | 厂商原生 Responses | `https://api.minimaxi.com/v1` | `MINIMAX_API_KEY` | 官方提供 Codex 桌面端接入说明 |
| StepFun | 厂商原生 Responses | `https://api.stepfun.com/v1` | `STEP_API_KEY` | 当前官方 Responses 文档列出 `step-3.7-flash` |

GLM、DeepSeek、Kimi、腾讯混元和硅基流动等厂商的已知直连域名如果只提供 Chat Completions，Codex X-Ray 会阻止把该地址误写成 Responses Provider。用户仍可选择真正实现 Responses 的云平台托管入口或自有转换网关。

### 让 Codex 桌面端看到环境变量

Provider 页面只写环境变量名。macOS 从 Finder 启动的 Codex 不会自动继承终端里的 `export`。为避免真实 Key 进入 shell 历史，可在 zsh 中运行下列命令（把变量名换成预设显示的名称），然后完全退出并重新打开 Codex 与 Codex X-Ray：

```zsh
read -s "provider_key?API Key: "
launchctl setenv DASHSCOPE_API_KEY "$provider_key"
unset provider_key
```

不要把真实 Key 写进项目文件或截图；不再使用时可运行 `launchctl unsetenv DASHSCOPE_API_KEY`。只使用 Codex CLI 时，在同一终端会话中设置环境变量即可。

### 写入与恢复边界

1. 首屏只调用 `config/read`，不会读取 `auth.json`；配置结果中的凭据字段不会被提取、返回前端或保存。
2. API Key 字段只接受环境变量名。保存配置时不要求、读取或持久化真实 Key；只有用户主动点击“测试连接”时，后端才临时读取该环境变量并发送一次最小 Responses 请求。Key 不返回前端、不写日志、不进入命令行参数。
3. “预览变更”只在界面显示当前与目标 Provider、模型和 Endpoint，不写文件。
4. “确认切换”调用 `config/batchWrite`，并携带 `config/read` 返回的用户层版本，避免静默覆盖并发修改。
5. 写入前的 Provider、模型与非敏感 Endpoint 定义保存到 Codex X-Ray 应用数据目录，可通过“恢复上一个”切回；恢复前的状态也会变成新的恢复点。
6. Provider 变更本身不启动模型回合、不消耗 Codex 额度；“测试连接”会向所选第三方 Provider 发出一次最小请求，可能产生极小 API 费用。新任务或重启 Codex 后使用新配置。

## Codex 可视化配置控制台

控制台不是直接暴露 `config.toml` 的键值编辑器。它把常用配置按“模型行为 / 权限与审批 / 上下文与 Memory / 工具与能力”分组，并为每项提供用途、推荐值、生效范围和风险说明。

- **模型行为**：推理强度、规划推理、推理摘要、回答详细度、人格，以及推理内容的隐藏/原始显示。
- **权限与审批**：沙箱模式、审批策略、审批复核者、工作区联网。`danger-full-access` 与 `never` 会显示明确风险，而不是伪装成普通偏好。
- **上下文与 Memory**：历史持久化、自动压缩阈值与计数范围、Memory 总开关、生成/使用 Memory、外部上下文下停用 Memory。
- **工具与能力**：Web Search、Apps、子 Agent、Goals、Hooks、统一执行器和 Fast mode。

读取时保留“未显式设置 / 跟随 Codex 默认值”的状态；保存 `null` 表示清除用户层覆盖，而不是写入一个猜测的默认值。控制台只提交实际发生变化的键，携带 `config/read` 返回的版本避免静默覆盖并发修改。保存前统一展示差异，写入前把这些键的旧值保存为一个非敏感恢复点；恢复同样通过官方 `config/batchWrite` 完成。

部分配置只影响新任务或下一回合，界面会按 Codex 的配置语义提示，不承诺所有设置热切换。

## Usage 指标

| 界面数据 | 类型 | 数据源 / 字段 | 展示与计算口径 | 时效与限制 |
|---|---|---|---|---|
| 套餐 | 官方直接值 | `account/read` → `planType` | 只做名称格式化，不推测套餐 | 取决于 App Server 返回 |
| 5 小时额度 | 官方直接值 | `account/rateLimits/read` → `primary.usedPercent`, `primary.resetsAt`, `windowDurationMins` | 百分比原样显示；剩余 = `100 - usedPercent`；倒计时由 `resetsAt - 当前时间` 得到 | 官方没返回窗口时显示“未返回”，不伪造无限 |
| 每周额度 | 官方直接值 | `account/rateLimits/read` → `secondary.*` | 与 5 小时额度相同 | 某些额度桶可能没有 secondary |
| Credits | 官方直接值 | `account/rateLimits/read` → `credits` | 显示是否存在、`unlimited` 和 `balance` | Credits 不是 Token，也不是 API 账单金额 |
| 今日总 Token | 本地推导，官方补位 | session `token_count`；无本地数据时使用官方当天 daily bucket | `总 Token = 输入 Token + 输出 Token` | 本地日志接近实时；官方日桶可能延迟 |
| 输入 Token | 本地推导 | session `token_count.input_tokens` | 请求输入总量，包含缓存命中部分 | 仅统计存在结构化 Token 事件的本机会话 |
| 缓存输入 | 本地推导 | session `token_count.cached_input_tokens` | 输入 Token 的子集，不再次计入总量 | 命中率 = 缓存输入 / 输入 |
| 缓存写入 | 本地直接值 | session `token_count.cache_write_input_tokens` | 仅在事件明确返回时单列展示；不自行加到总 Token | 多数 Codex Session 当前不返回该字段，列会自动隐藏 |
| 未缓存输入 | 本地推导 | 输入 Token、缓存输入 | `max(输入 - 缓存输入, 0)` | 用来观察真正重新处理的上下文 |
| 输出 Token | 本地推导 | session `token_count.output_tokens` | 与输入相加得到总 Token | 推理 Token 是输出子集，不再次相加 |
| 推理 Token | 本地推导 | session `token_count.reasoning_output_tokens` | 作为输出说明展示 | 不同模型/版本可能不返回 |
| 官方累计 Token | 官方直接值 | `account/usage/read` → `summary.lifetimeTokens` | 原样展示 | 是历史使用量，不是剩余额度 |
| 单日峰值 | 官方直接值 | `summary.peakDailyTokens` | 原样展示 | 官方账户统计 |
| 连续使用 | 官方直接值 | `currentStreakDays`, `longestStreakDays` | 原样展示 | 由官方按自然日计算 |
| 最长任务 | 官方直接值 | `summary.longestRunningTurnSec` | 格式化为时长 | 不是应用启动时长 |
| 日账本 | 本地推导 | 本地 session 日聚合 | 逐日列出未缓存输入、缓存读取、缓存写入、输出、本地总 Token、模型与估算成本 | 只显示存在本地结构化 Token 事件的日期 |
| 月账本 | 本地推导 | 上述本地日账本按自然月聚合 | 同月本地 Session 相加；点击月份可展开逐模型明细 | 第一个月和本月可能是不完整周期 |
| 项目 / 对话 / Turn 账本 | 本地推导 + 官方名称 | session `session_meta.cwd`、`task_started.turn_id`、`turn_context.model`、`token_count`；`thread/list.name` | 先按完整工作目录归属项目，再按 Session 和 Turn 汇总同一批 Token 事件与单价；点击对话或 Turn 进入执行追踪 | 只列出本机存在结构化 Token 事件的 Session；官方名称不可用时显示 Session ID |
| 账户视图对照 | 官方直接值 | `account/usage/read` → `dailyUsageBuckets` | 与相同日期/月的本地总量并列展示，并给出差额；绝不相加 | 官方桶和本地日志可能因同步时间、覆盖范围或统计口径不同 |

Usage 使用“概览 / 按日 / 按月 / 按项目 / 模型成本”五张独立报表。概览只保留今日精确拆分、官方额度、账户汇总和近一年活动；逐日明细统一进入“按日”报表，避免同一组数据在概览重复出现。日/月报默认展示可核账的本地 Session 明细，也可以切换趋势图，避免只能悬浮图表才能看到具体值。主表固定区分 **未缓存输入 / 缓存读取 / 缓存写入（存在时）/ 输出 / 本地总 Token / API 等价成本**，点击任意日期或月份可查看逐模型构成。

“按项目”不依赖执行追踪是否已经完成。首屏直接复用成本索引中的 Session ID、模型和 Token 数值，并用只读 `thread/list` 补充 `cwd` 与用户可见对话名，所以不会为了列项目而重扫全部历史。用户第一次展开某个旧对话时，只按需读取该 Session 的 `task_started`、`turn_context` 与 `token_count`，把 Turn 数值保存进 X-Ray 自己的 `codex-xray.sqlite`；后续直接复用。点击对话或 Turn 进入执行追踪时，如果目标尚未分析，应用只分析这一个 Session 一次，然后定位到目标 Turn。它不把用户消息正文写入成本或 Turn 索引。项目、对话和 Turn 使用与日/月账本相同的分支回放去重和自定义单价，因此已读取的各层之和可与本地总账核对。

Usage 启动缓存、成本文件索引、项目 Turn 与 Trace 文件索引位于同一个 `codex-xray.sqlite`。数据库使用 WAL。用量账本拆成 `usage_session_files`、`usage_session_turns`、`usage_token_events`；项目、对话和 Turn 直接查询同一组关系数据，不复制第二份项目明细。日期、项目、Session、Turn、模型和 Token 类型都可以直接用 SQL 查询。Trace 同步写入 `trace_sessions`、`trace_turns`、`trace_phase_events`、`trace_tool_events`、`trace_usage_events`，其中工具请求、Codex 执行结束和结果写回保留各自的来源行号与时间。Trace 仍按 Session 保留一份结构快照，用于不丢字段地快速还原完整 Timeline。刷新时只更新发生变化的 Session；Codex 的原始 Session JSONL 始终只读。

### 日/月账本与账户对照规则

1. 日/月主表只使用本地 Session 的结构化 Token 事件，保证输入、缓存、输出、模型与成本来自同一覆盖范围。
2. `未缓存输入 = max(输入 - 缓存读取, 0)`。
3. `本地总 Token` 采用 Session 事件返回的总量；在当前事件口径下通常等于 `输入 + 输出`，缓存读取已经包含在输入中，推理 Token 已经包含在输出中，均不重复相加。
4. 月账本只聚合同月的本地日记录；没有本地事件的空日期、空月份不制造 `0` 行。
5. 官方日桶单独作为“账户视图”展示。它用于发现同步时间、覆盖范围或统计口径差异，不会覆盖本地明细，也不会与本地总量相加。
6. API 等价成本只应用于同一批本地模型明细；未知模型明确标记为“未定价”。

## API 等价成本

API 等价成本回答的是：“如果同样的模型 Token 通过公开 Standard API 单价计价，大约价值多少？”它**不是** ChatGPT/Codex 订阅账单，也不代表实际扣款。

成本只根据本地 session 中带模型信息的 Token 明细计算，独立于日/月报表当前采用的 Token 来源。因此，当 Token 总量采用官方账户统计时，旁边的估算成本仍可能来自本地 session；二者覆盖范围不完全相同时，不应反推“平均每 Token 实际价格”。

计算过程：

1. 从本地 session 取模型名称、输入、缓存输入和输出 Token。
2. `未缓存输入 = 输入 - 缓存输入`。
3. 分模型应用 Codex X-Ray 内置的公开单价快照，或用户在“模型成本 → 单价设置”中保存的模型覆盖值：
   `成本 = 未缓存输入 × 输入单价 + 缓存输入 × 缓存单价 + 输出 × 输出单价`。
4. 无法识别模型或没有单价的 Token 计入“未定价 Token”，不套用通用价格。
5. 缓存预计节省 = 同批缓存输入按普通输入单价计算的金额 - 缓存输入金额。

价格单位统一为 **USD / 100 万 Token**。内置价格快照会显示日期；更新默认单价属于应用版本更新的一部分。自定义单价按“生效日期”保存为多个版本，事件使用其日期当时最近的有效版本，因此修改今天的单价不会重写历史月份。版本保存在 Codex X-Ray 自己的应用数据目录 `pricing-config.json`，不写入 `~/.codex`，也不改变 Codex、账号或 Provider 的真实计费配置。保存后会重新汇总日/月/总计、模型以及 Session/Turn 成本；Token 增量索引会被复用，不重新读取未变化的 Session 正文。

自定义值是模型的固定输入、缓存输入和输出价格，会覆盖该模型内置的常规价与长上下文阶梯价。恢复单个模型会重新使用内置阶梯；未知模型清除自定义值后恢复为“未定价”。这些金额始终是估算，不会读取或声称代表用户的实际账单。

## 执行解剖

执行解剖只保存还原真实调用过程所需的结构化聚合，不保存消息正文、完整补丁、读取到的文件内容或完整工具输出。为支持执行学习，它会保存限长且脱敏的命令、脚本和参数摘要。

### Session 详情层级

执行解剖工作台分为“目录”和“解剖”两步。进入页面时只调用官方 `thread/list` 完整分页建立项目/对话目录，不扫描全部 Session；用户明确点击“解剖这个对话”后，才只读解析该 Session：

1. **项目**：以 `thread/list.cwd` 的完整目录为稳定分组键，以目录末级名称作为项目名。
2. **对话**：优先显示 `thread/list.name`；没有官方名称时显示明确的时间型占位名，不读取首条消息生成标题。
3. **解剖状态**：`待解剖` 表示从未解析；`已解剖` 表示持久化索引与 Session 文件元数据一致；`已过期` 表示文件在上次解剖后发生变化，需要重新解剖。
4. **Session 汇总**：总 Token、回合数、工具调用、输入/缓存、输出/推理、上下文峰值/窗口、压缩次数和 API 等价成本。
5. **逐回合数据**：每个 turn 单独展示输入、未缓存输入、缓存读取、输出、推理输出、上下文峰值、上下文窗口占用、上下文变化、压缩、耗时和成本。
6. **真实结构化 Timeline**：将 `task_started`、assistant 的 `reasoning/message` 阶段、每个 `token_count`、`tool_search_call`、结构化工具调用/结果、`context_compacted` 和 `task_complete` 按 Session 中的时间顺序完整展示，不再截断为 80 个事件。
7. **LLM 事件**：每个 `token_count` 作为一次模型返回的 Token 结算记录并编号，展示模型、未缓存输入、缓存输入、输出、推理输出、总量、上下文窗口、相对上次调用的上下文变化、缓存命中率和当次 API 等价成本。`reasoning` 与 assistant `commentary/final_answer` 只显示阶段、分片数和记录字节数，不保存或展示正文。
8. **事件分类**：按结构化工具名、MCP namespace 和安全参数分为 LLM、MCP、CLI、Skill、文件、浏览器/自动化、子 Agent、上下文/回合和其他工具；分类仅用于筛选，界面始终保留原始工具名。
9. **MCP 事件**：展示完整工具名、由 `mcp__*` namespace 得到的 Server 名和脱敏后的参数键值摘要。
10. **调用审计详情**：每个工具事件可展开查看 `call_id`、原始 `response_item` 类型、精确开始/结束时间、递归脱敏后的输入树，以及结果类型、条目数、状态、退出码、耗时、chunk/session/cell ID 等白名单元数据。
11. **CLI / 自动化事件**：展示限长命令或脚本摘要、工作目录等非敏感参数、耗时、能够解析到的退出码，以及对应结果记录的 JSONL 字节数。结果记录大小不是终端真实输出字符数。
12. **Skill 事件**：当结构化调用明确读取 `SKILL.md` 时，显示 Skill 名和对应短路径。它表示该 Skill 指令被读取；Session 当前没有单独的“Skill 执行完成”事件。
13. **文件与补丁事件**：读取操作只保存可安全提取的短路径；`apply_patch` 只保存补丁涉及的文件名，不保存补丁正文。

折叠态参数摘要最多保留 12 个顶层字段；展开后的参数树最多递归 5 层、每个集合最多 20 项、每条预览最多 6,000 字符。字段名包含 `token`、`secret`、`password`、`authorization`、`credential`、`cookie`、`api_key` 或私钥/访问密钥语义时，值替换为 `[redacted]`。命令中的 Bearer、`sk-*` 和常见密钥赋值也会脱敏。结果只提取白名单元数据，Codex X-Ray 不把工具完整返回内容写入索引。

Codex 当前 Session 事件没有独立的 “Memory 用量” 字段。Codex X-Ray 不伪造该数字，而是展示可直接读取的相关量：输入峰值、模型上下文窗口、窗口占用比例、缓存读取和上下文压缩。

单对话分析只解析用户点选的一个 Session；项目分析则按目录逐个处理该项目中“未分析”或“已过期”的 Session，并提供进度、失败数与停止操作。两者都会把结构化聚合写进 Codex X-Ray 自己的增量索引，再次打开直接读取持久化结果。启动应用和刷新目录都不会触发项目分析，也不会调用模型或消耗 Codex 额度。

## 扩展使用

“环境诊断 → 扩展使用”完全来自上述已持久化的执行解剖索引，不会为了生成统计再扫描全部 Session：

1. 调用次数、失败、重复、结果记录体积、开始/结束时间来自结构化工具调用及其结果事件。
2. 项目、Session 和 Turn 覆盖由每个调用所在的 `cwd`、Session ID 和 Turn ID 去重计算。
3. 总耗时只累加同时存在开始与结束事件的调用；“可计时调用”明确显示分母，未结束调用不会被当成 0 毫秒拉低平均值。
4. MCP 以 Server 与原始工具名区分；Skill 只在明确读取 `SKILL.md` 时计入。CLI、浏览器、自动化、文件与子 Agent 使用同一套结构化事件分类。
5. Session 文件在解剖后变化会被标为“已过期”；旧结果仍可查看，但界面会提示统计可能少于最新实际值。
6. Codex Session 当前没有“单个工具调用消耗多少 Token”的因果字段。扩展使用页不把所在 Turn 的 Token 冒充为该工具的 Token，也不会据此计算虚假的单工具成本。
7. 点击某个调用入口会打开其最近一次持久化证据所在的 Session 和 Turn，并在存在 Call ID 时定位到对应工具请求；账本聚合仍使用全部已分析证据。

## 会话状态

当前版本读取官方 `thread/list.status`，并在它实际返回活动状态时采用：

- `active`：运行中。
- `active + waitingOnApproval`：等待审批。
- `active + waitingOnUserInput`：等待输入。
- `systemError`：失败。

由于 Codex X-Ray 的 App Server 与 Codex App 进程隔离，另一个进程内的 `activeFlags` 不保证可见，列表常会返回 `idle` 或 `notLoaded`。这时 Codex X-Ray 使用 `task_started`、`task_complete`、结构化失败和文件最近修改时间补充“运行 / 完成 / 中断 / 疑似失败”。等待审批和等待输入只在官方确实返回对应 flag 时显示，不由 Codex X-Ray 猜测。独立 App Server 不会附着或控制 Codex App 的运行中回合。

## 缓存与启动速度

- WebView `localStorage` 保存最近一次 Usage、成本和 Trace 聚合，页面可立即绘制。
- Codex X-Ray 应用数据目录保存同一份 Rust 侧快照，WebView 缓存丢失时回退。
- 成本索引使用文件元数据增量更新；未变化 session 不重新解析。
- Trace 首屏只刷新官方目录；不会因为打开页面而全量扫描 Session。
- 每次只分析用户点选的一个 Session。结果原子写入 Codex X-Ray 应用数据目录的增量索引，并同时保留在进程内存中。
- 后台刷新不会启动 Codex 任务，也不会消耗模型额度。

## 隐私与只读边界

Codex X-Ray：

- 不读取 `auth.json`；
- 不保存邮箱、提示词、回复正文、完整补丁、读取到的文件内容或完整工具输出；
- 只保存限长、脱敏的工具参数、命令和脚本摘要，用于用户主动打开的执行 Timeline；
- 不修改 session 或 Codex 数据库；
- 仅在用户于控制台完成差异预览并二次确认时，通过官方 `config/batchWrite` 更新用户选择的配置键；
- 不读取或保存 Provider API Key，只记录用户指定的环境变量名；
- 只把用户主动设置的模型单价写入自己的 `pricing-config.json`，不修改 Codex 计费或账号数据；
- 不代理或拦截 Codex 请求；
- 不上传本地分析数据；
- 自己的缓存和索引只写入 Codex X-Ray 应用数据目录。

## 官方参考

- [Codex App Server](https://learn.chatgpt.com/docs/app-server)
- [Codex 配置参考（含 `sqlite_home` 和 `$CODEX_HOME`）](https://learn.chatgpt.com/docs/config-file/config-reference)
- [阿里百炼 Responses API](https://help.aliyun.com/zh/model-studio/qwen-api-via-openai-responses)
- [火山方舟 Responses API](https://www.volcengine.com/docs/82379/1795150)
- [百度千帆 Responses API](https://cloud.baidu.com/doc/qianfan-docs/s/4mi400l1m)
- [MiniMax Codex 接入](https://platform.minimaxi.com/docs/token-plan/codex)
- [StepFun Responses API](https://platform.stepfun.com/docs/zh/api-reference/responses/responses-create)

`account/usage/read` 等账户字段以当前安装的 Codex App Server 生成 schema 为兼容依据。App Server 文档提示部分接口可能仍在演进，Codex X-Ray 会在兼容层中处理字段缺失。
