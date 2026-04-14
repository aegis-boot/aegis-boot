# Building Aegis-Boot

This document describes the reproducible build environment and verification process for Aegis-Boot.

## Overview

Aegis-Boot uses a multi-layered approach to ensure bit-for-bit reproducible builds:

1. **Dockerfile.locked** - Pinned base image with SHA256 hash
2. **flake.nix** - Nix flake for declarative, reproducible development environments
3. **GitHub Actions workflow** - Automated verification of build reproducibility

## Prerequisites

- Docker 24.0+
- Nix 2.18+ (optional, for Nix-based builds)
- Git

## Quick Start

### Using Docker

```bash
# Build the container
docker build -t nexus-build -f Dockerfile.locked .

# Run verification
docker save nexus-build | sha256sum > build.sha256
```

### Using Nix

```bash
# Enter the development shell
nix develop

# Or with flakes
nix develop github:williamzujkowski/aegis-boot
```

## Build Dependencies

The following dependencies are pinned in `Dockerfile.locked`:

| Component | Version      | Pin Method              |
| --------- | ------------ | ----------------------- |
| Ubuntu    | 22.04        | SHA256 digest           |
| Rust      | 1.85.0       | Exact version in rustup (edition2024 required by transitive deps) |
| EDK II    | stable202311 | Git tag                 |
| Python    | 3.10+        | System package          |
| pip       | 24.0.1       | Exact version           |
| nasm      | latest       | System package          |
| uuid-dev  | latest       | System package          |
| iasl      | latest       | System package          |

## Verification Process

The reproducible build is verified in CI via `.github/workflows/reproducible-build.yml`.

### How It Works

1. **Pass 1**: Build the Docker image and compute SHA256 hash
2. **Pass 2**: Rebuild from scratch and compute SHA256 hash
3. **Compare**: Diff the two cryptographic hashes

If the hashes match, the build is verified as reproducible. If they differ, the build environment or dependencies have drifted.

### Running Verification Locally

```bash
# Pass 1
docker build -t nexus-build:pass1 -f Dockerfile.locked .
docker save nexus-build:pass1 | sha256sum > build1.sha256

# Pass 2
docker build -t nexus-build:pass2 -f Dockerfile.locked .
docker save nexus-build:pass2 | sha256sum > build2.sha256

# Compare
diff build1.sha256 build2.sha256 && echo "Reproducible!" || echo "NOT reproducible"
```

### CI Verification

The workflow file is located at `.github/workflows/reproducible-build.yml` and runs on:

- Every push to `main`
- Every pull request
- Manual trigger via `workflow_dispatch`

## Troubleshooting

### Hash Mismatch

If builds are not reproducible:

1. Check for non-deterministic build steps (random seeds, timestamps)
2. Verify all package versions are pinned
3. Ensure base image uses digest, not tag
4. Review build scripts for network calls during build

### Docker Build Cache

To ensure clean verification, avoid using Docker build cache:

```bash
docker build --no-cache -t nexus-build -f Dockerfile.locked .
```

## Security Considerations

- Base image pinned with SHA256 to prevent supply chain attacks
- All build dependencies installed from trusted sources
- No secret handling in build environment
- Artifacts stored with 30-day retention
