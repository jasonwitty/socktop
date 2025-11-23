# socktop APT Repository

This repository contains Debian packages for socktop and socktop-agent.

## Adding this repository

Add the repository to your system:

```bash
# Add the GPG key
curl -fsSL https://jasonwitty.github.io/socktop/KEY.gpg | sudo gpg --dearmor -o /usr/share/keyrings/socktop-archive-keyring.gpg

# Add the repository
echo "deb [signed-by=/usr/share/keyrings/socktop-archive-keyring.gpg] https://jasonwitty.github.io/socktop stable main" | sudo tee /etc/apt/sources.list.d/socktop.list

# Update and install
sudo apt update
sudo apt install socktop socktop-agent
```

## Manual Installation

You can also download and install packages manually from the `pool/main/` directory.

```bash
wget https://jasonwitty.github.io/socktop/pool/main/socktop_VERSION_ARCH.deb
sudo dpkg -i socktop_VERSION_ARCH.deb
```

## Supported Architectures

- amd64 (x86_64)
- arm64 (aarch64)
- armhf (32-bit ARM)

## Building from Source

See the main repository at https://github.com/jasonwitty/socktop
