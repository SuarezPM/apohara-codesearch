# A small legacy Ruby module. Ruby is NOT one of the indexer's parsed
# languages, so `detect_language` returns None for `.rb` and this file is
# chunked into fixed-size windows: its hits carry no signature and no
# structural imports/exports (graceful degradation), yet it stays searchable
# through the text (FTS) and vector paths.

require "json"

# Ledger accumulates signed entries and reports a running balance.
class Ledger
  def initialize
    @entries = []
  end

  # Record a labeled amount in the ledger.
  def record(label, amount)
    @entries << { label: label, amount: amount }
    amount
  end

  # Sum every recorded amount into the current balance.
  def balance
    @entries.reduce(0) { |total, entry| total + entry[:amount] }
  end

  # Serialize the ledger to a compact JSON string.
  def to_json_string
    JSON.generate(@entries)
  end
end

# Top-level usage so the file has runnable module-level code too.
ledger = Ledger.new
ledger.record("opening", 100)
ledger.record("withdrawal", -25)
puts "balance=#{ledger.balance}"
puts ledger.to_json_string
