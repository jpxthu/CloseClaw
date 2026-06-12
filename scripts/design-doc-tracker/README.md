# Design Doc Tracker (ddt)

跟踪设计文档的实现状态。当设计文档被代码实现后，用 `ddt` 记录确认点；后续用 `check` 检查是否有文档发生变更。

## 用法

### `python3 ddt.py finished <dir>`

将 `<dir>` 下所有 `.md` 文件的当前 git commit 记录为已确认状态。

- **必须在 `master` 分支上执行**
- `<dir>` 必须存在且包含至少一个 `.md` 文件

### `python3 ddt.py check`

扫描项目中 `docs/design/` 目录下的 `.md` 文件，报告自上次确认以来发生变更的文档。

## 记录文件

确认记录存储在同目录下的 `records.json`，由 git 管理，请一起提交。
