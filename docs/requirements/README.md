# 需求文档

## 写作规范

详见 [STANDARDS.md](STANDARDS.md)。

## 必读

理解 CloseClaw 的产品行为，先读 [message_flow](message_flow.md)（完整消息流）——它串联 IM 接入、身份映射、消息路由、会话确定、LLM 对话与出站发送，是产品核心业务链路；各模块文档在此基础上按功能域展开细节。

## 模块职责与需求归属

各模块按下方需求索引划分职责。每项需求由其所属功能模块完整定义，其他模块通过交叉引用关联，不重复定义。

配置项的行为语义、默认值、覆盖优先级与生效时点归消费该配置的功能模块；配置文件的组织、注册、校验、备份与重载机制归 config。配置项存放在哪里、是否以 Agent 为配置粒度，不改变其功能归属。

Agent 定义静态身份与配置档案的组合维度，不包办档案中各项能力的规则。工具与技能范围分别归 tools、skills，授权与权限例外归 permission；子 Session 的创建控制、Fork、层级与并发、超时及后续运行行为统一归 session。模型选择、人格加载与记忆规则分别归 llm、system_prompt、memory，agent 仅保留相关配置维度的说明与交叉引用。

## 需求索引

| 模块 | 简述 | 状态 |
|------|------|------|
| [agent](agent.md) | Agent 静态身份与配置档案组合、身份人格分离、初始创建与配置查询 | ✅
| [cli](cli.md) | 终端对话、daemon 管理、配置向导 | ✅
| [common](common.md) | 跨模块共享数据结构与接口契约（无独立功能） | ✅
| [config](config.md) | 多文件配置结构、Agent 注册清单、安全写入、配置重载、凭据隔离 | ✅
| [daemon](daemon.md) | 系统启动、优雅/强制关闭、系统级后台任务生命周期 | ✅
| [debug_log](debug_log.md) | 调试日志框架：链路追踪、分级过滤、存储轮转、隐私脱敏 | ✅
| [fake_llm](fake_llm.md) | 本地假 LLM 服务：场景驱动的确定性黑盒 E2E 测试 | ✅
| [gateway](gateway.md) | 多平台消息接入、斜杠指令分派、出站统一处理 | ✅
| [im_adapter](im_adapter.md) | IM 平台插件化适配、消息归一化、流式渲染 | ✅
| [im_adapter/feishu](im_adapter/feishu.md) | 飞书平台实现：入站接收、卡片渲染、平台工具 | ✅
| [llm](llm.md) | 模型选择、多供应商统一对话、流式输出、缓存优化、用量统计 | ✅
| [memory](memory.md) | Session 结束后自动挖掘记忆、实体体系 | ✅
| [message_flow](message_flow.md) | 产品核心业务链路（必读）：入站（IM → Agent）与出站（Agent → IM） | ✅
| [permission](permission.md) | 身份型访问控制、授权与权限例外、审批工作流、子 Session 权限继承 | ✅
| [platform](platform.md) | 跨平台进程与路径管理、终端自适应、系统差异隔离 | ✅
| [processor_chain](processor_chain.md) | 入站出站消息统一处理、展示等级过滤、调试日志 | ✅
| [session](session.md) | 对话持久化与恢复、压缩、子 Session 创建与委托、Fork、层级与并发、超时、消息排队 | ✅
| [skills](skills.md) | 技能发现与加载、可用范围、目录层级、条件激活、多 Agent 隔离 | ✅
| [slash](slash.md) | 斜杠指令体系：Session 管理、状态查询等 | ✅
| [system_prompt](system_prompt.md) | System Prompt 组装、缓存、Session 类型适配、动态指令 | ✅
| [tools](tools.md) | 工具注册与发现、工具范围、文件读写、命令执行、后台任务 | ✅
| [workflow](workflow.md) | 工作流定义、步骤引导执行、分支控制、中断续跑 | ✅
