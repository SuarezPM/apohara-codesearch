#!/usr/bin/env bash
# Fixture for tree-sitter-bash imports/exports tests.
# Mirrors imports.{py,go,ts,rs}: one source-import, one dot-import, and
# multiple export forms (export FOO, export FOO=bar, declare -x FOO).

# `source` and `.` are both captured as Require-kind imports.
source ./common.sh
. ./env.sh

# All three export forms should appear in the exports vec.
export PATH_VAR
export CONFIG_PATH="/etc/app/config"
declare -x SECRET_TOKEN="xyz"
