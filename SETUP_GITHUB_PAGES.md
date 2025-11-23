# Setting Up GitHub Pages for socktop APT Repository

This guide walks you through the initial setup of your APT repository on GitHub Pages using the `gh-pages` branch.

## Prerequisites

- [ ] You've run `./scripts/setup-apt-repo.sh` or manually created `apt-repo/`
- [ ] `apt-repo/` contains signed packages and metadata
- [ ] You have a GitHub repository for socktop

## Step-by-Step Setup

### 1. Verify Your Local Repository

First, make sure everything is ready:

```bash
# Check that apt-repo exists and has content
ls -la apt-repo/

# You should see:
# - dists/stable/Release, Release.gpg, InRelease
# - pool/main/*.deb
# - KEY.gpg
# - index.html, README.md
```

### 2. Create and Switch to gh-pages Branch

```bash
# Create a new orphan branch (no history from main)
git checkout --orphan gh-pages

# Remove all files from staging
git rm -rf .
```

**Important:** This creates a completely separate branch. Don't worry - your main branch is safe!

### 3. Copy APT Repository to Root

```bash
# Copy CONTENTS of apt-repo to root of gh-pages
cp -r apt-repo/* .

# Remove the apt-repo directory itself
rm -rf apt-repo

# Verify the structure
ls -la

# You should see in the current directory:
# - dists/
# - pool/
# - KEY.gpg
# - index.html
# - README.md
```

**Why root?** GitHub Pages can only serve from:
- `/` (root) - what we're doing
- `/docs` directory
- NOT from custom directories like `/apt-repo`

### 4. Commit and Push

```bash
# Add all files
git add .

# Commit
git commit -m "Initialize APT repository for GitHub Pages"

# Push to gh-pages branch
git push -u origin gh-pages
```

### 5. Return to Main Branch

```bash
# Switch back to your main development branch
git checkout main

# Verify you're back on main
git branch
# Should show: * main
```

### 6. Enable GitHub Pages

1. Go to your repository on GitHub
2. Click **Settings** (top right)
3. Click **Pages** (left sidebar)
4. Under "Build and deployment":
   - Source: **Deploy from a branch**
   - Branch: **gh-pages** 
   - Folder: **/ (root)**
   - Click **Save**

### 7. Wait for Deployment

GitHub will deploy your site. This usually takes 1-2 minutes.

You can watch the progress:
- Go to **Actions** tab
- Look for "pages build and deployment" workflow

### 8. Verify Your Repository is Live

Once deployed, your repository will be at:

```
https://YOUR-USERNAME.github.io/socktop/
```

Test it:

```bash
# Check the public key is accessible
curl -I https://YOUR-USERNAME.github.io/socktop/KEY.gpg

# Should return: HTTP/2 200

# Check the Release file
curl -I https://YOUR-USERNAME.github.io/socktop/dists/stable/Release

# Should return: HTTP/2 200
```

### 9. Test Installation (Optional but Recommended)

On a Debian/Ubuntu VM or system:

```bash
# Add GPG key
curl -fsSL https://YOUR-USERNAME.github.io/socktop/KEY.gpg | \
    sudo gpg --dearmor -o /usr/share/keyrings/socktop-archive-keyring.gpg

# Add repository
echo "deb [signed-by=/usr/share/keyrings/socktop-archive-keyring.gpg] https://YOUR-USERNAME.github.io/socktop stable main" | \
    sudo tee /etc/apt/sources.list.d/socktop.list

# Update package lists
sudo apt update

# You should see:
# Get:1 https://YOUR-USERNAME.github.io/socktop stable InRelease [xxx B]

# Install packages
sudo apt install socktop socktop-agent

# Verify
socktop --version
```

## Understanding the Two Branches

After setup, you'll have two branches:

### `main` branch (development)
```
main/
├── src/
├── Cargo.toml
├── scripts/
├── docs/
├── apt-repo/          ← Local build artifact (not published)
└── ...
```

**Purpose:** Source code, development, building packages

### `gh-pages` branch (published)
```
gh-pages/
├── dists/
├── pool/
├── KEY.gpg
├── index.html         ← Customize this for a nice landing page!
└── README.md
```

**Purpose:** Published APT repository served by GitHub Pages

## Workflow Going Forward

### Manual Updates

When you release a new version:

