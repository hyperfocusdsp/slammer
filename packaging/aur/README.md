# Slammer AUR package

The `PKGBUILD` in this directory is the source-of-truth for the Arch
User Repository entry. It is **not** auto-published — pushing to AUR is
a manual step (see below) so a maintainer always reviews the diff
before it goes live.

## First-time publish

1. Make sure you have an AUR account at https://aur.archlinux.org and
   your SSH public key registered under your account profile.
2. Clone the empty AUR namespace (this also reserves the name on first
   push):
   ```bash
   git clone ssh://aur@aur.archlinux.org/slammer.git ~/aur-slammer
   ```
3. Copy the `PKGBUILD` and generate `.SRCINFO`:
   ```bash
   cp packaging/aur/PKGBUILD ~/aur-slammer/
   cd ~/aur-slammer
   makepkg --printsrcinfo > .SRCINFO
   ```
4. Sanity-build in a clean chroot before publishing:
   ```bash
   makepkg -si  # or: extra-x86_64-build
   ```
5. Commit + push:
   ```bash
   git add PKGBUILD .SRCINFO
   git commit -m "Initial import: slammer 0.3.0-1"
   git push
   ```

## Bumping for a new release

1. Update `pkgver` in `packaging/aur/PKGBUILD` (in this repo).
2. Refresh the source hash:
   ```bash
   curl -sL https://github.com/hyperfocusdsp/slammer/archive/refs/tags/v<NEW>.tar.gz \
     | sha256sum
   ```
   Replace the value in `sha256sums=(...)`.
3. Commit the change here.
4. Sync into the AUR clone, regenerate `.SRCINFO`, test-build, push.
