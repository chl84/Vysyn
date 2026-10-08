#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
prefix="${1:-$PWD/.native/install}"
source_dir="$PWD/.native/libheif"
if [[ ! -d "$source_dir" ]]; then
  git clone --depth 1 --branch v1.23.6 https://github.com/strukturag/libheif.git "$source_dir"
fi
[[ "$(git -C "$source_dir" describe --tags --exact-match)" == v1.23.6 ]]
cmake -S "$source_dir" -B .native/heif-build -G Ninja \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$prefix" \
  -DCMAKE_INSTALL_LIBDIR=lib -DBUILD_SHARED_LIBS=ON -DBUILD_TESTING=OFF \
  -DBUILD_DOCUMENTATION=OFF -DWITH_EXAMPLES=OFF -DWITH_GDK_PIXBUF=OFF \
  -DENABLE_PLUGIN_LOADING=OFF -DWITH_LIBDE265=ON -DWITH_DAV1D=ON \
  -DWITH_DAV1D_PLUGIN=OFF -DWITH_X265=OFF -DWITH_AOM_DECODER=OFF \
  -DWITH_AOM_ENCODER=OFF -DWITH_OpenH264_DECODER=OFF -DWITH_LIBSHARPYUV=OFF
cmake --build .native/heif-build --parallel 2
cmake --install .native/heif-build
printf 'Native library installed to %s\n' "$prefix"
