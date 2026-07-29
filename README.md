# Codex X-Ray

Codex X-Ray 是一个本地优先的 Codex 可视化助手与诊断工作台。它把额度、Token、成本、上下文、执行过程、运行环境和配置拆成可核对的事实。

English summary: Codex X-Ray is a local-first workbench for Codex usage, execution inspection, environment diagnosis, and safe controls.

> Codex X-Ray 是独立的非官方项目，与 OpenAI 无隶属或背书关系。

## 产品定位

Codex X-Ray 不照抄 Codex 官方已经做好的页面，而是围绕三个官方客户端没有必要深入做的本地问题：

- **看清用了什么**：每天、每月、模型、项目、会话和回合的 Token 与 API 等价成本。
- **看懂发生了什么**：按项目、对话和回合解剖 LLM、Token、上下文、MCP、Skill、CLI 与工具调用。
- **更容易正确配置**：把 Provider、安全边界、Memory、上下文和扩展状态变成可解释、可预览、可恢复的 GUI。

“庖丁解牛”是执行解剖能力的隐喻，不替代 Codex X-Ray 品牌名。完整方向与技术边界见 [产品方向与技术架构](docs/product-direction.zh-CN.md)。

## 已实现 / Implemented

- 中英文界面切换，语言选择会持久化到本机
- 官方 5 小时 / 每周额度、重置倒计时、Credits；Spark 等独立模型桶作为次级信息展示
- 今日输入、缓存输入、未缓存输入、输出和推理 Token
- Usage 按“概览 / 按日 / 按月 / 按项目 / 模型成本”拆分；日/月默认展示本地 Session 明细，可切换趋势图
- 日/月账本逐行展示未缓存输入、缓存读取、缓存写入（存在时）、输出、本地总 Token 和 API 等价成本；点击周期可展开逐模型明细
- 项目账本直接复用本地成本增量索引，先秒级列出 `cwd → 对话` Token 与 API 等价成本；展开对话时按需读取并持久化该 Session 的 Turn，点击 Turn 会按需分析并直达对应执行追踪
- 官方账户统计作为独立“账户视图”对照，不与本地 Session 总量混加；界面直接显示两者差额
- 官方累计、单日峰值、连续使用和最长任务
- 按模型、缓存输入和输出估算公开 Standard API 等价成本
- 执行解剖工作台：首屏只按官方 `cwd` 列“项目 → 对话”；既可分析单个对话，也可按项目增量分析未处理或已变化的对话，显示进度、失败数并支持停止；启动时不会自动扫描全部历史
- 执行解剖持久化增量索引：显示待解剖 / 已解剖 / 已过期；逐回合展示 Token、上下文峰值/窗口、缓存、压缩、耗时、成本和按时间排序的真实结构化事件
- Timeline 可按 LLM、MCP、CLI、Skill、文件、自动化、子 Agent 和上下文筛选；LLM 调用逐次显示上下文变化、缓存命中与成本，工具调用可展开查看 Call ID、原始事件类型、精确起止时间、递归脱敏参数树和安全结果元数据
- 扩展使用账本：汇总已按需解剖 Session 中 MCP、Skill、CLI、浏览器、自动化和子 Agent 的调用次数、覆盖项目/对话、耗时、失败、重复与结果记录体积；点击调用入口可返回最近一次真实 Session、Turn 与 Call
- Provider 控制台：通过 Codex 官方 `config/read` 发现当前模型与 Provider；内置 OpenAI、Qwen、豆包、千帆托管 GLM/DeepSeek、MiniMax、StepFun 和自定义 Responses 预设
- Provider 变更必须先预览再确认，通过官方 `config/batchWrite` 保存；每次切换前保存恢复点，可一键恢复或再次切回
- Codex 控制台：把模型行为、审批与沙箱、联网、上下文压缩、历史、Memory、Web Search、子 Agent、Hooks 等常用配置做成中英文 GUI；危险选项给出具体影响，保存前统一显示差异，可一键恢复
- OpenAI 模型选择来自官方 `model/list` 动态目录，同时允许手动输入第三方 Provider 的精确模型 ID，不把模型列表写死在前端
- 数据来源页：每个数字的来源、公式、延迟与可信边界
- 环境诊断：展示 Codex CLI/App Server、CODEX_HOME、配置与 Session 路径、X-Ray 分析数据库、Provider、沙箱/审批、MCP 与本机扩展目录；不读取凭据值
- 应用图标：用三层剖面切片表达项目、Session、Turn 与调用层级，中间分析层被点亮；已生成 macOS、Windows、iOS 和 Android 标准尺寸资源
- 系统托盘：恢复主窗口、退出
- Usage / 成本 / Trace 启动快照；先显示缓存，再后台增量刷新
- 任务状态页：区分运行、等待审批、等待输入、失败/中断和最近完成；标明状态来自 App Server 还是本地事件推断，点击进入对应执行追踪
- 自定义模型单价按生效日期保存版本；历史事件继续使用当时版本，修改今天的价格不会重写过去月份
- Provider 可先执行一次最小 Responses 连通测试；结果显示 HTTP 状态和耗时，第三方测试可能产生极小 API 费用
- 托盘在任务目录读取后显示运行、等待和需处理数量

