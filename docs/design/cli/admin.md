# CLI Admin

## 概述

CLI Admin 是 CLI 的管理命令层，覆盖两类操作：不经 daemon 的本地操作（daemon 生命周期、配置、权限规则）与经管理接口的 daemon RPC（agent/skill 查询）。所有命令绕过消息链路——不经 Processor Chain、不经 Session、不经 Gateway 路由。

## 架构

Admin 命令通过 `closeclaw <command> [args]` 调用，参数解析后由 handler 函数按命令类型分派到两类：

- **本地操作**（不经 daemon，由 CLI 进程直接执行）：run — 启动 daemon 进程；stop — 终止 daemon 进程；config — 读/写配置文件；rule — 校验/列出权限规则（只读）
- **daemon RPC**（依赖 daemon 已运行）：agent — 查询 agent；skill — 查询 skill 列表

### 与斜杠指令的关系

部分斜杠指令和 Admin 命令功能重叠。区别在于：
- Admin 命令可操作的范围更广（启停 daemon、交互式配置向导）
- Admin 命令仅 Owner 可执行（CLI 渠道调用者默认为 Owner；不走 Permission 引擎——指 Admin 命令自身的执行授权不做权限判定，依赖 OS 层 shell 访问控制；rule 命令仅读取权限规则内容）
- Slash 指令需要 session 上下文，Admin 命令不需要

### 命令分类

**本地操作**：不经 daemon 管理接口，由 CLI 进程直接操作文件系统或进程。

run 命令启动 daemon 进程：启动前检测 PID 文件——存在且对应进程存活则拒绝启动并提示已有实例；存在但进程已不存在（异常残留）则清理后正常启动；daemon 自身的组件初始化与消息循环见 [daemon 模块](../daemon/README.md)。stop 命令经 [platform 进程接口](../platform/process.md) 读取 PID 文件、终止 daemon 进程、等待退出并清理 PID 文件；PID 文件不存在（daemon 未运行）时正常退出，不报错。

config 命令管理配置文件。setup 子命令启动交互式配置向导（详见 [LLM Provider 配置向导](../llm/provider-config-wizard.md)），引导用户选择 Provider、输入凭据、发现模型并写入配置，首次配置时引导创建初始 Agent（创建规则见 [agent §F5](../../requirements/agent.md)）。validate 子命令校验配置文件：按配置文件做 schema 级校验（字段/类型/必填），校验规则复用 [config 模块「校验规则」](../config/README.md)，不另建 schema。

rule 命令查看权限规则（只读——权限规则的修改通过斜杠指令完成，对应需求 [cli §F7](../../requirements/cli.md)）：check 子命令校验单条规则语法，接受内联 JSON 字符串或文件路径（自动探测：以 `/`、`./`、`../` 开头或以 `.json` 结尾视为文件路径，否则视为内联 JSON），规则语法见 [权限模块](../permission/README.md)；list 子命令列出权限规则（规则来源、维度与存储见 [权限模块](../permission/README.md)）。

**daemon RPC**：依赖 daemon 已运行，通过管理接口查询 daemon 状态。

agent 命令查询 agent 实例（列表、详情，只读）。Agent 的创建经 `config setup` 向导（见 §命令分类）、增删经 config 配置（注册清单 agents.json + 对应 agent 配置目录）承载，CLI 的「Agent 管理」即只读查询口径。skill 命令查询 skill 列表（只读）：skill list 返回系统级已加载技能（含编译期内置技能，不按 Agent 维度过滤）。技能即插即用与变更生效边界见 [skills 模块](../skills/README.md)——技能文件放入目录后自下次 System Prompt 组装时生效，无运行时热加载路径。

agent 查询的返回内容按命令区分粒度：

- `agent list`：返回摘要列表，每项含 id、name、默认模型（model.default）
- `agent info <id>`：返回该 agent 的完整配置档案（全部字段以 [agent 配置字段](../agent/agent-config.md) 为准）。`<id>` 为 agent 配置的 `id` 字段——系统内唯一标识，`agent list` 返回的 id 即查询键；`name` 为显示名称，不保证唯一，不作为查询键。权限基线不在返回集内——权限与 agent 配置独立存储、独立变更（权限规则的查看见 rule 命令）。查询不存在的 agent ID 返回错误

## 数据流

1. `closeclaw <command> [args]` 输入 → 参数解析 → 确定命令类型（本地操作 / daemon RPC）
2. 按命令类型执行：
   - **本地操作**：
     1. run：检测 PID 文件（存活实例 → 拒绝启动；异常残留 → 清理）→ 启动 daemon 子进程 → 等待 daemon 就绪（管理接口可达）后返回
     2. stop：读 PID 文件 → 终止进程 → 清理 PID 文件；PID 文件不存在 → 正常退出
     3. config setup：交互式向导（选 Provider、输入凭据）→ 拉取模型列表 → 用户选择 → 写入配置 → 首次配置时创建初始 Agent
     4. config validate：读文件 → 语法解析（解析失败即报错）→ 按 [config 模块校验规则](../config/README.md) 逐配置文件校验 → 汇总输出所有问题
     5. rule check：解析输入（内联 JSON 或文件路径）→ 校验单条规则语法 → 输出结果；rule list：读规则文件 → 列出已有规则 → 输出结果
   - **daemon RPC**（发送 RPC → daemon 执行 → 返回结果）：
     1. agent list：daemon 遍历 AgentRegistry → 返回摘要列表
     2. agent info：daemon 按 ID 查询 AgentRegistry → 返回完整配置档案，未命中返回错误
     3. skill list：daemon 查询 Skills Registry → 返回已加载技能列表
3. 结果输出到 stdout（格式化文本 / 表格 / JSON）；run/stop 以状态提示与退出码反馈结果，不产出数据型输出

## 模块关系

- **上游**：操作系统命令行参数、platform 模块（提供 [进程生命周期/PID 文件](../platform/process.md)、[路径处理](../platform/file-path.md) 等 OS 抽象）、daemon（Admin 层经管理接口发起 daemon RPC 并消费其返回的查询结果——daemon README 登记 Admin RPC Server，Unix domain socket 管理服务）
- **下游**：Daemon（run 创建 daemon 实例，stop 终止 daemon 进程，agent/skill 经管理接口查询 daemon 状态）、Config 模块（config setup 写 models.json 和凭据文件；config validate 按 schema 级校验配置）、Agent 模块（config setup 首次配置时创建初始 Agent）、Permission 模块（rule 命令只读查看权限规则）、LLM 模块（config setup 向导中调用模型发现能力）
- **与模块内其他子功能**：与 CLI Chat 平级、同属 CLI 模块但互不调用——Chat 走消息链路，Admin 绕过消息链路；两者按需消费 platform 模块的不同子能力（Chat 用终端能力，Admin 用进程/PID/信号），配置数据的读写经 config 模块完成，不直接使用 platform 的配置目录接口
- **无关**：Gateway（Admin 命令不经 Gateway 路由）、Processor Chain（Admin 命令不经消息处理链）、Session（Admin 命令无 session 上下文）
