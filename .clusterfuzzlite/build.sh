#!/bin/bash -eu
# ClusterFuzzLite / OSS-Fuzz build script: build the cargo-fuzz targets and copy
# each resulting binary into $OUT, where the fuzzing runner expects them.
cd "$SRC/apohara-codesearch"

cargo fuzz build -O --debug-assertions

out_dir="fuzz/target/x86_64-unknown-linux-gnu/release"
for target in $(cargo fuzz list); do
  cp "$out_dir/$target" "$OUT/"
done
