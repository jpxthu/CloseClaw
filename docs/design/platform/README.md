# Platform

## 概述

Platform 模块是操作系统抽象层。它将进程管理、信号处理、配置目录、终端 I/O、文件路径等 OS 相关操作封装为统一接口，使上层模块不感知操作系统差异。

## 架构

Platform 模块按 OS 能力维度划分为独立的抽象接口，每个接口对应一个 OS 差异点。Linux、macOS、Windows 各有独立实现，编译时按目标平台选择。

```
platform/
├── process.rs    — 进程生命周期管理（启动、终止、PID 文件）
├── config.rs     — 配置目录解析
├── terminal.rs   — 终端能力检测与 I/O
└── fs.rs         — 文件路径与权限
```

### 各抽象接口

**进程管理**

封装 daemon 的启动和终止操作。Unix 平台通过信号（SIGTERM/SIGINT）终止进程，PID 文件路径遵循平台惯例。Windows 平台通过进程终止 API 完成，PID 管理可能改用 Named Pipe 或注册表键。

**配置目录**

根据操作系统返回配置文件的根目录。Linux/macOS 使用 `~/.closeclaw`，Windows 使用 `%APPDATA%\closeclaw`。接口返回平台无关的路径表示，调用方不拼接路径分隔符。

**终端检测**

检测当前终端是否支持 ANSI 转义序列。Linux/macOS 检查 TERM 环境变量，Windows 检测终端仿真环境。返回能力标记，上层 Renderer 据此决定是否启用 ANSI。

**文件路径**

统一内部路径表示为 `/` 分隔符。写入文件系统时按平台转换分隔符，读取时反向统一。环境变量展开（`~`、`%APPDATA%` 等）由本模块处理，上层传入原始路径。

### 平台实现边界

每个平台差异点对应一个抽象接口。接口设计考虑 Linux/macOS 的共性：macOS 在进程信号、文件路径、终端检测等方面与 Linux 基本一致，仅在配置目录惯例上有细微差异（可共用 Linux 实现或独立适配）。

Windows 对所有接口均有独立实现。Windows 实现通过同一接口提供，接口层预留 Windows 路径。

## 数据流

```
上层模块调用 platform 接口
  ↓
接口层：平台无关的抽象方法
  ↓
编译时选择平台实现
  ├── Linux 实现
  ├── macOS 实现（可复用 Linux 或独立适配）
  └── Windows 实现
  ↓
返回平台无关的结果 → 上层模块
```

## 模块关系

- **上游**：CLI 模块（Chat 和 Admin 层的进程管理、配置目录、终端 I/O）、Daemon（启动关闭时的信号处理）
- **下游**：操作系统 API（信号、文件系统、环境变量、进程管理）
- **无关**：Gateway（platform 不参与消息路由）、IM Adapter（platform 是 OS 层，与消息渠道无关）
