# APT Repository Setup Summary

## 🎉 What You Now Have

You now have a complete system for creating and hosting your own APT repository for socktop packages, **without needing a sponsor or official Debian/Ubuntu approval**.

## 📁 Files Created

### Scripts (in `scripts/`)
- **`init-apt-repo.sh`** - Initializes the APT repository directory structure
- **`add-package-to-repo.sh`** - Adds .deb packages to the repository and generates metadata
- **`sign-apt-repo.sh`** - Signs the repository with your GPG key
- **`setup-apt-repo.sh`** - All-in-one interactive wizard to set everything up

### Documentation
- **`QUICK_START_APT_REPO.md`** - Quick start guide (< 10 minutes)
- **`docs/APT_REPOSITORY.md`** - Comprehensive 600+ line guide covering everything
- **`APT_REPO_SUMMARY.md`** - This file

### GitHub Actions
- **`.github/workflows/publish-apt-repo.yml`** - Automated building, signing, and publishing

## 🚀 Quick Start (Choose One)

### Option 1: Interactive Setup (Recommended for First Time)

Run the setup wizard:

```bash
./scripts/setup-apt-repo.sh
```

This walks you through:
1. ✅ Checking prerequisites
2. 🔑 Setting up GPG key
3. 📦 Finding/building packages
4. 📝 Creating repository structure
5. ✍️ Signing the repository
6. 📋 Next steps to publish to gh-pages

### Option 2: Manual Step-by-Step

```bash
# 1. Initialize
./scripts/init-apt-repo.sh

# 2. Build packages
cargo deb --package socktop
cargo deb --package socktop_agent

# 3. Add packages
./scripts/add-package-to-repo.sh target/debian/socktop_*.deb
./scripts/add-package-to-repo.sh target/debian/socktop-agent_*.deb

# 4. Sign (replace YOUR-KEY-ID)
./scripts/sign-apt-repo.sh apt-repo stable YOUR-KEY-ID

# 5. Update URLs
sed -i 's/YOUR-USERNAME/your-github-username/g' apt-repo/*.{md,html}

# 6. Publish to gh-pages (see below)
```

### Option 3: Fully Automated (After Initial Setup)

Once gh-pages branch exists, just tag releases:

```bash
git tag v1.50.0
git push --tags

# GitHub Actions will:
# - Build packages for AMD64 and ARM64
# - Update APT repository
# - Sign with your GPG key
# - Push to gh-pages branch automatically
```

## 📤 Publishing to GitHub Pages (gh-pages branch)

**Why gh-pages branch?**
- ✅ Keeps main branch clean (source code only)
- ✅ Separate branch for published content  
- ✅ GitHub Actions can auto-update it
- ✅ You can customize the landing page

**Initial Setup:**
```bash
# Create gh-pages branch
git checkout --orphan gh-pages
git rm -rf .

# Copy apt-repo CONTENTS to root (not the folder!)
cp -r apt-repo/* .
rm -rf apt-repo

# Commit and push
git add .
git commit -m "Initialize APT repository"
git push -u origin gh-pages

# Return to main
git checkout main
```

**Enable in GitHub:**
1. Settings → Pages
2. Source: **gh-pages** → **/ (root)**
3. Save

Your repo will be at: `https://your-username.github.io/socktop/`

**Note:** GitHub Pages only allows `/` (root) or `/docs`. Since we use gh-pages branch, contents go in the root of that branch.

See `SETUP_GITHUB_PAGES.md` for detailed step-by-step instructions.

### Alternative: Self-Hosted Server

Copy `apt-repo/` contents to your web server:
```bash
rsync -avz apt-repo/ user@example.com:/var/www/apt/
```

Configure Apache/Nginx to serve the directory. See `docs/APT_REPOSITORY.md` for details.

## 🤖 GitHub Actions Automation

### Required Secrets

Add these in GitHub Settings → Secrets → Actions:

1. **GPG_PRIVATE_KEY**
   ```bash
   gpg --armor --export-secret-key YOUR-KEY-ID
   # Copy entire output including BEGIN/END lines
   ```

2. **GPG_KEY_ID**
   ```bash
   gpg --list-secret-keys --keyid-format LONG
   # Use the ID after "rsa4096/"
   ```

3. **GPG_PASSPHRASE**
   ```bash
   # Your GPG passphrase (leave empty if no passphrase)
   ```

### Triggers

The workflow runs on:
- **Version tags**: `git tag v1.50.0 && git push --tags`
- **Manual dispatch**: Actions tab → "Publish APT Repository" → Run workflow

