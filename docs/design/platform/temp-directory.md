# 临时目录

## 概述

临时目录接口提供 CloseClaw 存放临时产物的系统临时目录的绝对路径，供后台任务输出、超长命令输出等场景取用。各平台按操作系统惯例解析，上层无需感知平台分支，也不硬编码 `/tmp` 等具体路径。

## 架构

接口返回系统临时目录的绝对路径，按平台解析：

| 平台 | 取值 |
|------|------|
| Linux / WSL2 | `$TMPDIR`；未设置时回退系统默认 `/tmp` |
| macOS | `$TMPDIR`（launchd 为每个用户会话注入，形如 `/var/folders/<xx>/<rand>/T/`） |
| Windows | 不做原生适配；经 WSL2 时行为等同 Linux |

- **由 platform 统一承接**：系统临时目录按环境变量分平台解析（解析规则见 [requirements/platform.md F6](../../requirements/platform.md)）；由本接口集中解析，上层（tools 等）直接取用，不再各自硬编码 `/tmp` 或判断平台。
- **不硬编码路径**：路径由平台环境决定，上层只消费接口返回的结果，不写死 `/tmp`、仓库目录等具体路径。
- **解析基准**：与 Rust 标准库 `std::env::temp_dir()` 的解析规则一致（Rust 侧 `tempfile::TempDir` 亦基于它），确保文档语义与代码行为统一。
- **只返回路径**：接口只负责返回路径，不负责创建与清理。系统临时目录由操作系统保证存在（如 Linux 的 `/tmp`、macOS 的 `$TMPDIR` 均由系统初始化），故本接口不重复创建；可写性与临时文件生命周期由使用方（tools）负责。

## 数据流

1. 上层模块（tools）请求系统临时目录
2. 接口按平台解析环境变量（macOS/Linux/WSL2 取 `$TMPDIR`，未设置时回退 `/tmp`）
3. 返回系统临时目录的绝对路径给上层

## 模块关系

- **上游**：tools（后台任务输出文件、超长命令输出文件在本目录下按 session 隔离落盘，见 [background-tasks.md](../tools/background-tasks.md) 输出管理、[bash-tool.md](../tools/bash-tool.md) 输出持久化）
- **下游**：操作系统环境变量 API（本接口调用）
- **与模块内其他子功能**：与进程生命周期管理、配置目录、终端能力检测、文件路径处理并列，互不依赖
- **无关**：Gateway、IM Adapter（不涉及临时文件落盘）
