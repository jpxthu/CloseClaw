# Requirement E2E Tracker (ret)

跟踪各模块「测试用例发现」状态。当一个模块的需求文档（`docs/requirements/<module>.md`）发生变更时，该模块的测试用例应被标记为「需重新发现」。

## 跟踪单位：模块

一个**模块**由 `docs/requirements/<module>.md` 定义（`README.md`、`STANDARDS.md` 不是模块，会被排除）。模块清单从 `docs/requirements/*.md` 的文件名推导。

**只跟踪需求文档，不跟踪设计文档。** 原因：e2e 测试用例验证的是需求文档定义的用户可感知行为，设计文档是内部架构、黑盒不可观测。需求变 → 用户行为变 → 用例需重新发现；设计变但需求没变 → 用例不用动。

## 用法

### `python3 ret.py finished <module>`

将模块 `<module>` 的测试用例标记为已发现，记录的 commit 为 `git merge-base HEAD origin/master`，并清空该模块的 `comment` 和 `blocked_reason`。

- `<module>` 为模块名（如 `agent`、`session`），对应 `docs/requirements/<module>.md`
- 模块无需求文档时报错返回 rc=1
- `origin/master` 不存在或 merge-base 失败时，报错返回 rc=1（逻辑同 ddt）

### `python3 ret.py blocked <module> <reason>`

将模块标记为 blocked 状态，附带原因说明。

- 若模块已有记录，更新其 `blocked_reason`
- 若模块无记录，自动创建新记录（`commit` 留空），并设置 `blocked_reason`
- blocked 的模块在 `check` 中不会被报告（见下方 check 逻辑）
- 需求文档被更新后自动解除 blocked 状态

### `python3 ret.py comment <module> <text>`

为模块设置备注。

- 若模块已有记录，仅覆盖 `comment`
- 若模块无记录，自动创建新记录，`commit` 留空，`comment` 设为指定文本
- `<text>` 可以为空字符串

### `python3 ret.py check`

扫描所有模块，报告「需求文档自上次确认后变更」或「未跟踪」的模块。对每个模块的处理逻辑：

1. 无记录 → 输出（未跟踪）
2. 有记录且 `blocked_reason` 非空：
   - 需求文档未更新 → **跳过，不输出**（blocked 状态保持）
   - 需求文档已更新 → 自动清除 `blocked_reason`，更新 commit 等字段，正常输出
3. 有记录且 `blocked_reason` 为空：
   - 需求文档未更新 → 跳过
   - 需求文档已更新 → 输出（正常变更）

- 记录中 commit 为空的模块视为 changed，直接输出
- 输出格式：`module` 或 `module\tcomment`（有备注时）

## 记录文件

确认记录存储在同目录下的 `records.json`，由 git 管理，请一起提交。每条记录的字段：

| 字段 | 说明 |
|------|------|
| `module` | 模块名 |
| `commit` | 最后确认的 git commit hash（`finished` 时写入；`blocked`/`comment` 新建时留空） |
| `commit_time` | commit 的时间戳 |
| `confirmed_time` | 记录创建/更新的时间戳 |
| `comment` | 通用备注文本 |
| `blocked_reason` | block 原因文本。非空表示模块处于 blocked 状态；为空表示正常状态 |

## 与 ddt 的关系

`ret` 与 `scripts/design-doc-tracker/ddt.py`（设计文档实现状态跟踪）共用 `scripts/_tracker_core.py` 中的 git helpers、records 读写、merge-base、diff 检测、`.md` 收集/排序等核心逻辑。两者跟踪单位与判据不同：ddt 跟踪单个 `.md` 文件的实现状态，ret 跟踪模块（需求文档）的测试用例发现状态。
