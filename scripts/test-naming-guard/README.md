# test-naming-guard 测试命名守卫

对全仓 `.rs` 测试函数做 `test_` 前缀静态检查（STANDARDS.md §3；baseline=现状，仅对新增违规报警）：

- 扫描器：多行状态机识别 `#[test]` / `#[tokio::test]` 属性行（含带参形态如 `#[tokio::test(flavor = "current_thread")]`；`#[cfg(test)]` / `#[test_case]` / `#[test_suite]` 均不算），向后跳过连续属性行（如 `#[should_panic]`）取第一个 `(async )?fn` 声明行，函数名缺 `test_` 前缀即违规
- 扫描范围：按测试属性定位、不按文件名猜——全仓 `.rs`（`src/`、`tests/`、`crates/*/src`、`crates/*/tests`，排除 `target/`）
- baseline：`baselines/missing_test_prefix.txt` 三态比对——新增违规 FAIL（exit 1）/ 存量豁免 / 已消除条目提示可收窄（不 FAIL）；条目键格式 `相对路径:行号: missing_test_prefix(函数名)`，比对键去行号，行号漂移不触发 FAIL
- `--fix` 自动收窄 baseline：删除已消除条目并回写（头部注释与既有顺序原样保留）；新增违规照常 FAIL，退出码不变
- `--update-baseline` 以当前全量扫描结果重写 baseline 文件（与 `--fix` 互斥）

生产检查入口（从仓库根执行）：`python3 scripts/test-naming-guard/test_naming_guard.py`，经 `.github/workflows/ci.yml` test job 挂载（CI 同链路生效）。

## 单元测试

套件：`test_naming_guard_test.py`（扫描器正常路径与边界 / baseline 三态与收窄 / CLI 错误路径）。

手动运行命令（从仓库根执行，完整命令文本以此处为唯一出处）：

```bash
python3 -m unittest discover -s scripts/test-naming-guard -p "*_test.py"
```
