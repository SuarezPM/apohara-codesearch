# ClusterFuzzLite build container for the cargo-fuzz targets in fuzz/.
# Based on the OSS-Fuzz Rust base image (provides a nightly toolchain,
# cargo-fuzz, and the libFuzzer/sanitizer runtime).
FROM gcr.io/oss-fuzz-base/base-builder-rust

COPY . $SRC/apohara-codesearch
WORKDIR $SRC/apohara-codesearch
COPY .clusterfuzzlite/build.sh $SRC/
