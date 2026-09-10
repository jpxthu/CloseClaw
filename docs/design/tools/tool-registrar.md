# ToolRegistrar

## 概述

ToolRegistrar 是工具注册能力的统一 trait，定义在 [common 模块](../common/core-traits.md#toolregistrar)，抽象各模块「我能注册工具」的接口契约。Tools 模块收集所有 ToolRegistrar 实现者、按优先级数值升序依次调用其注册方法，完成全局工具编排；Mode 执行触发工具、Workflow 工具属系统级例外，不经此 trait。

## 架构

### Trait 接口

ToolRegistrar 的完整接口定义见 [common/core-traits.md](../common/core-traits.md#toolregistrar)。本文档聚焦 Tools 模块对 Registrar 的编排逻辑。

### 编排流程

Registrar 以「工具分组」为单位注册——注册一个分组即注册该组的全部工具。

1. Tools 模块启动初始化，收集所有 ToolRegistrar 实现者
2. 按优先级数值升序排序
3. 依次调用各 Registrar 的注册方法，将其工具分组写入 ToolRegistry
4. Mode 执行触发工具、Workflow 工具在冻结之前另行注册（独立于上述 Registrar 调用链）
5. 全部注册完成，ToolRegistry 冻结，进入运行态

### 四个标准 Registrar

| Registrar | 优先级 | 所属模块 | 注册的工具分组 |
|-----------|--------|---------|--------------|
| CoreToolsRegistrar | 1 | tools | bash、file_ops、git_ops、meta |
| SessionToolsRegistrar | 2 | session | sessions |
| SkillsToolsRegistrar | 3 | skills | skills |
| ImAdapterToolsRegistrar | 4 | im_adapter | feishu_im、feishu_calendar、feishu_task、feishu_bitable、feishu_doc、feishu_drive、feishu_sheet |

优先级数值决定调用顺序；新增工具提供模块时选择合适的优先级值即可加入编排链，Tools 模块无需修改。

Mode 的执行触发工具、Workflow 工具在 ToolRegistry 初始化阶段、冻结之前注册，不经上述四个标准 Registrar 编排（见 [mode/execution.md](../mode/execution.md)、[workflow-tools.md](../workflow/workflow-tools.md)）。

### 冻结语义

全部 Registrar 及系统级工具注册完成后，ToolRegistry 冻结进入运行态：冻结后不再接受新注册，仅对外提供详情查询、分组查询与索引构建。

### 补充规则

- **优先级重复**：允许多个 Registrar 使用相同优先级，同等优先级下注册顺序不保证

## 数据流

1. 系统启动，Tools 模块收集所有 ToolRegistrar 实现者，按优先级数值升序排序
2. 依次调用各 Registrar 的注册方法，将其工具分组写入 ToolRegistry：CoreToolsRegistrar -> bash/file_ops/git_ops/meta；SessionToolsRegistrar -> sessions；SkillsToolsRegistrar -> skills；ImAdapterToolsRegistrar -> feishu_im、feishu_calendar、feishu_task、feishu_bitable、feishu_doc、feishu_drive、feishu_sheet
3. Mode 执行触发工具、Workflow 工具在冻结之前另行注册
4. 全部注册完成，ToolRegistry 冻结，进入运行态
5. 运行期由 ToolRegistry 对外提供详情查询、分组查询与索引构建

## 模块关系

### 上游

| 模块 | 关系 |
|------|------|
| common | 提供 ToolRegistrar trait 的定义（[common/core-traits.md](../common/core-traits.md#toolregistrar)） |
| session | 以 SessionToolsRegistrar 实现 trait，注册 sessions 分组 |
| skills | 以 SkillsToolsRegistrar 实现 trait，注册 skills 分组 |
| im_adapter | 以 ImAdapterToolsRegistrar 实现 trait，注册飞书平台各分组 |

Tools 模块自身以 CoreToolsRegistrar 实现 trait，注册 bash/file_ops/git_ops/meta 分组，并编排调用上述各实现者。

### 下游

- **ToolRegistry**：接收来自各 Registrar 的工具注册，维护全局注册表

### 无关

- **Tool trait**：ToolRegistrar 管理「谁注册工具」，不关心单个工具的内部接口
- **注册后的工具调用路径**：权限校验、工具执行、结果返回等流程不受 ToolRegistrar 影响
