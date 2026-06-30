# 产品文档

模块级产品说明，讲清楚模块的职责、架构、数据流、模块间交互。

| 模块 | 简述 |
|------|------|
| [agent](agent/README.md) | Agent 配置档案（目录式、注册清单、permissions 独立）、spawn 协调与权限沿链路继承 |
| [common](common/README.md) | 跨模块共享的纯数据结构和核心 trait 定义——是业务模块间的唯一接口依赖层 |
| [cli](cli/README.md) | 命令行接口模块：CLI Chat（terminal 渠道的 IMPlugin 实现）和 CLI Admin（daemon 管理命令） |
| [config](config/README.md) | CloseClaw 运行时配置管理：多文件拆分、ConfigManager 统一读写、备份回退、凭据分离、Agent 注册清单与多级加载、热重载 |
| [daemon](daemon/README.md) | 进程入口和组件胶水层：依赖驱动启动、后台任务管理、graceful/forceful 双模关闭 |
| [gateway](gateway/README.md) | 消息路由中枢：管理 IM 插件、调度 Processor Chain、路由决策（slash vs normal）、选择平台插件完成出站渲染与发送 |
| [llm](llm/README.md) | 统一多供应商 LLM 调用与模型发现：五层分离架构、Provider/Protocol/Interpreter 分层、内容块归一化、缓存适配、Provider 配置向导 |
| [mode](mode/README.md) | Session 运行模式管理：Plan Mode（规划/执行分离、双路径、审批栅栏）和 Auto Mode（连续自主执行），通过工具过滤和权限边界约束 agent 行为 |
| [permission](permission/README.md) | 系统级身份型访问控制：交集模型、七类权限维度、审批工作流与配置管理 |
| [platform](platform/README.md) | 操作系统抽象层：进程管理、信号处理、配置目录、终端 I/O 的平台差异封装 |
| [processor_chain](processor_chain/README.md) | 统一出入站消息处理：入站纯变换链（审计日志、session_key 计算、文本标准化）、出站 DSL 解析、ContentBlock[] 传递 |
| [im_adapter](im_adapter/README.md) | IM 平台插件化适配框架：IMPlugin trait 统一接口，每个平台一个插件（含 Adapter + Renderer），通用代码渲染和流式渲染能力 |
| [session](session/README.md) | Agent 会话生命周期管理：session_key 路由映射与解析、持久化（创建、压缩、归档、清理）、执行层（三维执行状态、级联停止、后台结果注入） |
| [slash](slash/README.md) | 斜杠指令系统：Gateway 层拦截、统一分派、Handler 执行，不进入 LLM 对话流程 |
| [skills](skills/README.md) | Agent 可复用能力插件体系：磁盘即插即用、五层优先级、frontmatter 配置驱动、双执行模式 |
| [memory](memory/README.md) | Agent 长期记忆体系：两段式会话挖掘、实体级升格浓缩、SQLite 索引实时搜索注入，构建跨 session 概念网络 |
| [system_prompt](system_prompt/README.md) | 每次 API 调用的固定前缀：静态/动态层划分、Section 类型、缓存策略、构建与注入入口 |
| [tools](tools/README.md) | Agent 能力层：工具注册基础设施（ToolRegistry 注册中心、索引构建、工具发现）、文件操作工具（Read/Write/Edit）、Bash 工具、后台任务、多工具并行调度 |
| [workflow](workflow/README.md) | Workflow Engine：流程控制层，将多步骤流程从 prompt 驱动转为 Engine 驱动的状态机执行，管理 goal→verify→jump 三阶段协议和步骤跳转 |
