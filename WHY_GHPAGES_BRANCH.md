# Why We Use the gh-pages Branch

## The Problem

GitHub Pages has a limitation - it can only serve static sites from:

1. **`/` (root)** of a branch
2. **`/docs`** directory of a branch
3. **NOT** from custom directories like `/apt-repo`

## Why Not `/docs`?

When you tried to enable GitHub Pages with `apt-repo/` checked into main, you couldn't select it because:

```
main/
├── src/
├── Cargo.toml
├── apt-repo/          ← GitHub Pages can't serve from here!
└── ...
```

You could move it to `/docs`:
```
main/
├── src/
├── Cargo.toml
├── docs/              ← GitHub Pages CAN serve from here
│   ├── dists/
│   ├── pool/
│   └── ...
└── ...
```

But this has downsides:
- ❌ Mixed source code and published content
- ❌ Large .deb files bloat the main branch
- ❌ Can't easily customize the site without affecting source
- ❌ Messy git history with binary files

## Why gh-pages Branch (Our Solution)

Using a separate `gh-pages` branch is cleaner:

```
main branch (source code):
├── src/
├── Cargo.toml
├── scripts/
└── docs/             ← Documentation source

gh-pages branch (published):
├── dists/
├── pool/
├── KEY.gpg
├── index.html        ← Customizable landing page
└── README.md
```

### Benefits

✅ **Clean separation**: Source code stays in `main`, published content in `gh-pages`  
✅ **No binary bloat**: .deb files don't clutter your main branch history  
✅ **Easy automation**: GitHub Actions can push to gh-pages without affecting main  
✅ **Customizable**: You can make a beautiful landing page on gh-pages  
✅ **Standard practice**: Most GitHub Pages projects use gh-pages branch  
✅ **Root URL**: Your repo is at `https://username.github.io/socktop/` (not `/apt-repo`)  

### Workflow

```
Developer (main branch)
    ↓
  Build packages
    ↓
  Update apt-repo/ (local)
    ↓
  Push to gh-pages branch
    ↓
  GitHub Pages serves
    ↓
  Users: apt install socktop
```

## The Setup

**One-time setup:**
```bash
git checkout --orphan gh-pages
git rm -rf .
cp -r apt-repo/* .
rm -rf apt-repo
git add . && git commit -m "Initialize APT repository"
git push -u origin gh-pages
git checkout main
```

**Going forward:**
- Work on `main` branch for development
- `gh-pages` branch gets updated by GitHub Actions (or manually)
- Never need to switch branches manually after automation is set up!

## Comparison

| Approach | Location | URL | Pros | Cons |
|----------|----------|-----|------|------|
| **gh-pages branch** ✅ | gh-pages:/ | `username.github.io/socktop/` | Clean, automated, customizable | Two branches |
| `/docs` on main | main:/docs | `username.github.io/socktop/` | One branch | Mixed content, binary bloat |
| `/apt-repo` on main | main:/apt-repo | ❌ Not possible | - | GitHub Pages won't allow it |

## Conclusion

The `gh-pages` branch approach is:
- The **cleanest** solution
- The **most flexible** for customization
- The **easiest to automate**
- **Industry standard** for GitHub Pages

That's why we chose it! 🚀
