# Cross-Compiling socktop_agent for Raspberry Pi

This guide explains how to cross-compile the socktop_agent on various host systems and deploy it to a Raspberry Pi. Cross-compiling is particularly useful for older or resource-constrained Pi models where native compilation might be slow.

## Cross-Compilation Host Setup

Choose your host operating system:

- [Debian/Ubuntu](#debianubuntu-based-systems)
- [Arch Linux](#arch-linux-based-systems)
- [macOS](#macos)
- [Windows](#windows)

## Debian/Ubuntu Based Systems

### Prerequisites

Install the cross-compilation toolchain for your target Raspberry Pi architecture:

```bash
# For 64-bit Raspberry Pi (aarch64)
sudo apt update
sudo apt install gcc-aarch64-linux-gnu libc6-dev-arm64-cross libdrm-dev:arm64

# For 32-bit Raspberry Pi (armv7)
sudo apt update
sudo apt install gcc-arm-linux-gnueabihf libc6-dev-armhf-cross libdrm-dev:armhf
```

### Setup Rust Cross-Compilation Targets

```bash
# For 64-bit Raspberry Pi
rustup target add aarch64-unknown-linux-gnu

# For 32-bit Raspberry Pi
rustup target add armv7-unknown-linux-gnueabihf
```

### Configure Cargo for Cross-Compilation

Create or edit `~/.cargo/config.toml`:

```toml
[target.aarch64-unknown-linux-gnu]
linker = "aarch64-linux-gnu-gcc"

[target.armv7-unknown-linux-gnueabihf]
linker = "arm-linux-gnueabihf-gcc"
```

## Arch Linux Based Systems

### Prerequisites

Install the cross-compilation toolchain using pacman and AUR:

```bash
# Install base dependencies
sudo pacman -S base-devel

# For 64-bit Raspberry Pi (aarch64)
sudo pacman -S aarch64-linux-gnu-gcc
# Install libdrm for aarch64 using an AUR helper (e.g., yay, paru)
yay -S aarch64-linux-gnu-libdrm

# For 32-bit Raspberry Pi (armv7)
sudo pacman -S arm-linux-gnueabihf-gcc
# Install libdrm for armv7 using an AUR helper
yay -S arm-linux-gnueabihf-libdrm
```

### Setup Rust Cross-Compilation Targets

```bash
# For 64-bit Raspberry Pi
rustup target add aarch64-unknown-linux-gnu

# For 32-bit Raspberry Pi
rustup target add armv7-unknown-linux-gnueabihf
```

### Configure Cargo for Cross-Compilation

Create or edit `~/.cargo/config.toml`:

```toml
[target.aarch64-unknown-linux-gnu]
linker = "aarch64-linux-gnu-gcc"

[target.armv7-unknown-linux-gnueabihf]
linker = "arm-linux-gnueabihf-gcc"
```

## macOS

The recommended approach for cross-compiling from macOS is to use Docker:

```bash
# Install Docker
brew install --cask docker

# Pull a cross-compilation Docker image
docker pull messense/rust-musl-cross:armv7-musleabihf  # For 32-bit Pi
docker pull messense/rust-musl-cross:aarch64-musl      # For 64-bit Pi
```

### Using Docker for Cross-Compilation

```bash
# Navigate to your socktop project directory
cd path/to/socktop

# For 64-bit Raspberry Pi
docker run --rm -it -v "$(pwd)":/home/rust/src messense/rust-musl-cross:aarch64-musl cargo build --release --target aarch64-unknown-linux-musl -p socktop_agent

# For 32-bit Raspberry Pi
docker run --rm -it -v "$(pwd)":/home/rust/src messense/rust-musl-cross:armv7-musleabihf cargo build --release --target armv7-unknown-linux-musleabihf -p socktop_agent
```

The compiled binaries will be available in your local target directory.

## Windows

The recommended approach for Windows is to use Windows Subsystem for Linux (WSL2):

1. Install WSL2 with a Debian/Ubuntu distribution by following the [official Microsoft documentation](https://docs.microsoft.com/en-us/windows/wsl/install).

2. Once WSL2 is set up with a Debian/Ubuntu distribution, open your WSL terminal and follow the [Debian/Ubuntu instructions](#debianubuntu-based-systems) above.

## Cross-Compile the Agent

After setting up your environment, build the socktop_agent for your target Raspberry Pi:

```bash
# For 64-bit Raspberry Pi
cargo build --release --target aarch64-unknown-linux-gnu -p socktop_agent

# For 32-bit Raspberry Pi
cargo build --release --target armv7-unknown-linux-gnueabihf -p socktop_agent
```

## Transfer the Binary to Your Raspberry Pi

Use SCP to transfer the compiled binary to your Raspberry Pi:

```bash
# For 64-bit Raspberry Pi
scp target/aarch64-unknown-linux-gnu/release/socktop_agent pi@raspberry-pi-ip:~/

# For 32-bit Raspberry Pi
scp target/armv7-unknown-linux-gnueabihf/release/socktop_agent pi@raspberry-pi-ip:~/
```

Replace `raspberry-pi-ip` with your Raspberry Pi's IP address and `pi` with your username.

## Install Dependencies on the Raspberry Pi

SSH into your Raspberry Pi and install the required dependencies:

```bash
ssh pi@raspberry-pi-ip

# For Raspberry Pi OS (Debian-based)
sudo apt update
sudo apt install libdrm-dev libdrm-amdgpu1

# For Arch Linux ARM
sudo pacman -Syu
sudo pacman -S libdrm
```

## Make the Binary Executable and Install

```bash
chmod +x ~/socktop_agent

# Optional: Install system-wide
sudo install -o root -g root -m 0755 ~/socktop_agent /usr/local/bin/socktop_agent

# Optional: Set up as a systemd service
sudo install -o root -g root -m 0644 ~/socktop-agent.service /etc/systemd/system/socktop-agent.service
sudo systemctl daemon-reload
sudo systemctl enable --now socktop-agent
```

## Troubleshooting

If you encounter issues with the cross-compiled binary:

1. **Incorrect Architecture**: Ensure you've chosen the correct target for your Raspberry Pi model:
   - For Raspberry Pi 2: use `armv7-unknown-linux-gnueabihf`
   - For Raspberry Pi 3/4/5 in 64-bit mode: use `aarch64-unknown-linux-gnu`
   - For Raspberry Pi 3/4/5 in 32-bit mode: use `armv7-unknown-linux-gnueabihf`

2. **Dependency Issues**: Check for missing libraries:
   ```bash
   ldd ~/socktop_agent
   ```

3. **Run with Backtrace**: Get detailed error information:
   ```bash
   RUST_BACKTRACE=1 ~/socktop_agent
   ```
