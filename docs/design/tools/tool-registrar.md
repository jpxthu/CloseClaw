# ToolRegistrar

## 概述

ToolRegistrar 是工具注册能力的统一 trait，抽象各模块"我能注册工具"的接口契约。Tools 模块通过收集已注册的 Registrar 并依次调用其注册方法完成全局工具编排，不再硬编码各模块的特定接口。

## 架构

### Trait 接口

ToolRegistrar 的完整接口定义见 [core-traits](../common/core-traits.md#toolregistrar)。本文档聚焦 tools 模块对 Registrar 的编排逻辑。

### Registrar 收集与编排

1. Tools 模块启动初始化
2. 收集所有 ToolRegistrar 实现者
3. 按优先级排序
4. 依次调用各 Registrar，向 ToolRegistry 注册工具
5. 全部注册完成，ToolRegistry 冻结，进入运行态

### 四个标准 Registrar

| Registrar | priority | 所属模块 | 注册的工具分组 |
|-----------|----------|---------|--------------|
| CoreToolsRegistrar | 1 | tools | bash、file_ops、git_ops、meta |
| SessionToolsRegistrar | 2 | session | sessions |
| SkillsToolsRegistrar | 3 | skills | skills、skill_creator |
| ImAdapterToolsRegistrar | 4 | im_adapter | feishu_im、feishu_calendar、feishu_task、feishu_bitable、feishu_doc、feishu_drive、feishu_sheet |

注册顺序与 tools 模块当前编排逻辑一致，优先级值保证了向后兼容。新增工具提供模块时，选择合适的优先级值即可加入编排链，Tools 模块无需修改。

### 错误处理

注册阶段的错误策略：

- 工具名冲突：报告冲突工具名和双方 Registrar，中断启动
- 单个 Registrar 内部错误：由 Registrar 自行处理（跳过无效工具并记录警告，不中断其他工具注册），但若 Registrar 整体注册失败则报告错误
- 优先级重复：允许多个 Registrar 使用相同优先级，同等优先级下注册顺序不保证

## 数据流

1. 系统启动，Tools 模块收集 ToolRegistrar 实现者
2. 按优先级排序
3. 依次调用各 Registrar 的注册方法：
   - CoreToolsRegistrar 注册 bash/file_ops/git_ops/meta 分组工具
   - SessionToolsRegistrar 注册 sessions 分组工具
   - SkillsToolsRegistrar 注册 skills/skill_creator 分组工具
   - ImAdapterToolsRegistrar 注册飞书平台各分组工具
4. ToolRegistry 包含所有已注册工具
5. 后续流程不变（索引构建、工具发现、system prompt 注入等）

注册完成后，ToolRegistry 进入冻结态，后续的工具发现、索引构建、权限校验等流程与引入 ToolRegistrar 前完全一致——仅改变了"谁调用注册方法"的编排方式，不改变注册结果。

## 模块关系

### 上游

| 模块 | 关系 |
|------|------|
| tools | 收集所有 ToolRegistrar 实现者，按优先级编排调用 |
| session | 以 SessionToolsRegistrar 身份实现 trait，注册 sessions 分组工具 |
| skills | 以 SkillsToolsRegistrar 身份实现 trait，注册 skills/skill_creator 分组工具 |
| im_adapter | 以 ImAdapterToolsRegistrar 身份实现 trait，注册各飞书平台分组工具 |

### 下游

- **ToolRegistry**：接收来自各 Registrar 的工具注册，维护全局注册表

### 无关

- **Tool trait**：ToolRegistrar 管理"谁注册工具"，不关心单个工具的内部接口
- **注册后的工具调用路径**：权限校验、工具执行、结果返回等流程不受 ToolRegistrar 影响
