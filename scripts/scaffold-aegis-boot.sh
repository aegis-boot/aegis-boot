#!/bin/bash
# Scaffold for aegis-boot Rust workspace
# Creates: crates/efi-orchestrator, crates/iso-parser, crates/config

set -e

REPO_DIR="${1:-./aegis-boot}"

echo "=== Scaffolding aegis-boot Rust workspace ==="
echo "Target: $REPO_DIR"

mkdir -p "$REPO_DIR"
cd "$REPO_DIR"

# 1. Create rust-toolchain.toml (pinned to stable)
cat > rust-toolchain.toml << 'EOF'
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
EOF

# 2. Create LICENSE (MIT as default)
cat > LICENSE << 'EOF'
MIT License

Copyright (c) 2024

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
EOF

# 3. Create .gitignore (without Cargo.lock - binary crate needs it for reproducible builds)
cat > .gitignore << 'EOF'
/target/
**/*.rs.bk
*.swp
*.swo
*~
.DS_Store
.env
.env.local
coverage/
dist/
build/
*.log
EOF

# 4. Create workspace Cargo.toml with [workspace.package] for inheritance
cat > Cargo.toml << 'EOF'
[workspace.package]
version = "0.1.0"
edition = "2021"
authors = ["Aegis Team"]
license = "MIT"

[workspace]
resolver = "2"

members = [
    "crates/efi-orchestrator",
    "crates/iso-parser",
    "crates/config",
]

[workspace.dependencies]
# Common dependencies go here
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror = "1.0"
log = "0.4"
env_logger = "0.10"
EOF

# 5. Create crates directory structure
mkdir -p crates/efi-orchestrator/src
mkdir -p crates/iso-parser/src
mkdir -p crates/config/src

# 6. Initialize each crate with cargo new (binary for efi-orchestrator, lib for others)
echo "Creating crates..."
cargo new --name efi-orchestrator crates/efi-orchestrator
cargo new --lib crates/iso-parser
cargo new --lib crates/config

# 7. Update efi-orchestrator Cargo.toml to inherit from workspace
cat > crates/efi-orchestrator/Cargo.toml << 'EOF'
[package]
name = "efi-orchestrator"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true

[[bin]]
name = "efi-orchestrator"
path = "src/main.rs"

[dependencies]
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
log.workspace = true
env_logger.workspace = true
EOF

# 8. Update iso-parser Cargo.toml to inherit from workspace
cat > crates/iso-parser/Cargo.toml << 'EOF'
[package]
name = "iso-parser"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true

[dependencies]
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
log.workspace = true
env_logger.workspace = true

[dev-dependencies]
tempfile = "3.8"
EOF

# 9. Update config Cargo.toml to inherit from workspace
cat > crates/config/Cargo.toml << 'EOF'
[package]
name = "aegis-config"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true

[dependencies]
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
log.workspace = true
env_logger.workspace = true
serde_derive = "1.0"
EOF

# 10. Initialize git repository
echo "Initializing git..."
git init
git add -A
git commit -m "Initial: scaffold aegis-boot Rust workspace"

# 11. Verify build
echo "Verifying build..."
cd "$REPO_DIR"
cargo check

echo "=== Done! ==="
echo "Workspace created at: $REPO_DIR"
echo "Run 'cd $REPO_DIR && cargo build' to build"