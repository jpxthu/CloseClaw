# Platform

## 概述

- 关联需求文档：[requirements/platform.md](../../requirements/platform.md)
- 一句话：Platform 模块是操作系统抽象层。它将进程管理、配置目录、终端检测、文件路径四类 OS 相关操作封装为统一接口，使上层模块不感知操作系统差异。

## 架构

Platform 模块按 OS 能力维度划分为独立的抽象接口，每个接口对应一个 OS 差异点。平台实现范围（Linux/macOS/Windows-WSL2 的复用关系）见 §平台实现边界。

```
platform/
├── process.rs    — 进程生命周期管理（启动、终止、PID 文件）
├── config.rs     — 配置目录解析
├── terminal.rs   — 终端能力检测与 I/O
└── fs.rs         — 文件路径与权限
```

### 各抽象接口

**进程管理**

封装 daemon 的启动和终止操作。启动时写入 PID 文件作为进程实例追踪的唯一事实源（单次轻量同步写，不构成启动瓶颈），终止时等待进程退出确认并清理 PID 文件，不产生僵尸进程或 PID 文件残留。平台通过信号（SIGTERM/SIGINT）终止进程，PID 文件路径遵循平台惯例。

**配置目录**

根据操作系统返回配置文件的根目录。Linux/macOS 使用 `~/.closeclaw`。目录不存在时由本接口负责创建，确保首次启动即无需手工建目录。接口返回平台无关的路径表示，调用方不拼接路径分隔符。

**终端检测**

检测终端能力与尺寸，一次性返回能力标记和可用宽度，上层 Renderer 消费两者：

- **ANSI 能力检测**：`TERM` 环境变量含 `xterm`、`screen`、`ansi`、`vt100` 或 `color` → 支持 ANSI；其余 → 不支持（降级为纯文本输出，不出现乱码或转义序列原样显示）
- **终端宽度获取**：通过操作系统终端尺寸接口获取当前可用列数；获取失败时回退默认值（约 80 列）

该检测覆盖主流终端：Ubuntu bash（通常 TERM=xterm-256color）、macOS Terminal（xterm-256color）、Windows WSL2（xterm-256color）均默认支持 ANSI。

**文件路径**

统一内部路径表示为 `/` 分隔符。写入文件系统时按平台转换分隔符，读取时反向统一。环境变量展开（`~` 等）由本模块处理，上层传入原始路径。

### 平台实现边界

每个平台差异点对应一个抽象接口。macOS 在进程信号、文件路径、终端检测、配置目录等方面与 Linux 行为一致（含统一使用 `~/.closeclaw`，对应需求 F2），直接复用 Linux 实现。

Windows 场景经 WSL2 覆盖（行为等同 Linux，直接复用 Linux 实现），不做原生 Windows 适配。

## 数据流

1. 上层模块调用 platform 接口
2. 接口层路由到平台无关的抽象方法
3. 编译时选定的平台实现执行（Linux 实现；macOS 直接复用 Linux 实现，无独立代码路径）
4. 返回平台无关的结果给上层模块

## 模块关系

- **上游**：CLI 模块（Chat 和 Admin 层的进程管理、配置目录；Chat 层内 TerminalRenderer 消费终端能力检测结果）、Daemon（启动关闭时的信号处理）
- **下游**：操作系统 API（信号、文件系统、环境变量、进程管理）
- **无关**：Gateway（platform 不参与消息路由）、IM Adapter（platform 是 OS 层，与消息渠道无关）
