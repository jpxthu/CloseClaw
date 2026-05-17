# 产品文档

模块级产品说明，讲清楚模块的职责、架构、数据流、模块间交互。

| 模块 | 简述 |
|------|------|
| [agent](agent/README.md) | Agent 配置档案管理、spawn 协调与权限沿链路继承 |
| [config](config/README.md) | CloseClaw 运行时配置管理：多文件拆分、ConfigManager 统一读写、备份回退、凭据分离、Agent 独立配置、热重载 |
| [llm](llm/README.md) | 统一多供应商 LLM 调用：五层分离架构、Provider/Protocol/Interpreter 分层、内容块归一化、缓存适配 |
| [permission](permission/README.md) | 系统级身份型访问控制：交集模型、七类权限维度、审批工作流与配置管理 |
| [processor_chain](processor_chain/README.md) | 消息出站处理与平台渲染：DSL 解析、结构化内容块渲染、流式输出、代码高亮 |
| [session](session/README.md) | Agent 会话生命周期管理：创建、持久化、压缩、归档与清理 |
| [slash](slash/README.md) | 斜杠指令系统：Gateway 层拦截、统一分派、Handler 执行，不进入 LLM 对话流程 |
| [skills](skills/README.md) | Agent 可复用能力插件体系：磁盘即插即用、五层优先级、frontmatter 配置驱动、双执行模式 |
| [system-prompt](system-prompt/README.md) | 每次 API 调用的固定前缀：静态/动态层划分、Section 类型、缓存策略、构建与注入入口 |
| [tools](tools/README.md) | Agent 能力层：两级索引工具体系、内建工具、平台工具、Bash 工具、提示词注入 |
