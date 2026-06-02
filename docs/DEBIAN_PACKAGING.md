# Debian Packaging for socktop

This document describes how to build and use Debian packages for socktop and socktop_agent.

## Prerequisites

Install `cargo-deb`:

```bash
cargo install cargo-deb
```

## Building Packages Locally

### Build for your current architecture (x86_64)

```bash
# Build socktop TUI client
cargo deb --package socktop

# Build socktop_agent daemon
cargo deb --package socktop_agent
```

The `.deb` files will be created in `target/debian/`.

### Cross-compile for ARM64 (Raspberry Pi, etc.)

First, install cross-compilation tools:

```bash
sudo apt-get update
sudo apt-get install gcc-aarch64-linux-gnu libc6-dev-arm64-cross
```

Add the ARM64 target:

```bash
rustup target add aarch64-unknown-linux-gnu
```

Configure the linker by creating `.cargo/config.toml`:

```toml
[target.aarch64-unknown-linux-gnu]
linker = "aarch64-linux-gnu-gcc"
```

Build the packages:

```bash
# Build for ARM64
cargo deb --package socktop --target aarch64-unknown-linux-gnu
cargo deb --package socktop_agent --target aarch64-unknown-linux-gnu
```

## Installing Packages

### Install socktop TUI client

```bash
sudo dpkg -i socktop_*.deb
```

### Install socktop_agent daemon

```bash
sudo dpkg -i socktop_agent_*.deb
```

The agent package will:
- Create a `socktop` system user and group
- Install the binary to `/usr/bin/socktop_agent`
- Install a systemd service file (disabled by default)
- Create `/var/lib/socktop` for state files

### Enable and start the agent service

```bash
# Enable to start on boot
sudo systemctl enable socktop-agent

# Start the service
sudo systemctl start socktop-agent

# Check status
sudo systemctl status socktop-agent
```

### Configure the agent

Edit the systemd service to customize settings:

```bash
sudo systemctl edit socktop-agent
```

Add configuration in the override section:

```ini
[Service]
Environment=SOCKTOP_PORT=8080
Environment=SOCKTOP_TOKEN=your-secret-token
Environment=RUST_LOG=info
```

Then restart:

```bash
sudo systemctl restart socktop-agent
```

## GitHub Actions

The project includes a GitHub Actions workflow (`.github/workflows/build-deb.yml`) that automatically builds `.deb` packages for both x86_64 and ARM64 architectures on every push to master or when tags are created.

### Downloading pre-built packages

1. Go to the [Actions tab](https://github.com/jasonwitty/socktop/actions)
2. Click on the latest "Build Debian Packages" workflow run
3. Download the artifacts:
   - `debian-packages-amd64` - x86_64 packages
   - `debian-packages-arm64` - ARM64 packages
   - `all-debian-packages` - All packages combined
   - `checksums` - SHA256 checksums

### Release packages

When you create a git tag starting with `v` (e.g., `v1.50.0`), the workflow will automatically create a GitHub Release with all `.deb` packages attached.

```bash
git tag v1.50.0
git push origin v1.50.0
```

## Package Details

### socktop package

- **Binary**: `/usr/bin/socktop`
- **Documentation**: `/usr/share/doc/socktop/`
- **Size**: ~5-8 MB (depends on architecture)

### socktop_agent package

- **Binary**: `/usr/bin/socktop_agent`
- **Service**: `socktop-agent.service`
- **User/Group**: `socktop`
- **State directory**: `/var/lib/socktop`
- **Config directory**: `/etc/socktop` (created but empty by default)
- **Documentation**: `/usr/share/doc/socktop_agent/`
- **Size**: ~5-8 MB (depends on architecture)

## Uninstalling

```bash
# Remove packages but keep configuration
sudo apt remove socktop socktop_agent

# Remove packages and all configuration (purge)
sudo apt purge socktop socktop_agent
```

When purging `socktop_agent`, the following are removed:
- The `socktop` user and group
- `/var/lib/socktop` directory
- Empty `/etc/socktop` directory (if empty)

## Verifying Packages

Check package contents:

```bash
dpkg -c socktop_*.deb
dpkg -c socktop_agent_*.deb
```

Check package information:

```bash
dpkg -I socktop_*.deb
dpkg -I socktop_agent_*.deb
```

After installation, verify files:

```bash
dpkg -L socktop
dpkg -L socktop-agent
```

## Troubleshooting

### Service fails to start

Check logs:

```bash
sudo journalctl -u socktop-agent -f
```

Verify the socktop user exists:

```bash
id socktop
```

### Permission issues

Ensure the state directory has correct permissions:

```bash
sudo chown -R socktop:socktop /var/lib/socktop
sudo chmod 755 /var/lib/socktop
```

### Missing dependencies

If installation fails due to missing dependencies:

```bash
sudo apt --fix-broken install
```

## Creating a Local APT Repository (Advanced)

To create your own APT repository for easy installation:

1. Install required tools:
   ```bash
   sudo apt install dpkg-dev
   ```

2. Create repository structure:
   ```bash
   mkdir -p ~/socktop-repo/pool/main
   cp *.deb ~/socktop-repo/pool/main/
   ```

3. Generate package index:
   ```bash
   cd ~/socktop-repo
   dpkg-scanpackages pool/main /dev/null | gzip -9c > pool/main/Packages.gz
   ```

4. Serve via HTTP (for testing):
   ```bash
   cd ~/socktop-repo
   python3 -m http.server 8000
   ```

5. Add to sources on client machines:
   ```bash
   echo "deb [trusted=yes] http://your-server:8000 pool/main/" | \
     sudo tee /etc/apt/sources.list.d/socktop.list
   sudo apt update
   sudo apt install socktop socktop-agent
   ```

## Contributing

When adding new features that affect packaging:

1. Update `Cargo.toml` metadata in the `[package.metadata.deb]` section
2. Add new assets to the `assets` array if needed
3. Update maintainer scripts in `socktop_agent/debian/` if needed
4. Test package building locally before committing
5. Update this documentation

## References

- [cargo-deb documentation](https://github.com/kornelski/cargo-deb)
- [Debian Policy Manual](https://www.debian.org/doc/debian-policy/)
- [systemd service files](https://www.freedesktop.org/software/systemd/man/systemd.service.html)