## 数据与隐私边界

- 官方账户数据通过当前安装的 `codex app-server` 的只读账户方法获取。
- 今日 Token、成本和 Trace 只读解析 `$CODEX_HOME/sessions/**/*.jsonl` 的结构化事件。
- Codex X-Ray 不读取 `auth.json`，不保存邮箱、消息正文、完整补丁或完整工具输出；Trace 明细只持久化限长且脱敏的命令、脚本和参数摘要。
- 不修改 Codex session 或数据库，也不代理 Codex 请求。只有用户在控制台查看差异并明确确认后，才通过 Codex 官方 `config/batchWrite` 更新所选的用户配置项。
- Provider 配置只保存环境变量名，不持久化 API Key。只有用户点击“测试连接”时，后端才从指定环境变量临时读取 Key 并发起一次最小 Responses 请求；Key 不返回前端、不写日志、不进入进程参数，测试结束后即丢弃。
- Codex X-Ray 启动的 App Server 使用独立 `sqlite_home`，不影响 Codex App 正在运行的 App Server。
- Codex X-Ray 自己的 Usage 缓存、成本文件索引、项目 Turn 与 Trace 索引统一保存在 `codex-xray.sqlite`，使用 WAL。Session、Turn、Token、阶段和工具调用都有可直接查询的关系表；Trace 另保留逐 Session 的结构快照用于快速还原完整 Timeline，不再把用量账本写成整块 JSON。
- 对话名和 `cwd` 来自官方 `thread/list`。独立 App Server 若返回活动 flags 就直接采用；另一个 Codex App 进程内的状态不可见时，运行/完成/中断由本地结构化事件补位，不伪装成官方实时状态。
- API 等价成本用于比较资源价值，不是 ChatGPT/Codex 订阅账单或实际扣款。

完整口径：

- [中文数据来源说明](docs/data-sources.zh-CN.md)
- [English data source guide](docs/data-sources.en.md)

官方参考：

- [Codex App Server](https://learn.chatgpt.com/docs/app-server)
- [Codex configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference)

## 开发 / Development

要求 Node.js 18+、Rust stable，以及已经登录的 Codex。

```bash
npm install
npm run tauri dev
```

验证并构建：

```bash
npm run check
npm run build
npm run test:rust
npm run tauri build
```

macOS 产物：

- `src-tauri/target/release/bundle/macos/Codex X-Ray.app`
- `src-tauri/target/release/bundle/dmg/Codex X-Ray_0.1.0_aarch64.dmg`

如 Codex 不在 `PATH`，Codex X-Ray 会检测 Codex/ChatGPT App 内置路径，也可设置 `CODEX_BIN` 指向 CLI 可执行文件。

## 开源与发布

- 贡献约定见 [CONTRIBUTING.md](CONTRIBUTING.md)。
- 版本、GitHub Release、签名与自动更新准备见 [docs/releasing.md](docs/releasing.md)。
- 当前仓库已包含 CI 与跨平台草稿 Release 工作流；正式自动更新仍需仓库地址、更新签名公钥以及各平台签名/公证凭据，未配置前不会伪装成可用。
