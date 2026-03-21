# Cargo Commands Guide

## Essential Commands

### Build
```bash
# Debug build (fast compilation, slower execution) - for development
cargo build

# Release build (optimized) - for production/testing
cargo build --release
```

### Test
```bash
# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run with output
cargo test -- --nocapture

# Run lib tests only
cargo test --lib

# Run integration tests
cargo test --test integration_test_name
```

### Check
```bash
# Type check without building
cargo check

# Check with all features
cargo check --all-features
```

### Clippy (Linting)
```bash
cargo clippy -- -D warnings
```

### Format
```bash
cargo fmt
```

### Documentation
```bash
# Build docs
cargo doc

# Build and open in browser
cargo doc --open
```

## Dependencies

### Add dependency
```bash
cargo add serde_json
cargo add tokio --features full
```

### Update dependencies
```bash
cargo update
```

## Workspace
```bash
# Build all packages in workspace
cargo build --workspace

# Build specific package
cargo build -p closeclaw
```
