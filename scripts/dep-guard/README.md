# dep-guard 依赖治理守卫

对仓库做五项静态检查（baseline=现状，仅对新增越界报警，退出码 = FAIL 项数）：

1. 依赖方向（`dep_edges.py` + `allow_table.py`）
2. 禁二次出口（`second_export.py`）
3. common 准入一致性（`common_admission.py`）
4. 死依赖（`dead_deps.py`）
5. rust-version 声明完整性（`rust_version.py`）

生产检查入口（从仓库根执行）：`python3 scripts/dep-guard/dep_guard.py check`，经 `scripts/periodic-checks.sh --dep-guard` 挂载（CI 同链路生效）。

## 单元测试

套件：6 个 `*_test.py`（baseline / common_admission / dead_deps / dep_guard / rust_version / second_export），随 `--dep-guard` 自动运行。

手动运行命令（从仓库根执行，完整命令文本以此处为唯一出处）：

```bash
python3 -m unittest discover -s scripts/dep-guard -p "*_test.py"
```
