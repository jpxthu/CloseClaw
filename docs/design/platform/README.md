# Platform

## 概述

- 关联需求文档：[requirements/platform.md](../../requirements/platform.md)
- 一句话：Platform 是操作系统抽象层。它将进程生命周期、配置目录、终端能力检测、文件路径、系统临时目录五类操作系统相关操作封装为平台无关的接口，使上层模块不感知操作系统差异。

## 架构

Platform 按操作系统能力维度划分为五个独立的抽象接口，每个接口对应一类操作系统能力。上层模块只调用平台无关的接口；受支持平台行为一致（见 §平台实现边界）。

### 子功能索引

| 文档 | 内容 |
|------|------|
| [进程生命周期管理](process.md) | 封装 daemon 进程的启动、终止与实例追踪，通过 PID 文件与操作系统信号实现跨平台一致的启停行为 |
| [配置目录](config-directory.md) | 返回配置目录，缺失时自动创建 |
| [终端能力检测](terminal.md) | 检测终端 ANSI 能力与可用宽度，供上层渲染器决定渲染模式与布局约束 |
| [文件路径处理](file-path.md) | 展开 home / 环境变量简写，统一为平台无关的路径表示 |
| [临时目录](temp-directory.md) | 返回系统临时目录，供临时产物按 session 落盘 |

### 平台实现边界

- **Linux**：原生实现
- **macOS**：进程信号、文件路径、终端能力检测、配置目录均与 Linux 行为一致；系统临时目录同样取 `$TMPDIR` 变量，但 macOS 的 `$TMPDIR` 由 launchd 为每个用户会话注入（形如 `/var/folders/<xx>/<rand>/T/`），与 Linux 的默认 `/tmp` 取值不同（见 [临时目录](temp-directory.md)）
- **Windows**：经 WSL2 覆盖，行为等同 Linux，不做原生 Windows 适配

## 数据流

1. 上层模块调用 platform 接口（平台无关）
2. 接口层调用统一的平台实现（见 §平台实现边界），经操作系统 API 完成实际操作
3. 返回平台无关的结果给上层模块

## 模块关系

> 「上游/下游」指数据流与调用关系（含经 common trait 完成的调用），不等于 crate 依赖；crate 依赖以 [STANDARDS.md 依赖方向允许边表](../STANDARDS.md) 为准。

- **上游**：CLI 模块（Chat 层的终端能力检测；Admin 层的进程管理与文件路径处理）、Daemon（启动关闭时的进程与信号处理、配置目录初始化）、config 模块（经配置目录接口定位配置目录并布局其下子目录，见 [配置目录](config-directory.md)）、tools 模块（后台任务输出与超长命令输出经临时目录接口落盘，见 [临时目录](temp-directory.md)）
- **下游**：操作系统 API（进程与信号、文件系统、环境变量、终端尺寸）（platform 调用）；此外，配置数据经 config 模块供 CLI（Admin config 命令）、Gateway、IM Adapter 间接消费（三者不直接调用 platform 接口，见 [配置目录](config-directory.md)）
- **无关**：无模块级无关关系——platform 作为底层操作系统抽象被上层模块普遍消费，不存在模块级「无调用且无数据流关联」的模块；个别接口对特定调用方的无关界定以子文档为准（如 [文件路径处理](file-path.md)、[终端能力检测](terminal.md)、[临时目录](temp-directory.md)）
- **共享类型**：无——platform 收发的均为操作系统原语（路径字符串、进程标识、终端尺寸等），不与业务模块共享 [common](../common/README.md) 中定义的数据结构