### What It Does

1. ✅ Builds packages for AMD64 and ARM64
2. ✅ Initializes or updates APT repository
3. ✅ Generates Packages files and metadata
4. ✅ Signs with your GPG key
5. ✅ Commits and pushes to gh-pages branch
6. ✅ Creates GitHub Release with artifacts
7. ✅ Generates summary with installation instructions

## 👥 User Installation

Once published, users install with:

```bash
# Add repository
curl -fsSL https://your-username.github.io/socktop/KEY.gpg | \
    sudo gpg --dearmor -o /usr/share/keyrings/socktop-archive-keyring.gpg

echo "deb [signed-by=/usr/share/keyrings/socktop-archive-keyring.gpg] https://your-username.github.io/socktop stable main" | \
    sudo tee /etc/apt/sources.list.d/socktop.list

# Install
sudo apt update
sudo apt install socktop socktop-agent

# The agent service is automatically installed and configured
sudo systemctl enable --now socktop-agent
```

## 🔧 Maintenance

### Release New Version (Automated)

```bash
# Update version in Cargo.toml, commit changes
git add . && git commit -m "Bump version to 1.51.0"
git tag v1.51.0
git push origin main --tags

# GitHub Actions automatically:
# - Builds packages for AMD64 and ARM64
# - Updates apt-repo
# - Signs with GPG
# - Pushes to gh-pages branch
```

### Manual Update (if needed)

```bash
# On main branch
cargo deb --package socktop
./scripts/add-package-to-repo.sh target/debian/socktop_*.deb
./scripts/sign-apt-repo.sh

# Switch to gh-pages and update
git checkout gh-pages
cp -r apt-repo/* .
git add . && git commit -m "Release v1.51.0" && git push
git checkout main
```

### Remove Old Versions

```bash
# On gh-pages branch
git checkout gh-pages
rm pool/main/socktop_1.50.0_*.deb
# Regenerate metadata (re-add remaining packages)
git add . && git commit -m "Remove old versions" && git push
git checkout main
```

## 🎯 Key Benefits

✅ **No sponsor needed** - Host your own repository  
✅ **Full control** - You decide when to release  
✅ **Free hosting** - GitHub Pages at no cost  
✅ **Automated** - GitHub Actions does the work  
✅ **Professional** - Just like official repos  
✅ **Multi-arch** - AMD64, ARM64 support built-in  
✅ **Secure** - GPG signed packages  
✅ **Easy updates** - Users get updates via `apt upgrade`  

## 📊 Repository Structure

```
apt-repo/
├── dists/
│   └── stable/
│       ├── Release              # Main metadata (checksums)
│       ├── Release.gpg          # Detached signature
│       ├── InRelease            # Clearsigned release
│       └── main/
│           ├── binary-amd64/
│           │   ├── Packages     # Package list
│           │   ├── Packages.gz  # Compressed
│           │   └── Release      # Component metadata
│           ├── binary-arm64/
│           └── binary-armhf/
├── pool/
│   └── main/
│       ├── socktop_1.50.0_amd64.deb
│       ├── socktop-agent_1.50.1_amd64.deb
│       ├── socktop_1.50.0_arm64.deb
│       └── socktop-agent_1.50.1_arm64.deb
├── KEY.gpg                      # Public GPG key
├── README.md                    # Repository info
├── index.html                   # Web interface
└── packages.html                # Package listing
```

## 🔑 GPG Key Management

### Create New Key

```bash
gpg --full-generate-key
# Choose RSA 4096, no expiration (or 2 years)
```

### Export Keys

```bash
# Public key (for users)
gpg --armor --export YOUR-KEY-ID > KEY.gpg

# Private key (for GitHub Secrets)
gpg --armor --export-secret-key YOUR-KEY-ID
```

### Backup Keys

```bash
# Backup to safe location
gpg --export-secret-keys YOUR-KEY-ID > gpg-private-backup.key
gpg --export YOUR-KEY-ID > gpg-public-backup.key
```

### Key Rotation

If your key expires or is compromised:
```bash
./scripts/sign-apt-repo.sh apt-repo stable NEW-KEY-ID
gpg --armor --export NEW-KEY-ID > apt-repo/KEY.gpg
# Users need to re-import the key
```

## 🐛 Troubleshooting

### "Repository not signed"
```bash
./scripts/sign-apt-repo.sh apt-repo stable YOUR-KEY-ID
ls apt-repo/dists/stable/Release*  # Should show 3 files
```

