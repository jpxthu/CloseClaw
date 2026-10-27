# CI

工作流定义在 `workflows/ci.yml`，本文件只说明口径。

## Job

| Job | 内容 |
|-----|------|
| Test | fmt / env 禁令 / test-audit / test-naming-guard / clippy / dep-guard / nextest 默认跑批 / doctest |
| Feature-gated tests | `--features closeclaw-llm/fake-llm,closeclaw-gateway/full-tests`，覆盖 feature 门控用例（默认跑批不含） |
| MSRV check | 以根 `Cargo.toml` 的 `rust-version` 执行 `check --workspace --all-targets --locked` |

## 版本固定

外部依赖全部固定版本，禁止浮动 tag 与 latest——上游发版会静默改变 CI 行为，红灯变得无法复现。

| 对象 | 固定方式 |
|------|----------|
| action（`uses:`，含 GitHub 官方） | pin 到 40 位 commit SHA，行尾 `# vX` 注释版本 |
| CI 工具（`tool:`） | 显式写版本号，禁止 latest；版本集中在 workflow 顶部 `env:` 段 |
| 本地 action（`./...`）、`docker://` | 不适用 |

升级 = 改 SHA + 改行尾注释 + 改 `env` 版本号，走普通 PR。守卫：`scripts/periodic-checks.sh --pin-audit`。
