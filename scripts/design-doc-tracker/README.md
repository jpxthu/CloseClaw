# Design Doc Tracker (ddt)

跟踪设计文档的实现状态。当设计文档被代码实现后，用 `ddt` 记录确认点；后续用 `check` 检查是否有文档发生变更。

## 用法

### `python3 ddt.py finished <path>`

将 `<path>` 指向的 `.md` 文件（或目录下所有 `.md` 文件）的当前 git commit 记录为已确认状态，并清空备注。

- `<path>` 可以是单个 `.md` 文件或目录
  - **文件模式**：仅处理该文件
  - **目录模式**：递归处理目录下所有 `.md` 文件（原有行为）
- 在非 `master` 分支上执行时，自动使用 `master` 分支最近一次 commit 作为基准，并输出 warning

### `python3 ddt.py comment <path> <text>`

为已记录的 design doc 设置备注。`<path>` 为相对于项目根的文件路径。

- 文件必须已有记录（先执行 `finished`）

### `python3 ddt.py check`

扫描项目中 `docs/design/` 目录下的 `.md` 文件，报告自上次确认以来发生变更的文档。

- 输出格式：`path` 或 `path\tcomment`（有备注时）

## 记录文件

确认记录存储在同目录下的 `records.json`，由 git 管理，请一起提交。
