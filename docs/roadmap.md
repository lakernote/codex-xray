# Codex X-Ray 路线图

## 产品边界

Codex X-Ray 是 Codex 的本地优先可视化助手与诊断工作台：

1. Usage：解释账户额度、Token 和 API 等价成本。
2. Trace：按项目与对话组织本地结构化证据，解释一次任务的成本、失败和上下文压力。
3. Control：把常用 Codex 配置与 Responses Provider 转成可解释、可预览、可恢复的 GUI。
4. Environment：把 CLI/App Server、路径、安全边界、Provider、MCP、Skills 与 Plugins 汇总成只读诊断页。

不读取 `auth.json`，不写 Codex session 或数据库，不代理请求。用户配置只在差异预览并二次确认后通过官方 `config/batchWrite` 执行。

## 0.1 — Usage、Trace 与桌面外壳

已实现：

- 官方 App Server 账户读取与独立 `sqlite_home`
- 5 小时 / 每周额度、重置、Credits、多额度桶
- 今日 Token 实时拆分、官方历史日桶、14/30 天和月聚合
- 项目 → 对话 → Turn 用量账本，与日/月账本共用去重和单价索引，可直接跳入执行追踪
- 易读数字、精确值、悬浮详情和完整指标说明
- Standard API 等价成本与缓存预计节省
- Usage / 成本 / Trace 双层启动缓存和增量索引
- fork / 子 Agent 重放前缀去重
- Trace 八类浪费/压力信号与会话证据面板
- 中英文切换
- 内置数据来源页与中英项目文档
- 项目 → 对话 → 回合 → 事件/工具的分析层级
- 官方 `thread/list` 对话名、目录与活动状态合并
- Provider 只读发现、国产 Responses 预设、变更预览、官方写入与可逆恢复点
- 模型行为、权限审批、上下文与 Memory、工具能力的中英文 GUI，含推荐设置、风险说明、版本校验和恢复点
- 系统托盘
- 主窗口关闭后驻留托盘
- 只读环境诊断：运行时、关键路径、独立 SQLite、当前行为、MCP 与扩展根目录
- 扩展使用基础账本：基于已按需解剖 Session 汇总 MCP、Skill、CLI 等调用的覆盖、耗时、失败、重复和结果记录体积
- 项目级增量分析队列：只处理未分析或已变化的 Session，显示进度与失败并允许停止
- 项目级增量分析在开始前确认范围，保留失败列表并支持只重试失败 Session
- 扩展调用入口可跳回最近一次真实 Session、Turn 与 Call
- Codex X-Ray 矢量应用图标与 macOS、Windows、iOS、Android 标准尺寸资源
- 任务状态页与托盘摘要；状态来源分为 App Server 与本地事件推断
- 自定义单价按生效日期版本化
- SQLite `quick_check`、外键、WAL 与损坏 Session 行诊断
- GitHub CI 与跨平台草稿 Release 工作流

## 0.2 — Trace 证据深化

- 回合时间线：输入增长、工具输出、压缩和最终结果的顺序
- 扩展使用账本继续增加完整的项目/Session 调用列表和导出
- 从“发现问题”进入“如何减少”的可执行建议
- 规则阈值按模型上下文窗口自适应
- 文件读取热图、失败工具排行和高成本回合排行
- 增量索引版本迁移与可清理的本地存储设置
- 精确区分父任务、子 Agent 增量和重放 Token

## 0.3 — 官方交互增强

- 评估官方 Hooks 提供的任务生命周期事件
- 继续验证跨客户端活动状态的时效性与兼容性
- 点击任务直接跳回 Codex 的官方 deep link 或 thread API（仅在官方公开稳定接口后）
- 对官方值、本地补位与 Codex X-Ray 推导持续分层标注

## 0.4 — 托盘与分发

- 托盘菜单内直接展示今日 Token、缓存命中与运行任务摘要
- 可选开机启动
- 配置正式仓库、更新签名、公证和应用内自动更新
- 匿名且明确可选的兼容性诊断
