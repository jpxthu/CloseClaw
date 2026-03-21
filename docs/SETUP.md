# 环境配置

## Rust 版本

CloseClaw 需要 **Rust 1.85 或更高版本**（推荐：最新版 stable）。

```bash
rustc --version
# 应该 >= 1.85.0
```

### 升级 Rust

如果已有旧版 Rust（如 1.75），通过 rustup 升级：

```bash
# 如果已安装 rustup
rustup update stable

# 如果没有安装，先安装 rustup
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## Cargo 镜像加速（中国）

为加速在中国地区的下载，配置 Tuna 镜像：

```bash
mkdir -p ~/.cargo
cat > ~/.cargo/config.toml << 'EOF'
[source.crates-io]
replace-with = "tuna"

[source.tuna]
registry = "sparse+https://mirrors.tuna.tsinghua.edu.cn/crates.io-index/"
EOF
```

然后清除旧索引并重新下载：
```bash
rm -rf ~/.cargo/registry/index/*
cargo check
```

## 构建命令

### 基础构建
```bash
# Debug 构建 - 编译快，执行慢
cargo build

# Release 构建 - 编译慢，执行快
cargo build --release

# 运行测试
cargo test

# 运行并显示输出
cargo run -- [args]
```

## 目录结构

```
closeclaw/
├── src/
│   ├── main.rs           # CLI 入口
│   ├── permission/       # 权限引擎
│   ├── config/           # 配置系统
│   ├── agent/            # Agent 运行时
│   ├── gateway/          # 网关 + IM 适配器
│   ├── skills/           # 内置 Skills
│   └── llm/              # LLM 接口抽象
├── configs/
│   ├── agents.json       # Agent 配置
│   ├── permissions.json  # 权限规则
│   └── .env              # API Key（不提交到 Git）
├── docs/
│   ├── SETUP.md          # 本文件
│   ├── cli/README.md     # CLI 文档
│   └── permission/       # 权限文档
└── tests/                # 集成测试
```

## 配置步骤

### 1. 复制环境配置示例

```bash
cp configs/.env.example configs/.env
# 然后编辑 configs/.env 填入你的 API Key
```

### 2. 配置 Agent

编辑 `configs/agents.json`：

```json
{
  "version": "1.0",
  "agents": [
    {
      "name": "guide",
      "model": "minimax/MiniMax-M2.7",
      "persona": "你是 CloseClaw 的引导助手。",
      "max_iterations": 100,
      "timeout_minutes": 30
    }
  ]
}
```

### 3. 配置权限规则

编辑 `configs/permissions.json`。详见 [permission/RULES.md](permission/RULES.md)。

### 4. 启动 Daemon

```bash
# 启动 daemon
cargo run --release -- run

# 停止 daemon
cargo run --release -- stop
```

## 快速验证

```bash
# 检查代码是否通过编译检查
cargo check

# 运行所有测试
cargo test

# 查看所有内置 skills
cargo run -- skill list
```

## 常见问题

### 编译报错：link for cretonofound

确保 Rust 版本 >= 1.85，或者降级相关依赖版本。

### 测试失败

检查是否有环境变量未配置（部分测试需要 MINIMAX_API_KEY）。
