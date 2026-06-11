# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Basic descriptive statistics in Python. Distinct numeric vocabulary so a query
# about variance or percentiles resolves here rather than to pricing.py.


def arithmetic_mean(values):
    """Return the arithmetic mean (average) of a non-empty sequence of numbers."""
    if not values:
        raise ValueError("mean of empty sequence")
    return sum(values) / len(values)


def population_variance(values):
    """Return the population variance: the mean of the squared deviations from
    the arithmetic mean. Uses N, not N-1, in the denominator.
    """
    if not values:
        raise ValueError("variance of empty sequence")
    mu = arithmetic_mean(values)
    return sum((v - mu) ** 2 for v in values) / len(values)


def median(values):
    """Return the median value. For an even count, averages the two middle
    elements. Does not mutate the input.
    """
    if not values:
        raise ValueError("median of empty sequence")
    ordered = sorted(values)
    mid = len(ordered) // 2
    if len(ordered) % 2 == 1:
        return ordered[mid]
    return (ordered[mid - 1] + ordered[mid]) / 2


def nearest_rank_percentile(values, percentile):
    """Return the value at the given percentile (0..100) using the nearest-rank
    method. Percentile 50 approximates the median for large samples.
    """
    if not values:
        raise ValueError("percentile of empty sequence")
    ordered = sorted(values)
    rank = max(1, round(percentile / 100 * len(ordered)))
    return ordered[rank - 1]
