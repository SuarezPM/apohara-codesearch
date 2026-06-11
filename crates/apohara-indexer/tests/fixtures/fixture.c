// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Fixture for tree-sitter-c extractor tests.
// Mirrors the structure of fixture.{py,go,ts,rs,sh,java}: top-level functions
// with imports. Trivial bodies — the test asserts the EXTRACTOR surface
// (function names + parameters + return types + imports), not C semantic
// understanding.

#ifndef FIXTURE_C
#define FIXTURE_C

#include <stdio.h>
#include <stdlib.h>
#include "myheader.h"

int add(int a, int b) {
    return a + b;
}

int main(int argc, char **argv) {
    return add(1, 2);
}

int unused_helper(void) {
    return 0;
}

#endif
