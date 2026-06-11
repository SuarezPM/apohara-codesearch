// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Invoice and money handling. Coherent billing logic with distinguishable
// identifiers so the benchmark can label specific symbols by name.

use std::collections::HashMap;
use std::fmt;

/// A monetary amount in integer minor units (cents) plus an ISO currency code.
/// Kept as integer cents to avoid floating-point rounding drift in totals.
pub struct Money {
    pub cents: i64,
    pub currency: String,
}

impl Money {
    /// Construct a `Money` from a whole-dollar value and a currency code.
    pub fn from_dollars(dollars: i64, currency: &str) -> Money {
        Money {
            cents: dollars * 100,
            currency: currency.to_string(),
        }
    }

    /// Add two amounts of the SAME currency. Panics on a currency mismatch —
    /// cross-currency arithmetic must go through an explicit conversion first.
    pub fn add(&self, other: &Money) -> Money {
        assert_eq!(self.currency, other.currency, "currency mismatch");
        Money {
            cents: self.cents + other.cents,
            currency: self.currency.clone(),
        }
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{:02} {}", self.cents / 100, self.cents % 100, self.currency)
    }
}

/// A single billable line on an invoice: a description, a unit price, and a
/// quantity. The extended amount is `unit_price * quantity`.
pub struct LineItem {
    pub description: String,
    pub unit_cents: i64,
    pub quantity: i64,
}

impl LineItem {
    /// Extended amount for this line: unit price times quantity.
    pub fn extended_cents(&self) -> i64 {
        self.unit_cents * self.quantity
    }
}

/// An invoice: a customer reference plus its line items. The grand total is the
/// sum of every line's extended amount.
pub struct Invoice {
    pub customer: String,
    pub lines: Vec<LineItem>,
}

impl Invoice {
    /// Build an empty invoice for a customer.
    pub fn new(customer: &str) -> Invoice {
        Invoice {
            customer: customer.to_string(),
            lines: Vec::new(),
        }
    }

    /// Append one line item to the invoice.
    pub fn push_line(&mut self, item: LineItem) {
        self.lines.push(item);
    }

    /// Grand total across all line items, in integer cents.
    pub fn grand_total_cents(&self) -> i64 {
        self.lines.iter().map(|l| l.extended_cents()).sum()
    }
}

/// Apply a percentage discount to an amount of cents, rounding half-up to the
/// nearest cent. A 0% discount returns the input unchanged; a 100% discount
/// returns zero.
pub fn apply_percentage_discount(cents: i64, percent: i64) -> i64 {
    let kept = 100 - percent;
    (cents * kept + 50) / 100
}

/// Group line items by their description and sum the quantities, returning a map
/// from description to total quantity. Used to roll up repeated SKUs on a long
/// invoice into one summary line per distinct product.
pub fn rollup_quantities(lines: &[LineItem]) -> HashMap<String, i64> {
    let mut totals: HashMap<String, i64> = HashMap::new();
    for line in lines {
        *totals.entry(line.description.clone()).or_insert(0) += line.quantity;
    }
    totals
}
