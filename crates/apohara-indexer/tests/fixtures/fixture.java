// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Fixture for tree-sitter-java extractor tests.
// Mirrors the structure of fixture.py / fixture.go / fixture.ts / fixture.rs /
// fixture.sh: top-level class, one interface, one enum, one record, with
// import + method bodies. Trivial bodies — the test asserts the EXTRACTOR
// surface (class/interface/enum/record names + method names + spans +
// imports), not Java semantic understanding.

import java.util.List;
import com.example.Foo;

public interface Greeter {
    void greet(String name);
}

public class Hello {
    public static void main(String[] args) {
        System.out.println("hi");
    }

    private int add(int a, int b) {
        return a + b;
    }
}

public enum Color {
    RED, GREEN, BLUE;
}

public record Point(int x, int y) {}
