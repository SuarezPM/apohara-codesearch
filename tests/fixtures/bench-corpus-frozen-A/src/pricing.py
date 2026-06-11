# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Pricing and tax math in Python. Unparsed for structural extraction (Python
# has no signature/import rows here) but fully text-searchable. The `class`
# gives Python a symbol chunk for labeling.


def apply_sales_tax(amount_cents, tax_rate_basis_points):
    """Add sales tax to an amount given a rate in basis points (1/100 of a
    percent). 2500 basis points == 25% tax. Rounds half-up to the nearest cent.
    """
    tax = (amount_cents * tax_rate_basis_points + 5000) // 10000
    return amount_cents + tax


def bulk_discount_cents(unit_cents, quantity):
    """Tiered bulk discount: 10% off at 10+ units, 20% off at 100+ units. Returns
    the discounted line total in integer cents.
    """
    gross = unit_cents * quantity
    if quantity >= 100:
        return gross * 80 // 100
    if quantity >= 10:
        return gross * 90 // 100
    return gross


def convert_currency(amount_cents, rate_micros):
    """Convert an amount using an exchange rate expressed in micros (rate * 1e6).
    Truncates toward zero, the conservative direction for a payable.
    """
    return amount_cents * rate_micros // 1_000_000


class PriceBook:
    """An in-memory price list keyed by SKU, with a lookup that falls back to a
    configurable default price when the SKU is unknown.
    """

    def __init__(self, default_cents):
        self.default_cents = default_cents
        self.prices = {}

    def set_price(self, sku, cents):
        self.prices[sku] = cents

    def price_of(self, sku):
        return self.prices.get(sku, self.default_cents)
