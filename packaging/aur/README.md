# AUR packages

Two packages for Arch Linux live here. Each folder holds the `PKGBUILD` and the `.SRCINFO`
that the AUR needs.

- `lemonup-bin` installs the release binary (`lemonup-linux-x64`, x86_64 only). It needs
  glibc 2.39 or newer, which current Arch has.
- `lemonup` builds the tagged source with cargo, runs the tests, and installs the result.
  It supports x86_64 and aarch64.

Both install `/usr/bin/lemonup` and conflict with each other.

## How they stay current

The `aur` job in `.github/workflows/release.yml` runs after a stable release (a `v*` tag without
a `-`) is published. It sets `pkgver` to the tag, fills in the new checksums with `updpkgsums`,
regenerates `.SRCINFO`, and pushes both packages to the AUR. Pre-releases are skipped.

Without the `AUR_SSH_PRIVATE_KEY` secret the job does nothing and does not fail.

## One-time setup (owner)

1. Create an account at https://aur.archlinux.org and add an SSH public key to its profile.
   Use a key made only for this, for example `ssh-keygen -t ed25519 -C aur-lemonup -f aur_key`.
2. In the GitHub repository settings, add these Actions secrets:
   - `AUR_SSH_PRIVATE_KEY`: the private key.
   - `AUR_USERNAME`: the name for the AUR commits.
   - `AUR_EMAIL`: the email for the AUR commits.
3. Create the packages on the AUR. The first push creates each one. From a folder with a
   copy of each package's `PKGBUILD` and `.SRCINFO`:

   ```
   git clone ssh://aur@aur.archlinux.org/lemonup-bin.git
   cp PKGBUILD .SRCINFO lemonup-bin/
   cd lemonup-bin && git add PKGBUILD .SRCINFO && git commit -m "Initial import" && git push
   ```

   Do the same for `lemonup`. After that, releases need no manual work. The workflow
   would also create the packages on its first run, so this step is optional.

## Checking a package by hand

Copy the folder somewhere temporary and run `makepkg -f` there. Do not build inside the
repository, because `makepkg` writes the sources and the package next to the `PKGBUILD`.
After editing a `PKGBUILD`, refresh the metadata with `makepkg --printsrcinfo > .SRCINFO`.
