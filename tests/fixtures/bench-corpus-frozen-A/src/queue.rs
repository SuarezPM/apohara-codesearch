// SPDX-License-Identifier: MIT OR Apache-2.0
//
// A bounded ring buffer (circular FIFO queue). Distinct "ring"/"wrap"/"head"/
// "tail" vocabulary so a circular-buffer query lands here.

/// A fixed-capacity FIFO queue backed by a ring buffer. Pushes past capacity are
/// rejected rather than overwriting; pops advance the head with wraparound.
pub struct RingBuffer<T: Clone + Default> {
    slots: Vec<T>,
    head: usize,
    len: usize,
}

impl<T: Clone + Default> RingBuffer<T> {
    /// Allocate a ring buffer that holds up to `capacity` elements.
    pub fn with_capacity(capacity: usize) -> RingBuffer<T> {
        RingBuffer {
            slots: vec![T::default(); capacity.max(1)],
            head: 0,
            len: 0,
        }
    }

    /// Enqueue a value at the tail. Returns false when the buffer is full.
    pub fn push(&mut self, value: T) -> bool {
        if self.len == self.slots.len() {
            return false;
        }
        let tail = (self.head + self.len) % self.slots.len();
        self.slots[tail] = value;
        self.len += 1;
        true
    }

    /// Dequeue the value at the head, advancing the head with wraparound.
    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let value = self.slots[self.head].clone();
        self.head = (self.head + 1) % self.slots.len();
        self.len -= 1;
        Some(value)
    }

    /// Current number of buffered elements.
    pub fn len(&self) -> usize {
        self.len
    }

    /// True when no elements are buffered.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}
