# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Classic sorting algorithms in Python. The in-place partition routine is named
# `_arrange_around_pivot` (no word "quicksort" in its body) so a query for
# "quicksort partition step" is a partial-miss: vector may find it via shared
# tokens, lexical struggles on the natural-language phrasing.


def insertion_sort(values):
    """Return a new list sorted ascending using insertion sort. Stable, O(n^2),
    good for small or nearly-sorted inputs.
    """
    result = list(values)
    for i in range(1, len(result)):
        key = result[i]
        j = i - 1
        while j >= 0 and result[j] > key:
            result[j + 1] = result[j]
            j -= 1
        result[j + 1] = key
    return result


def _arrange_around_pivot(values, lo, hi):
    """Lomuto partition: move elements smaller than the pivot to its left and
    return the pivot's final index. Mutates `values` in place.
    """
    pivot = values[hi]
    i = lo - 1
    for j in range(lo, hi):
        if values[j] <= pivot:
            i += 1
            values[i], values[j] = values[j], values[i]
    values[i + 1], values[hi] = values[hi], values[i + 1]
    return i + 1


def quicksort_in_place(values, lo=0, hi=None):
    """Sort `values` ascending in place using recursive quicksort."""
    if hi is None:
        hi = len(values) - 1
    if lo < hi:
        p = _arrange_around_pivot(values, lo, hi)
        quicksort_in_place(values, lo, p - 1)
        quicksort_in_place(values, p + 1, hi)
    return values


def binary_search(sorted_values, target):
    """Return the index of `target` in an ascending list, or -1 when absent."""
    lo, hi = 0, len(sorted_values) - 1
    while lo <= hi:
        mid = (lo + hi) // 2
        if sorted_values[mid] == target:
            return mid
        if sorted_values[mid] < target:
            lo = mid + 1
        else:
            hi = mid - 1
    return -1
