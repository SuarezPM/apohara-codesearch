# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Fixture for tree-sitter-ruby extractor tests.
# Mirrors the structure of fixture.{py,go,ts,rs,sh,java,c}: a class, a module,
# a top-level method, with a do_block inside one method (the R-1.3 anti-gotcha:
# the block's lambda/proc body must NOT be emitted as a separate top-level
# method).
#
# Trivial bodies — the test asserts the EXTRACTOR surface (class/module/method
# names + parameters), not Ruby semantic understanding.

require "json"
require_relative "./lib/utils"

class Greeter
  def initialize(name)
    @name = name
  end

  def greet
    [1, 2, 3].each do |n|
      puts n
    end
  end
end

module Util
  def self.add(a, b)
    a + b
  end
end

def top_level_helper
  42
end
