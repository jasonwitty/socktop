# Quick Start: Setting Up Your socktop APT Repository

This guide will get your APT repository up and running in **under 10 minutes**.

## Prerequisites

- [ ] Debian packages built (or use GitHub Actions to build them)
- [ ] GPG key for signing
- [ ] GitHub repository with Pages enabled

## Step 1: Create GPG Key (if needed)

```bash
# Generate a new key
gpg --full-generate-key

# Select:
# - RSA and RSA (default)
# - 4096 bits
# - Key does not expire (or 2 years)
# - Your name and email

# Get your key ID
gpg --list-secret-keys --keyid-format LONG
# Look for the part after "rsa4096/" - that's your KEY-ID
```

## Step 2: Initialize Repository Locally

```bash
cd socktop

# Create the repository structure
./scripts/init-apt-repo.sh

# Build packages (or download from GitHub Actions)
cargo install cargo-deb
cargo deb --package socktop
cargo deb --package socktop_agent

# Add packages to repository
./scripts/add-package-to-repo.sh target/debian/socktop_*.deb
./scripts/add-package-to-repo.sh target/debian/socktop-agent_*.deb

# Sign the repository (replace YOUR-KEY-ID with actual key ID)
./scripts/sign-apt-repo.sh apt-repo stable YOUR-KEY-ID

# Update URLs with your GitHub username
sed -i 's/YOUR-USERNAME/your-github-username/g' apt-repo/README.md apt-repo/index.html
```

## Step 3: Publish to GitHub Pages (gh-pages branch)

```bash
# Create gh-pages branch
git checkout --orphan gh-pages
git rm -rf .

# Copy apt-repo CONTENTS to root (not the folder itself)
cp -r apt-repo/* .
rm -rf apt-repo

# Commit and push
git add .
git commit -m "Initialize APT repository"
git push -u origin gh-pages

# Go back to main branch
git checkout main
```

Then in GitHub:
1. Go to **Settings → Pages**
2. Source: **Deploy from a branch**
3. Branch: **gh-pages** → **/ (root)** → **Save**

Wait 1-2 minutes, then visit: `https://your-username.github.io/socktop/`

## Step 4: Automate with GitHub Actions

Add these secrets to your repository (Settings → Secrets → Actions):

```bash
# Export your private key
gpg --armor --export-secret-key YOUR-KEY-ID

# Copy the ENTIRE output and save as secret: GPG_PRIVATE_KEY
```

Add these three secrets:
- **GPG_PRIVATE_KEY**: Your exported private key
- **GPG_KEY_ID**: Your key ID (e.g., `ABC123DEF456`)
- **GPG_PASSPHRASE**: Your key passphrase (leave empty if no passphrase)

The workflow in `.github/workflows/publish-apt-repo.yml` will now:
- Build packages for AMD64 and ARM64
- Update the APT repository
- Sign with your GPG key
- Push to gh-pages automatically

Trigger it by:
- Creating a version tag: `git tag v1.50.0 && git push --tags`
- Manual dispatch from GitHub Actions tab

## Step 5: Test It

On any Debian/Ubuntu system:

```bash
# Add your repository
curl -fsSL https://your-username.github.io/socktop/KEY.gpg | \
    sudo gpg --dearmor -o /usr/share/keyrings/socktop-archive-keyring.gpg

echo "deb [signed-by=/usr/share/keyrings/socktop-archive-keyring.gpg] https://your-username.github.io/socktop stable main" | \
    sudo tee /etc/apt/sources.list.d/socktop.list

# Install
sudo apt update
sudo apt install socktop socktop-agent

# Verify
socktop --version
socktop_agent --version
```

## Maintenance

### Add a New Version

```bash
# Build new packages
cargo deb --package socktop
cargo deb --package socktop_agent

# Add to repository
./scripts/add-package-to-repo.sh target/debian/socktop_*.deb
./scripts/add-package-to-repo.sh target/debian/socktop-agent_*.deb

# Re-sign
./scripts/sign-apt-repo.sh apt-repo stable YOUR-KEY-ID

# Publish
cd docs/apt  # or wherever your apt-repo is
git add .
git commit -m "Release v1.51.0"
git push origin main
```

### Or Just Tag and Let GitHub Actions Do It

```bash
# Update version in Cargo.toml
# Commit changes
git add .
git commit -m "Bump version to 1.51.0"

# Tag and push
git tag v1.51.0
git push origin main --tags

# GitHub Actions will:
# - Build packages for AMD64 and ARM64
# - Update gh-pages branch automatically
# - Sign and publish!
```

## Troubleshooting

### "Repository not signed" error

Make sure you signed it:
```bash
./scripts/sign-apt-repo.sh apt-repo stable YOUR-KEY-ID
ls apt-repo/dists/stable/Release*
# Should show: Release, Release.gpg, InRelease, KEY.gpg
```

### "404 Not Found" on GitHub Pages

1. Check Settings → Pages is enabled
2. Wait 2-3 minutes for GitHub to deploy
3. Verify the URL structure matches your settings

### GitHub Actions not signing

Check that all three secrets are set correctly:
- Settings → Secrets and variables → Actions
- Make sure GPG_PRIVATE_KEY includes the BEGIN/END lines
- Test locally first

## What's Next?

✅ You now have a working APT repository!

**Share it:**
- Add installation instructions to your main README
- Tweet/blog about it
- Submit to awesome-rust lists

**Improve it:**
- Customize your GitHub Pages site (it's just HTML!)
- Add more architectures (ARMv7)
- Create multiple distributions (stable, testing)
- Set up download statistics
- Apply to Ubuntu PPA (Launchpad)
- Eventually submit to official Debian repos

## Full Documentation

For detailed information, see:
- `docs/APT_REPOSITORY.md` - Complete APT repository guide
- `docs/DEBIAN_PACKAGING.md` - Debian packaging details
- `DEBIAN_PACKAGING_SUMMARY.md` - Quick summary

## Questions?

Open an issue on GitHub or check the full documentation.

---

**Happy packaging! 📦**