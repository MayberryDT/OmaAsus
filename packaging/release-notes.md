## Install

Arch and Omarchy:

```sh
sudo pacman -U @PKG@
```

If you installed the helper earlier with `scripts/install-helper.sh`, pacman stops on files that already exist. Add `--overwrite '*'` the first time.

Other distributions (glibc @GLIBC@ or newer, systemd, polkit):

```sh
tar xf omaasus-@VERSION@-x86_64-linux.tar.gz
sudo omaasus-@VERSION@-x86_64-linux/install.sh
```

Run the same script with `--uninstall` to remove it.

Either way, the helper starts on demand over D-Bus. To start the overlay at login: `systemctl --user enable --now omaasus.service`.

`PKGBUILD` and `omaasus.install` build the same package from `omaasus-@VERSION@-source.tar.gz` with `makepkg`. `SHA256SUMS` covers every file.
