# Environment Setup

## Rust Version

CloseClaw requires **Rust 1.85 or later** (recommended: latest stable).

```bash
rustc --version
# Should be >= 1.85.0
```

### Upgrading Rust

If you have an older Rust version (e.g., 1.75), upgrade via rustup:

```bash
# If rustup is installed
rustup update stable

# If rustup is not installed, install it first
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## Cargo Registry Mirror (China)

For faster downloads in China, configure the Aliyun mirror:

```bash
mkdir -p ~/.cargo
cat > ~/.cargo/config.toml << 'EOF'
[source.crates-io]
replace-with = "aliyun"

[source.aliyun]
registry = "sparse+https://mirrors.aliyun.com/crates.io-index/"
EOF
```

Then clear the old index and download fresh:
```bash
rm -rf ~/.cargo/registry/index/*
cargo check
```

## Build Commands

### Basic Build
```bash
# Debug build - fast compilation, slower execution
cargo build

# Release build - slower compilation, faster execution (recommended for production)
cargo build --release
```

### Build with Specific Cores
```bash
# Use specific number of cores
CARGO_BUILD_JOBS=16 cargo build --release
```

### Check Cores Available
```bash
nproc  # Linux
sysctl -n hw.ncpu  # macOS
```

## Common Issues

### "failed to fetch `https://mirrors.aliyun.com/crates.io-index/`"
- Registry index is outdated. Clear and re-fetch:
```bash
rm -rf ~/.cargo/registry/index/*
cargo update
```

### "Unable to update crates.io index"
- Network issue. Try switching to a different mirror or use VPN.

### "file not found for module"
- Run `cargo check` from the project root, not from a subdirectory.

### Compilation errors
- Ensure Rust version is >= 1.85
- Run `cargo update` to update dependencies

---

## Deployment (Pull → Rebuild → Restart)

### Standard Deployment Flow

> **No `cargo clean` needed** for routine code pulls — Cargo's incremental build handles updates automatically.

```bash
cd /home/admin/code/closeclaw

# 1. Stop running instance (if any)
pkill closeclaw   # or Ctrl+C if running in foreground

# 2. Pull latest code
git pull

# 3. Rebuild (release mode)
cargo build --release

# 4. Restart
./target/release/closeclaw run --config-dir ./configs
```

### When You DO Need `cargo clean`
- Rust version was upgraded (e.g., 1.75 → 1.85)
- Dependencies changed fundamentally (not just updated)
- Build cache is corrupted (symptoms: cryptic/unexplainable errors)
- After `Cargo.toml` dependency changes

### Health Check After Deploy
```bash
# Verify binary runs
./target/release/closeclaw --version

# Check it starts without errors
./target/release/closeclaw run --config-dir ./configs &
sleep 2
# If no crash/error output, it's healthy
```
