# 需求文档

记录用户（owner）的需求、使用场景和日常使用习惯。每条需求来自 owner 本人或与 owner 共同进行的调研商讨。

设计文档是需求与代码之间的桥梁，**必须遵从需求文档**。

## 写作规范

详见 [STANDARDS.md](STANDARDS.md)。

## 需求索引

| 模块 | 简述 | 状态 |
|------|------|------|
| [agent](agent.md) | Agent 配置档案、身份人格分离、Spawn 创建与控制 | ✅
| [cli](cli.md) | 终端对话、daemon 管理、配置向导 | ✅
| [common](common.md) | 跨模块共享数据结构与接口契约（无独立功能） | ✅
| [config](config.md) | 多文件配置结构、安全写入、热重载、凭据隔离 | ✅
| [daemon](daemon.md) | 系统启动、优雅/强制关闭、后台任务生命周期 | ✅
| [debug_log](debug_log.md) | 调试日志框架：链路追踪、分级过滤、存储轮转、隐私脱敏 | ✅
| [gateway](gateway.md) | 多平台消息接入、斜杠指令分派、出站统一处理 | ✅
| [im_adapter](im_adapter.md) | IM 平台插件化适配、消息归一化、流式渲染 | ✅
| [im_adapter/feishu](im_adapter/feishu.md) | 飞书平台实现：入站接收、卡片渲染、交互按钮 | ✅
| [llm](llm.md) | 多供应商统一对话、流式输出、缓存优化、用量统计 | ✅
| [memory](memory.md) | 会话后自动挖掘记忆、实体体系、升格为行为规则 | ✅
| [mode](mode.md) | Plan Mode / Auto Mode、plan 文件、执行与失败处理 | ✅
| [permission](permission.md) | 身份型访问控制、审批工作流、子 Agent 权限继承 | ✅
| [platform](platform.md) | 跨平台进程与路径管理、终端自适应、系统差异隔离 | ✅
| [processor_chain](processor_chain.md) | 入站出站消息统一处理、DSL 交互指令、审计日志 | ✅
| [session](session.md) | 对话持久化与恢复、压缩、子 Session 委托、消息排队 | ✅
| [skills](skills.md) | 技能即插即用、目录层级、条件激活、多 Agent 隔离 | ✅
| [slash](slash.md) | 斜杠指令体系：模式切换、会话管理、状态查询等 | ✅
| [system_prompt](system_prompt.md) | System Prompt 组装、缓存、会话类型适配、动态指令 | ✅
| [tools](tools.md) | 工具注册与发现、文件读写、命令执行、后台任务 | ✅
| [workflow](workflow.md) | 工作流定义、步骤引导执行、分支控制、中断续跑 | ✅
