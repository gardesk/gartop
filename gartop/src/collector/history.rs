//! Generic ring buffer for time-series history

use std::collections::VecDeque;

/// Ring buffer for maintaining a fixed-size history of samples.
#[derive(Debug, Clone)]
pub struct History<T> {
    buffer: VecDeque<T>,
    capacity: usize,
}

impl<T: Clone> History<T> {
    /// Create a new history buffer with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Push a new sample, evicting the oldest if at capacity.
    pub fn push(&mut self, value: T) {
        if self.buffer.len() >= self.capacity {
            self.buffer.pop_front();
        }
        self.buffer.push_back(value);
    }

    /// Get the most recent sample.
    pub fn latest(&self) -> Option<&T> {
        self.buffer.back()
    }

    /// Get all samples as owned values (oldest to newest).
    pub fn to_vec(&self) -> Vec<T> {
        self.buffer.iter().cloned().collect()
    }

    /// Get the last N samples as owned values (oldest to newest).
    pub fn last_n(&self, n: usize) -> Vec<T> {
        let skip = self.buffer.len().saturating_sub(n);
        self.buffer.iter().skip(skip).cloned().collect()
    }

    /// Current number of samples.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Clear all samples.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }
}

impl<T: Clone> Default for History<T> {
    fn default() -> Self {
        Self::new(300) // Default: 5 minutes at 1 sample/second
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_and_latest() {
        let mut h: History<i32> = History::new(3);
        assert!(h.latest().is_none());

        h.push(1);
        assert_eq!(h.latest(), Some(&1));

        h.push(2);
        assert_eq!(h.latest(), Some(&2));
    }

    #[test]
    fn test_capacity() {
        let mut h: History<i32> = History::new(3);
        h.push(1);
        h.push(2);
        h.push(3);
        assert_eq!(h.len(), 3);

        h.push(4);
        assert_eq!(h.len(), 3);
        assert_eq!(h.to_vec(), vec![2, 3, 4]);
    }

    #[test]
    fn test_last_n() {
        let mut h: History<i32> = History::new(5);
        for i in 1..=5 {
            h.push(i);
        }
        assert_eq!(h.last_n(3), vec![3, 4, 5]);
        assert_eq!(h.last_n(10), vec![1, 2, 3, 4, 5]);
    }
}
