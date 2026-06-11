// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Fixture for tree-sitter-cpp extractor tests.
// Mirrors fixture.{c,java,rb,sh,etc}: a class with methods, a free function,
// and a struct. Trivial bodies — the test asserts the EXTRACTOR surface,
// not C++ semantic understanding.

#include <iostream>
#include <vector>
#include "myheader.h"

class Greeter {
public:
    void greet(const std::string& name) {
        std::cout << "hi " << name << std::endl;
    }
};

struct Point {
    int x;
    int y;
};

int add(int a, int b) {
    return a + b;
}

int main(int argc, char **argv) {
    return add(1, 2);
}
