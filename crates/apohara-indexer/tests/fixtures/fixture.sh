#!/usr/bin/env bash
# Fixture for tree-sitter-bash extractor tests.
# Mirrors the structure of fixture.py / fixture.go / fixture.ts / fixture.rs:
# three top-level functions, one source-import, one export. The function bodies
# are intentionally trivial — the test asserts the EXTRACTOR surface (names +
# spans + imports + exports), not the parser's deeper comprehension of bash.

source ./lib/helpers.sh

export FOO="hello"
export BAR

# Function 1: no args, single echo.
greet() {
    echo "$FOO, world"
}

# Function 2: positional args, local variable.
add_numbers() {
    local result=$(( $1 + $2 ))
    echo "$result"
}

# Function 3: nested call to function 2, with an if/else.
run_demo() {
    if [ "$1" = "yes" ]; then
        add_numbers 1 2
    else
        echo "skipped"
    fi
}
