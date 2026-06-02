# Debian Packaging Implementation Summary

## Overview

Successfully implemented Debian packaging for socktop using `cargo-deb`, with GitHub Actions automation for building packages for both AMD64 and ARM64 architectures.

## Branches Created

1. **`feature/debian-packaging`** - Main branch with debian packaging implementation
2. **`feature/man-pages`** - Separate branch for man pages work (to be researched further)

## What Was Added

### 1. Cargo.toml Updates

Both `socktop/Cargo.toml` and `socktop_agent/Cargo.toml` were updated with:
- `[package.metadata.deb]` sections
- Package metadata (maintainer, description, dependencies)
- Asset definitions (binaries, documentation)
- Systemd service configuration (agent only)

### 2. Systemd Service

**File**: `socktop_agent/socktop-agent.service`
- Runs as `socktop` user/group
- Listens on port 3000 by default
- Security hardening enabled
- Disabled by default (user must explicitly enable)

### 3. Maintainer Scripts

**Directory**: `socktop_agent/debian/`

- **`postinst`**: Creates `socktop` user/group, sets up `/var/lib/socktop` directory
- **`postrm`**: Cleanup on package removal/purge

### 4. GitHub Actions Workflow

**File**: `.github/workflows/build-deb.yml`

Features:
- Builds for both x86_64 and ARM64
- Triggered on:
  - Push to `master` or `feature/debian-packaging`
  - Pull requests to `master`
  - Version tags (v*)
  - Manual workflow dispatch
- Creates artifacts:
  - `debian-packages-amd64`
  - `debian-packages-arm64`
  - `all-debian-packages` (combined)
  - `checksums` (SHA256SUMS)
- Automatic GitHub releases for version tags

### 5. Documentation

**File**: `docs/DEBIAN_PACKAGING.md`

Comprehensive guide covering:
- Building packages locally
- Cross-compilation for ARM64
- Installation and configuration
- Using GitHub Actions artifacts
- Creating local APT repositories
- Troubleshooting

## Package Details

### socktop (TUI Client)
- **Binary**: `/usr/bin/socktop`
- **Size**: ~3.5 MB (x86_64)
- **Dependencies**: Auto-detected

### socktop_agent (Daemon)
- **Binary**: `/usr/bin/socktop_agent`
- **Service**: `socktop-agent.service`
- **User/Group**: `socktop` (created automatically)
- **State directory**: `/var/lib/socktop`
- **Size**: ~6.7 MB (x86_64)
- **Dependencies**: Auto-detected

## Testing

Both packages successfully built locally:
```
✓ socktop_1.50.0-1_amd64.deb
✓ socktop-agent_1.50.1-1_amd64.deb
```

Verified:
- Package contents (dpkg -c)
- Package metadata (dpkg -I)
- Systemd service file inclusion
- Maintainer scripts inclusion
- Documentation inclusion

## Usage

### For Users

Download pre-built packages from GitHub Actions artifacts:
1. Go to Actions tab
2. Select latest "Build Debian Packages" run
3. Download architecture-specific artifact
4. Install: `sudo dpkg -i socktop*.deb`

### For Developers

Build locally:
```bash
cargo install cargo-deb
cargo deb --package socktop
cargo deb --package socktop_agent
```

Cross-compile for ARM64:
```bash
rustup target add aarch64-unknown-linux-gnu
sudo apt install gcc-aarch64-linux-gnu libc6-dev-arm64-cross
cargo deb --package socktop --target aarch64-unknown-linux-gnu
```

## Next Steps

To get packages in official APT repositories:

1. **Short term**: Host packages on GitHub Releases (automated)
2. **Medium term**: Create PPA for Ubuntu users
3. **Long term**: Submit to Debian/Ubuntu official repositories

## Files Modified/Created

```
Modified:
  socktop/Cargo.toml
  socktop_agent/Cargo.toml

Created:
  .github/workflows/build-deb.yml
  docs/DEBIAN_PACKAGING.md
  socktop_agent/socktop-agent.service
  socktop_agent/debian/postinst
  socktop_agent/debian/postrm
```

## Commit

```
532ed16 Add Debian packaging support with cargo-deb
```

## Resources

- [cargo-deb documentation](https://github.com/kornelski/cargo-deb)
- [Debian Policy Manual](https://www.debian.org/doc/debian-policy/)
- Full documentation in `docs/DEBIAN_PACKAGING.md`
