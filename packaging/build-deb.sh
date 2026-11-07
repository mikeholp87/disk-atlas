#!/usr/bin/env bash
# Build a binary .deb of Disk Atlas from the current checkout.
# Usage: packaging/build-deb.sh [output-dir]      (default: target/deb)
# Maintainer comes from DEBFULLNAME/DEBEMAIL, else from `git config user.name/email`.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(dirname "$here")
out=${1:-$root/target/deb}

name=${DEBFULLNAME:-$(git -C "$root" config user.name || true)}
email=${DEBEMAIL:-$(git -C "$root" config user.email || true)}
if [[ -z $name || -z $email ]]; then
	echo "error: set DEBFULLNAME and DEBEMAIL (or git config user.name/user.email)" >&2
	exit 1
fi
maintainer="$name <$email>"

version=$(cargo metadata --no-deps --format-version 1 --manifest-path "$root/Cargo.toml" |
	jq -r '.packages[] | select(.name == "diskatlas") | .version')
arch=$(dpkg --print-architecture)

cargo build --release --locked -p diskatlas --manifest-path "$root/Cargo.toml"
bin=$root/target/release/diskatlas

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
stage=$work/pkg
doc=$stage/usr/share/doc/diskatlas

install -D -m 0755 "$bin" "$stage/usr/bin/diskatlas"
strip --strip-unneeded "$stage/usr/bin/diskatlas"
install -D -m 0644 "$here/diskatlas.desktop" "$stage/usr/share/applications/diskatlas.desktop"
install -D -m 0644 "$here/diskatlas.metainfo.xml" "$stage/usr/share/metainfo/diskatlas.metainfo.xml"
install -D -m 0644 "$here/diskatlas.svg" "$stage/usr/share/icons/hicolor/scalable/apps/diskatlas.svg"
install -D -m 0644 "$here/copyright" "$doc/copyright"

# Reproducible when SOURCE_DATE_EPOCH is set.
stamp=$(date -R -d "@${SOURCE_DATE_EPOCH:-$(date +%s)}")
printf '%s\n' \
	"diskatlas ($version) unstable; urgency=medium" \
	"" \
	"  * Development build." \
	"" \
	" -- $maintainer  $stamp" | gzip -9n >"$doc/changelog.gz"
chmod 0644 "$doc/changelog.gz"

# Validate the desktop integration files when the tools are installed.
if command -v desktop-file-validate >/dev/null; then
	desktop-file-validate "$here/diskatlas.desktop"
fi
if command -v appstreamcli >/dev/null; then
	# Advisory: an rDNS id and a homepage need a domain/URL that does not exist yet.
	appstreamcli validate --no-net "$here/diskatlas.metainfo.xml" ||
		echo "warning: AppStream metadata has issues (see above); fix before publishing" >&2
fi

# Library dependencies of the binary, computed by dpkg rather than guessed.
mkdir -p "$work/debian"
printf 'Source: diskatlas\n\nPackage: diskatlas\nArchitecture: any\n' >"$work/debian/control"
shlibs=$(cd "$work" && dpkg-shlibdeps -O "$stage/usr/bin/diskatlas" | sed -n 's/^shlibs:Depends=//p')

installed_kb=$(du -sk "$stage" | cut -f1)
mkdir -p "$stage/DEBIAN"
# wgpu/winit load the graphics and windowing libraries at run time (dlopen), so dpkg cannot see
# them: at least one graphics and one windowing library must be present.
cat >"$stage/DEBIAN/control" <<CONTROL
Package: diskatlas
Version: $version
Section: utils
Priority: optional
Architecture: $arch
Maintainer: $maintainer
Installed-Size: $installed_kb
Depends: $shlibs, libvulkan1 | libegl1, libwayland-client0 | libx11-6, libxkbcommon0
Recommends: mesa-vulkan-drivers, libwayland-egl1, libxkbcommon-x11-0, libxcursor1, libxi6, libx11-xcb1, libxcb1, fonts-noto-core
Description: disk usage analyzer with sunburst and treemap charts
 Disk Atlas scans a folder or volume and shows where the space goes as a
 sunburst chart, a treemap, a folder listing and a list of the largest files.
 It is read-only: it never modifies or deletes files.
CONTROL

mkdir -p "$out"
deb=$out/diskatlas_${version}_${arch}.deb
dpkg-deb --root-owner-group -Zxz --build "$stage" "$deb" >/dev/null
echo "$deb"
