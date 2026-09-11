# Packaging

`stage.sh` lays out every installed file under a root directory. The Arch package and the portable archive both call it, so a file added there ships in both.

| File | Purpose |
|---|---|
| `arch/PKGBUILD`, `arch/omaasus.install` | The pacman package, ready for the AUR |
| `arch/build.sh` | Builds that package from `HEAD`, in CI or locally |
| `portable/install.sh` | The installer inside the portable archive |
| `make-tarball.sh` | Builds the portable archive and checks what the binaries link |
| `version.sh` | Checks that `Cargo.toml`, the PKGBUILD and a tag agree |
| `publish.sh`, `release-notes.md` | Create the GitHub release |

## Releasing

1. Set the version in `Cargo.toml` (`[workspace.package]`) and `arch/PKGBUILD` (`pkgver`, with `pkgrel=1`), then run `cargo update --workspace` to refresh `Cargo.lock`.
2. Merge to `main` once CI passes.
3. Tag the merge commit and push the tag:

   ```sh
   git tag -a v0.2.0 -m "OmaAsus 0.2.0"
   git push origin v0.2.0
   ```

The Release workflow checks the tag against both versions, runs clippy and the tests, builds the pacman package in an Arch container and the portable archive on Ubuntu 24.04, and publishes both with checksums and install notes. The notes end with the pull requests merged since the previous tag.

To build the package locally: `packaging/arch/build.sh /tmp/omaasus-pkg`. It builds `HEAD`, so commit first.

## AUR

The `PKGBUILD` attached to a release carries that release's checksum. makepkg downloads the source archive from the release, so the repository has to be public before the AUR package works.

```sh
git clone ssh://aur@aur.archlinux.org/omaasus.git && cd omaasus
cp ~/Downloads/PKGBUILD ~/Downloads/omaasus.install .
makepkg --printsrcinfo > .SRCINFO
git add . && git commit -m "omaasus 0.2.0" && git push
```
