<p align="center">
  <img src="src-tauri/icons/icon.svg" width="96" height="96" alt="Codex X-Ray 图标" />
</p>

<h1 align="center">Codex X-Ray</h1>

<p align="center">
  看清 Codex 的用量、成本、上下文与每一步执行。
</p>

<p align="center">
  <a href="README.md">English</a> · <strong>简体中文</strong>
</p>

<p align="center">
  <a href="https://github.com/lakernote/codex-xray/actions/workflows/ci.yml"><img src="https://github.com/lakernote/codex-xray/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="https://github.com/lakernote/codex-xray/releases"><img src="https://img.shields.io/github/v/release/lakernote/codex-xray?include_prereleases&amp;sort=semver&amp;label=release" alt="GitHub Release" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-2f4f20.svg" alt="MIT License" /></a>
</p>

Codex X-Ray 是 Codex 的用量与执行分析工具。它展示额度和 Token，按项目、对话和 Turn 统计成本，并还原 LLM 与工具调用的执行 Timeline。

> [!IMPORTANT]
> Codex X-Ray 是非官方开源项目，与 OpenAI 无隶属或背书关系。

## 下载

预览版安装包发布在 [GitHub Releases](https://github.com/lakernote/codex-xray/releases)，根据文件名后缀选择：

| 系统 | 安装包 |
| --- | --- |
| macOS · Apple 芯片 | `_aarch64.dmg` |
| macOS · Intel | `_x64.dmg` |
| Windows · 64 位 | `_x64-setup.exe` |
| Ubuntu / Debian · 64 位 | `_amd64.deb` |
| Fedora / RHEL · 64 位 | `.x86_64.rpm` |

当前预览版尚未使用可信发布者证书签名或公证，操作系统可能显示“未知开发者”警告。使用前需要已经安装并登录 Codex。“Source code”压缩包是源码快照，不是安装包。

应用每天最多检查一次 GitHub Releases。发现新版本后可以忽略该版本，或打开下载页面；Codex X-Ray 不会自动下载或安装更新。

## 界面

### 用量概览

官方额度与本地 Session 用量分开呈现，同时展示输入、缓存、输出、年度活动和 API 等价成本。

![Codex X-Ray 用量概览](docs/assets/usage-overview.zh-CN.png)

### 项目、对话与回合账本

按原始工作目录组织项目，继续下钻到对话和 Turn，核对 Token 与成本去向。

![Codex X-Ray 项目用量账本](docs/assets/project-usage.zh-CN.png)

### 真实执行 Timeline

按 Session 中的顺序解释本地准备、用户输入、LLM 返回、工具请求、Codex 执行、结果写回、Token 结算和回合结束。

![Codex X-Ray 执行 Timeline](docs/assets/execution-trace.zh-CN.png)

以上截图全部由虚构项目和模拟 Session 数据生成，不包含真实用户路径、对话、账号或密钥。

## 现在能做什么

### 用量

- 展示 Codex 官方返回的账号、额度窗口、重置时间、Credits、累计用量、单日峰值和连续使用天数；官方没有返回的字段不会猜测。
- 建立今日、按日、按月、按模型和按项目的本地用量账本，分别统计未缓存输入、缓存读取、输出、Reasoning、总 Token 与 API 等价成本。
- 从项目继续下钻到对话和 Turn，定位每项工作的 Token 消耗与估算成本。
- 提供近一年活动热力图和按生效日期配置的模型单价；本地汇总结果使用 SQLite 增量索引。

### 执行追踪

- 按工作目录 → 对话 → Turn 组织 Codex Session。
- 按真实事件顺序还原本地准备、用户输入、LLM 返回、工具请求、Codex 执行、结果写回、Token 结算、上下文压缩和回合完成。
- 识别 CLI 命令、MCP、Skill、浏览器、自动化与子 Agent；在原始数据存在时展示参数、结果、耗时、Token、上下文占用、缓存命中率和来源行号。
- 由用户选择对话后按需分析，并把分析结果保存在 X-Ray 自己的 SQLite 索引中；不会修改原始 Session 文件。

### 控制台

- 切换原生 Responses Provider 或 OpenAI Chat Completions 兼容 Provider。内置 OpenAI 与常见厂商预设，也可自定义 Base URL、模型、上下文窗口和协议。
- Chat Completions Provider 通过 X-Ray 本机兼容桥接入，把文本、流式响应、函数调用、工具结果和用量转换成 Codex 需要的 Responses 结构。
- Provider Key 可保存在操作系统凭据库，也可引用环境变量，不会明文写入 `config.toml`。
- 可视化配置模型、Reasoning、输出详细度、Personality、审批、沙箱、联网、历史、Memory、压缩、工具、App、子 Agent、Goal 与 Hook，并解释各设置的作用。
- 应用配置前显示准确差异，确认后通过 Codex App Server 写入，同时保留可恢复的上一状态。
- 显示检测到的 Codex CLI 与 App Server 版本，并可打开或复制配置、Session、Skills、Plugins 和 X-Ray SQLite 索引目录。

### 桌面体验

- 支持中文/英文、亮色/暗色主题。
- 关闭主窗口后，应用和正在使用的 Chat 兼容桥会留在系统托盘。
- 每天最多检查一次 GitHub Releases；发现新版本时可以稍后处理、忽略该版本或打开页面手动下载，应用不会自动安装更新。

## 数据如何流动

```text
Codex App Server ──账户、额度、对话目录、配置──┐
                                                ├─ Rust 本地分析 ─ SQLite 索引 ─ React 界面
$CODEX_HOME/sessions ──只读 JSONL 事件─────────┘

可选 Chat Provider：Codex Responses 请求 ─ X-Ray 本机桥 ─ 厂商 /chat/completions
```

- 官方账户值保持原始口径；本地推导和成本估算会明确标注。
- 原始 Codex Session、数据库和任务内容始终只读。
- 对话只在用户选择后分析；打开目录不会自动解析全部历史。
- 索引存放在 Codex X-Ray 自己的应用数据目录，并使用 SQLite WAL 增量更新。

## 隐私与安全

- 不读取 `auth.json`，不上传本地分析数据。原生 Responses Provider 由 Codex 直连；只有用户在控制台明确选择的 Chat Completions Provider 会经过 X-Ray 本机兼容桥。
- 执行详情会按需从原始 Session 显示用户消息、助手消息和可读摘要；这些正文不写入 SQLite 索引。
- SQLite 只保存用量、结构化阶段、来源行号，以及限长且脱敏的命令、参数和结果元数据；不保存完整工具输出、完整补丁或读取到的文件内容。
- Provider Key 可保存到 macOS 钥匙串、Windows 凭据管理器或 Linux Secret Service。Codex 通过官方命令式认证按需读取；Key 不写入 `config.toml`、SQLite、日志或进程参数。本机 Chat 桥只转发当前 Provider 的 Key，并忽略无关的入站认证信息。仍可选择环境变量认证。
- 配置修改必须先查看差异并再次确认，同时保留可恢复的上一状态。

更完整的字段来源、计算公式和边界见[中文数据说明](docs/data-sources.zh-CN.md)。安全问题请阅读 [SECURITY.md](SECURITY.md)。

## 从源码运行

开发环境需要：

- Node.js 22（最低 18）
- Rust stable
- 已安装并登录的 Codex

```bash
git clone https://github.com/lakernote/codex-xray.git
cd codex-xray
npm ci
npm run tauri dev
```

构建与验证：

```bash
npm run version:check
npm run check
npm run build
npm run test:rust
npm run tauri build
```

如果 `codex` 不在 `PATH`，应用会尝试检测 Codex/ChatGPT App 的内置 CLI；也可以通过 `CODEX_BIN` 指定可执行文件。

## 当前边界

- API 等价成本用于比较 Token 价值，不是 ChatGPT/Codex 订阅账单或实际扣款。
- 独立 App Server 无法保证看到另一个 Codex App 进程的全部瞬时状态；界面会区分官方状态与本地事件推断。
- Codex App Server 与 Session 格式仍可能变化，兼容性以当前安装版本为准。
- Chat 兼容桥转换文本、流式输出、函数工具、工具结果和 Token 用量；原生 Web Search、加密 Reasoning、服务端压缩等 Responses 专属能力不会被转换。
- 使用 Chat Provider 时需要保持 Codex X-Ray 运行；关闭主窗口后应用会留在系统托盘。
- 当前主要在 macOS 上验证；发布流程覆盖 Windows、Linux 和 macOS。

## License

[MIT](LICENSE)
