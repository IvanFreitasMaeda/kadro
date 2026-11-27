#!/usr/bin/env bash
# Build dependencies for the Linux packages, on Ubuntu 22.04 (glibc 2.35, so the
# AppImage/.deb also run on Debian 12, Mint 21 and older Fedora).
#
# The Rust PipeWire bindings need headers from PipeWire 1.0, newer than 22.04's
# 0.3.48. So we take the headers from Ubuntu 24.04 but link against 22.04's
# library: the link fails if Kadro used a function 0.3.48 doesn't have, and the
# binary runs on any PipeWire from 0.3.48 on.
set -euo pipefail

sudo apt-get update
sudo apt-get install -y libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev \
  libxdo-dev libssl-dev libpipewire-0.3-0 libclang-dev libgbm-dev libegl-dev libwayland-dev \
  libxcb1-dev libxcb-randr0-dev libxcb-shm0-dev libxcb-xfixes0-dev libdbus-1-dev patchelf file

arch=$(dpkg --print-architecture)
case "$arch" in
  amd64)
    mirror=http://archive.ubuntu.com/ubuntu
    sums="c0b34ed6a34391a94c0711966dbea9b6b5a98758d88813824c05d6662e0b60c9  libpipewire-0.3-dev_1.0.5-1_amd64.deb
6c99d162e23e13cacf8bc3edc312c60545281078398e52e34dc5ebb4209649dc  libspa-0.2-dev_1.0.5-1_amd64.deb" ;;
  arm64)
    mirror=http://ports.ubuntu.com/ubuntu-ports
    sums="98d109f570847e78247b3b90b778c689fb821af5a3962b3f99db332dd05170e8  libpipewire-0.3-dev_1.0.5-1_arm64.deb
e7b5391f786a4fe793f242640dddc8aa20d3e3e25c48aad2c614d5053bec99d1  libspa-0.2-dev_1.0.5-1_arm64.deb" ;;
  *) echo "arquitetura não suportada: $arch" >&2; exit 1 ;;
esac

dir=$(mktemp -d)
cd "$dir"
for pkg in libpipewire-0.3-dev libspa-0.2-dev; do
  curl -fsSL --retry 3 -o "${pkg}_1.0.5-1_${arch}.deb" "$mirror/pool/main/p/pipewire/${pkg}_1.0.5-1_${arch}.deb"
done
echo "$sums" | sha256sum -c -
for deb in *.deb; do
  sudo dpkg-deb -x "$deb" /
done
pkg-config --modversion libpipewire-0.3
