#!/usr/bin/env bash
# Build the release-owned FFmpeg tool pair from the admitted source archives.

set -euo pipefail
export LC_ALL=C
export SOURCE_DATE_EPOCH=1781654400
export TZ=UTC
umask 022

readonly MAX_BUILD_JOBS=8

if [[ $# -ne 2 ]]; then
  echo "usage: scripts/release/build-media.sh <source-directory> <output-directory>" >&2
  exit 2
fi

source_directory="$(cd "$1" && pwd)"
readonly source_directory
readonly output_directory="$2"
readonly ffmpeg_archive="$source_directory/ffmpeg-8.1.2.tar.xz"
readonly x264_archive="$source_directory/x264-b35605ace3ddf7c1a5d67a2eb553f034aef41d55.tar.bz2"
readonly zlib_archive="$source_directory/zlib-1.3.1.tar.xz"

for archive in "$ffmpeg_archive" "$x264_archive" "$zlib_archive"; do
  if [[ ! -f "$archive" ]]; then
    echo "missing admitted media source: $archive" >&2
    exit 1
  fi
done
for tool in make pkg-config; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "media build requires $tool" >&2
    exit 1
  fi
done
if [[ "$(uname -m)" == "x86_64" ]] && ! command -v nasm >/dev/null 2>&1; then
  echo "x86_64 media builds require nasm" >&2
  exit 1
fi
if [[ -e "$output_directory" ]]; then
  echo "media build output already exists: $output_directory" >&2
  exit 1
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/onmark-media-build.XXXXXX")"
readonly work
trap 'rm -rf "$work"' EXIT

readonly sources="$work/sources"
readonly install_prefix="/onmark-media"
readonly install_root="$work/install"
readonly prefix="$install_root$install_prefix"
readonly staging="$work/output"
mkdir -p "$sources" "$prefix" "$staging/bin" "$staging/licenses" "$staging/sources"
tar -xf "$ffmpeg_archive" -C "$sources"
tar -xf "$x264_archive" -C "$sources"
tar -xf "$zlib_archive" -C "$sources"

readonly ffmpeg_source="$sources/ffmpeg-8.1.2"
x264_source="$(find "$sources" -mindepth 1 -maxdepth 1 -type d -name 'x264-*' -print -quit)"
readonly x264_source
readonly zlib_source="$sources/zlib-1.3.1"
if [[ ! -d "$ffmpeg_source" || ! -d "$x264_source" || ! -d "$zlib_source" ]]; then
  echo "an admitted media archive has an unexpected root directory" >&2
  exit 1
fi

case "$(uname -s)" in
  Darwin | Linux)
    readonly executable_suffix=""
    readonly ffmpeg_platform_flag=""
    ;;
  MINGW* | MSYS* | CYGWIN*)
    readonly executable_suffix=".exe"
    readonly ffmpeg_platform_flag="--extra-ldflags=-static"
    ;;
  *)
    echo "unsupported media build host: $(uname -s)" >&2
    exit 1
    ;;
esac

if command -v nproc >/dev/null 2>&1; then
  detected_jobs="$(nproc)"
else
  detected_jobs="$(sysctl -n hw.logicalcpu)"
fi
if ((detected_jobs > MAX_BUILD_JOBS)); then
  jobs="$MAX_BUILD_JOBS"
else
  jobs="$detected_jobs"
fi
readonly detected_jobs
readonly jobs

ffmpeg_configure_options=(
  "--prefix=$install_prefix"
  "--pkg-config-flags=--static"
  "--disable-autodetect"
  "--disable-avdevice"
  "--disable-debug"
  "--disable-doc"
  "--disable-ffplay"
  "--disable-network"
  "--disable-nonfree"
  "--disable-shared"
  "--enable-gpl"
  "--enable-libx264"
  "--enable-static"
  "--enable-zlib"
)
if [[ -n "$ffmpeg_platform_flag" ]]; then
  ffmpeg_configure_options+=("$ffmpeg_platform_flag")
fi
readonly -a ffmpeg_configure_options

(
  cd "$zlib_source"
  ./configure --prefix="$install_prefix" --static
  make -j"$jobs"
  make DESTDIR="$install_root" install
)

(
  cd "$x264_source"
  ./configure \
    --prefix="$install_prefix" \
    --bit-depth=8 \
    --disable-opencl \
    --enable-static \
    --enable-pic \
    --disable-cli
  make -j"$jobs"
  make DESTDIR="$install_root" install
)

(
  cd "$ffmpeg_source"
  PKG_CONFIG_PATH="$prefix/lib/pkgconfig" \
    PKG_CONFIG_SYSROOT_DIR="$install_root" \
    ./configure "${ffmpeg_configure_options[@]}"
  make -j"$jobs"
  make DESTDIR="$install_root" install-progs
)

cp "$prefix/bin/ffmpeg$executable_suffix" "$staging/bin/"
cp "$prefix/bin/ffprobe$executable_suffix" "$staging/bin/"
cp "$ffmpeg_source/COPYING.GPLv2" "$staging/licenses/FFmpeg-GPLv2.txt"
cp "$x264_source/COPYING" "$staging/licenses/x264-GPLv2.txt"
cp "$zlib_source/LICENSE" "$staging/licenses/zlib.txt"
cp "$ffmpeg_archive" "$x264_archive" "$zlib_archive" "$staging/sources/"
cp "$(dirname "$0")/build-media.sh" "$staging/sources/build-media.sh"
cp "$(dirname "$0")/media-sources.json" "$staging/media-sources.json"

{
  printf '%s\n' \
    "Onmark release media toolchain" \
    "===============================" \
    "Sources:" \
    "  $(basename "$ffmpeg_archive")" \
    "  $(basename "$x264_archive")" \
    "  $(basename "$zlib_archive")" \
    ""
  "$staging/bin/ffmpeg$executable_suffix" -version
} >"$staging/build.txt"

mkdir -p "$(dirname "$output_directory")"
mv "$staging" "$output_directory"