### "Package not found"
```bash
cd apt-repo
dpkg-scanpackages --arch amd64 pool/main /dev/null > dists/stable/main/binary-amd64/Packages
gzip -9 -k -f dists/stable/main/binary-amd64/Packages
cd ..
./scripts/sign-apt-repo.sh
```

### "404 Not Found" on GitHub Pages
- Wait 2-3 minutes after pushing
- Check Settings → Pages is enabled
- Verify source branch/directory

### GitHub Actions not signing
- Check all 3 secrets are set correctly
- GPG_PRIVATE_KEY must include BEGIN/END lines
- Test signing locally first

## 📚 Documentation

| File | Purpose | Length |
|------|---------|--------|
| `QUICK_START_APT_REPO.md` | Get started in < 10 minutes | Quick |
| `SETUP_GITHUB_PAGES.md` | Detailed gh-pages setup guide | Step-by-step |
| `docs/APT_REPOSITORY.md` | Complete guide with all options | Comprehensive |
| `docs/DEBIAN_PACKAGING.md` | How .deb packages are built | Technical |
| `DEBIAN_PACKAGING_SUMMARY.md` | Overview of packaging work | Summary |
| `APT_REPO_SUMMARY.md` | This file | Overview |

## 🎓 Learning Path

1. **Start here**: `QUICK_START_APT_REPO.md` (10 min)
2. **Set up**: Run `./scripts/setup-apt-repo.sh` (15 min)
3. **Publish**: Follow `SETUP_GITHUB_PAGES.md` (5 min)
4. **Automate**: Set up GitHub Actions secrets (10 min)
5. **Advanced**: Read `docs/APT_REPOSITORY.md` as needed

## 🚦 Next Steps

Choose your path:

### Just Getting Started?
1. ✅ Read `QUICK_START_APT_REPO.md`
2. ✅ Run `./scripts/setup-apt-repo.sh`
3. ✅ Follow `SETUP_GITHUB_PAGES.md` to publish
4. ✅ Test installation on a VM

### Want Automation?
1. ✅ Generate/export GPG key
2. ✅ Add GitHub Secrets
3. ✅ Tag a release: `git tag v1.50.0 && git push --tags`
4. ✅ Watch GitHub Actions magic happen

### Want to Understand Everything?
1. ✅ Read `docs/APT_REPOSITORY.md` (comprehensive)
2. ✅ Study the scripts in `scripts/`
3. ✅ Examine `.github/workflows/publish-apt-repo.yml`
4. ✅ Learn about Debian repository format

### Ready for Production?
1. ✅ Set up monitoring/analytics
2. ✅ Create PPA for Ubuntu (Launchpad)
3. ✅ Apply to Debian mentors for official inclusion
4. ✅ Set up repository mirrors
5. ✅ Document best practices for users

## 🌟 Success Criteria

You'll know you're successful when:

- [ ] Users can `apt install socktop`
- [ ] Updates work with `apt upgrade`
- [ ] Multiple architectures supported
- [ ] Repository is GPG signed
- [ ] GitHub Actions publishes automatically
- [ ] Installation instructions in README
- [ ] Zero sponsor or approval needed

## 💡 Pro Tips

1. **Test first**: Always test on a fresh VM before publishing
2. **Keep versions**: Don't delete old .deb files immediately
3. **Backup GPG key**: Store it safely offline
4. **Monitor downloads**: Use GitHub Insights or server logs
5. **Document everything**: Help users troubleshoot
6. **Version consistently**: Use semantic versioning
7. **Sign always**: Never publish unsigned repositories

## 🔗 Resources

- [Debian Repository Format](https://wiki.debian.org/DebianRepository/Format)
- [GitHub Pages Docs](https://docs.github.com/en/pages)
- [cargo-deb](https://github.com/kornelski/cargo-deb)
- [Ubuntu PPA Guide](https://help.launchpad.net/Packaging/PPA)
- [Debian Mentors](https://mentors.debian.net/)

## 🎊 Congratulations!

You now have everything you need to:
- ✅ Create your own APT repository
- ✅ Host it for free on GitHub Pages
- ✅ Automate the entire process
- ✅ Distribute packages professionally
- ✅ Provide easy installation for users

**No sponsor required. No approval needed. You're in control!** 🚀

---

**Questions?** Check the docs or open an issue.

**Ready to publish?** Run `./scripts/setup-apt-repo.sh` and follow the wizard!