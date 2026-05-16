# 产品文档

模块级产品说明，讲清楚模块的职责、架构、数据流、模块间交互。

| 模块 | 简述 |
|------|------|
| [llm](llm/README.md) | 统一多供应商 LLM 调用：五层分离架构、Provider/Protocol/Interpreter 分层、内容块归一化、缓存适配 |
| [permission](permission/README.md) | 系统级身份型访问控制：交集模型、七类权限维度、审批工作流与配置管理 |
| [processor_chain](processor_chain/README.md) | 消息出站处理与平台渲染：DSL 解析、Renderer 跨平台渲染框架 |
| [session](session/README.md) | Agent 会话生命周期管理：创建、持久化、压缩、归档与清理 |
| [skills](skills/README.md) | Agent 可复用能力插件体系：磁盘即插即用、五层优先级、frontmatter 配置驱动、双执行模式 |
| [tools](tools/README.md) | Agent 能力层：两级索引工具体系、内建工具、平台工具、Bash 工具、提示词注入 |
