#!/bin/sh
# Installs the latest Crowmation release.
#
#   curl -fsSL https://crowmation.com/install.sh | sh
#
# Puts `crowmation` in ~/.local/bin (or $CROWMATION_BIN). No root needed.
# Prebuilt binaries: Linux x86_64. Elsewhere, build from source:
#   cargo install --git https://github.com/InfinitiWarrior/Crowmation crowmation
set -eu

repo="InfinitiWarrior/Crowmation"
bindir="${CROWMATION_BIN:-$HOME/.local/bin}"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target="x86_64-linux" ;;
  *)
    echo "No prebuilt Crowmation for $(uname -s) $(uname -m) yet. Build it from source:" >&2
    echo "  cargo install --git https://github.com/$repo crowmation" >&2
    exit 1
    ;;
esac

for tool in curl tar xz sha256sum; do
  command -v "$tool" >/dev/null 2>&1 || { echo "install.sh needs \`$tool\`" >&2; exit 1; }
done

url="https://github.com/$repo/releases/latest/download/crowmation-$target.tar.xz"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading $url"
curl -fsSL "$url" -o "$tmp/crowmation.tar.xz"
curl -fsSL "$url.sha256" -o "$tmp/crowmation.sha256"
expected="$(cut -d ' ' -f 1 "$tmp/crowmation.sha256")"
actual="$(sha256sum "$tmp/crowmation.tar.xz" | cut -d ' ' -f 1)"
if [ "$expected" != "$actual" ]; then
  echo "Checksum mismatch, not installing (expected $expected, got $actual)" >&2
  exit 1
fi

tar -xJf "$tmp/crowmation.tar.xz" -C "$tmp"
mkdir -p "$bindir"
install -m 755 "$tmp/crowmation/crowmation" "$bindir/crowmation"
echo "Installed $bindir/crowmation"

case ":$PATH:" in
  *":$bindir:"*) ;;
  *) echo "Add it to your PATH: export PATH=\"$bindir:\$PATH\"" ;;
esac

# Rendering needs a Vulkan driver (Mesa or NVIDIA); NVENC is used when present.
if ! ldconfig -p 2>/dev/null | grep -q 'libvulkan\.so\.1'; then
  echo "Note: no Vulkan loader found. Install your distro's vulkan-icd-loader / libvulkan1 package."
fi

echo "Try it: crowmation render scene.cwm out.mp4   (guide: https://github.com/$repo/blob/main/docs/guide.md)"
