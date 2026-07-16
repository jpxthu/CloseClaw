# Platform 需求

## 概述

Owner 将 CloseClaw 部署到不同操作系统时，不需要关心 OS 差异——进程管理、配置目录、终端行为、文件路径都由系统自动适配，体验一致。

## 功能需求

### F1. 跨平台进程生命周期管理

Owner 通过 daemon 命令启动、停止 CloseClaw 服务。系统在 Linux、macOS、Windows 上提供一致的启停行为：启动时创建 PID 标记以追踪进程实例；停止时通过操作系统标准的终止机制关闭进程（Unix 信号、Windows 进程终止 API），而非暴力杀进程。

### F2. 操作系统标准配置目录

Owner 在任意操作系统上首次启动 CloseClaw 时，配置文件自动创建在操作系统约定的标准配置目录下，无需手工指定路径。Linux/macOS 使用 `~/.closeclaw`，Windows 使用 `%APPDATA%\closeclaw`。

### F3. 终端能力自适应

使用 CLI 交互的 User 获得与终端能力匹配的显示体验。当终端支持 ANSI 转义序列时，系统渲染彩色和样式化的输出；当终端不支持时，自动降级为纯文本输出，不出现乱码或转义字符原样泄漏。

### F4. 跨平台文件路径处理

Owner 和 User 使用自己操作系统习惯的路径格式（Unix 用 `~` 和 `/` 分隔，Windows 用 `%APPDATA%` 和 `\` 分隔），系统自动展开环境变量并按平台转换路径分隔符，使用者不需要了解内部如何统一路径表示。

## 关联设计文档

- [✓] [platform/README.md](../design/platform/README.md)

## 非功能需求

- **启动速度**：进程创建和 PID 文件写入不应成为 daemon 启动的瓶颈，操作本身对用户不可感知
- **跨平台一致性**：同一功能在 Linux、macOS、Windows 上的行为语义一致，Owner 切换操作系统时不需要重新学习或调整配置
- **稳定性**：进程终止信号/API 调用必须可靠，不产生僵尸进程或 PID 文件残留
