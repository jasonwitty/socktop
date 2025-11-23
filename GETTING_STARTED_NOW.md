# 🚀 Get Your APT Repository Live in 5 Minutes

## You're Here Because...

You want to publish socktop packages via APT, but GitHub Pages won't let you select `apt-repo/` folder. Here's why and how to fix it:

**The Issue:** GitHub Pages only serves from `/` (root) or `/docs`, not custom folders like `/apt-repo`.

**The Solution:** Use a `gh-pages` branch where `apt-repo` contents go in the root.

## Quick Setup (5 Steps)

### 1. Create apt-repo locally (if you haven't)

```bash
./scripts/setup-apt-repo.sh
```

This creates `apt-repo/` with your packages and signs them.

### 2. Create gh-pages branch

```bash
git checkout --orphan gh-pages
git rm -rf .
```

### 3. Copy apt-repo to root

```bash
cp -r apt-repo/* .
rm -rf apt-repo
ls
# You should see: dists/ pool/ KEY.gpg index.html README.md
```

### 4. Push to GitHub

```bash
git add .
git commit -m "Initialize APT repository"
git push -u origin gh-pages
git checkout main
```

### 5. Enable GitHub Pages

1. Go to: **Settings → Pages**
2. Source: **gh-pages** → **/ (root)**
3. Click **Save**

**Done!** ✅ Your repo will be live at `https://your-username.github.io/socktop/` in 1-2 minutes.

## Test It

```bash
curl -I https://your-username.github.io/socktop/KEY.gpg
# Should return: HTTP/2 200
```

## Install It (On Any Debian/Ubuntu System)

```bash
curl -fsSL https://your-username.github.io/socktop/KEY.gpg | \
    sudo gpg --dearmor -o /usr/share/keyrings/socktop-archive-keyring.gpg

echo "deb [signed-by=/usr/share/keyrings/socktop-archive-keyring.gpg] https://your-username.github.io/socktop stable main" | \
    sudo tee /etc/apt/sources.list.d/socktop.list

sudo apt update
sudo apt install socktop socktop-agent
```

## What's Next?

### Now (Optional):
- Customize `index.html` on gh-pages for a nice landing page
- Add installation instructions to your main README

### Later:
- Set up GitHub Actions automation (see `QUICK_START_APT_REPO.md`)
- Add more architectures (ARM64, ARMv7)

## Understanding the Setup

```
main branch:               gh-pages branch:
├── src/                   ├── dists/
├── Cargo.toml             ├── pool/
├── scripts/               ├── KEY.gpg
└── apt-repo/ (local)      └── index.html ← GitHub Pages serves this
                           
Work here ↑                Published here ↑
```

- **main**: Your development work
- **gh-pages**: What users see/download
- **apt-repo/**: Local folder (ignored in git, see `.gitignore`)

## Need More Help?

- **Quick start**: `QUICK_START_APT_REPO.md`
- **Detailed setup**: `SETUP_GITHUB_PAGES.md`
- **Why gh-pages?**: `WHY_GHPAGES_BRANCH.md`
- **Full guide**: `docs/APT_REPOSITORY.md`

---

**You got this!** 🎉