```bash
# 1. On main branch, build new packages
git checkout main
cargo deb --package socktop
cargo deb --package socktop_agent

# 2. Update local apt-repo
./scripts/add-package-to-repo.sh target/debian/socktop_*.deb
./scripts/add-package-to-repo.sh target/debian/socktop-agent_*.deb
./scripts/sign-apt-repo.sh apt-repo stable YOUR-GPG-KEY-ID

# 3. Switch to gh-pages and update
git checkout gh-pages
cp -r apt-repo/* .
git add .
git commit -m "Release v1.51.0"
git push origin gh-pages

# 4. Return to main
git checkout main
```

### Automated Updates (Recommended)

Set up GitHub Actions to do this automatically:

1. Add GitHub Secrets (Settings → Secrets → Actions):
   - `GPG_PRIVATE_KEY` - Your exported private key
   - `GPG_KEY_ID` - Your GPG key ID
   - `GPG_PASSPHRASE` - Your GPG passphrase (if any)

2. Tag and push:
   ```bash
   git tag v1.51.0
   git push origin main --tags
   ```

3. GitHub Actions will automatically:
   - Build packages for AMD64 and ARM64
   - Update apt-repo
   - Sign with your GPG key
   - Push to gh-pages
   - Create GitHub Release

See `.github/workflows/publish-apt-repo.yml` for details.

## Customizing Your GitHub Pages Site

The `gh-pages` branch contains `index.html` which users see when they visit:
`https://YOUR-USERNAME.github.io/socktop/`

You can customize this! On the `gh-pages` branch:

```bash
git checkout gh-pages

# Edit index.html
nano index.html

# Add features, badges, screenshots, etc.

git add index.html
git commit -m "Improve landing page"
git push origin gh-pages

git checkout main
```

## Troubleshooting

### "404 Not Found" on GitHub Pages

**Check:**
- Settings → Pages shows "Your site is live at..."
- Wait 2-3 minutes after pushing
- Verify branch is `gh-pages` and folder is `/`
- Check Actions tab for deployment errors

### "Repository not found" when installing

**Check:**
- URL is correct: `https://USERNAME.github.io/REPO/` (no trailing /apt-repo)
- Files exist at the URLs:
  ```bash
  curl -I https://USERNAME.github.io/REPO/dists/stable/InRelease
  curl -I https://USERNAME.github.io/REPO/KEY.gpg
  ```

### "GPG error" when installing

**Check:**
- Repository is signed: `ls gh-pages/dists/stable/Release.gpg`
- Users imported the key: `curl https://USERNAME.github.io/REPO/KEY.gpg | gpg --import`

### Changes not appearing

**Check:**
- You committed and pushed to `gh-pages` (not `main`)
- Wait 1-2 minutes for GitHub to redeploy
- Clear browser cache if viewing index.html
- For apt: `sudo apt clean && sudo apt update`

## Success Checklist

After completing this guide, you should have:

- [ ] `gh-pages` branch created and pushed
- [ ] GitHub Pages enabled and deployed
- [ ] Site accessible at `https://USERNAME.github.io/socktop/`
- [ ] `KEY.gpg` downloadable
- [ ] `dists/stable/InRelease` accessible
- [ ] Packages in `pool/main/*.deb` downloadable
- [ ] Successfully tested installation on a test system
- [ ] Understand the workflow for future updates

## Next Steps

1. **Update your main README.md** with installation instructions
2. **Set up GitHub Actions** for automated releases
3. **Customize index.html** on gh-pages for a nice landing page
4. **Test on multiple architectures** (AMD64, ARM64)
5. **Share your repository** with users

## Quick Reference

**Switch branches:**
```bash
git checkout main        # Development
git checkout gh-pages    # Published site
```

**Update published site manually:**
```bash
git checkout main
# ... build packages, update apt-repo ...
git checkout gh-pages
cp -r apt-repo/* .
git add . && git commit -m "Update" && git push
git checkout main
```

**Your repository URL:**
```
https://YOUR-USERNAME.github.io/socktop/
```

**User installation command:**
```bash
curl -fsSL https://YOUR-USERNAME.github.io/socktop/KEY.gpg | \
    sudo gpg --dearmor -o /usr/share/keyrings/socktop-archive-keyring.gpg
echo "deb [signed-by=/usr/share/keyrings/socktop-archive-keyring.gpg] https://YOUR-USERNAME.github.io/socktop stable main" | \
    sudo tee /etc/apt/sources.list.d/socktop.list
sudo apt update && sudo apt install socktop socktop-agent
```

---

**Need help?** See:
- `QUICK_START_APT_REPO.md` - Overall quick start
- `docs/APT_REPOSITORY.md` - Comprehensive guide
- `docs/APT_WORKFLOW.md` - Visual workflow diagrams