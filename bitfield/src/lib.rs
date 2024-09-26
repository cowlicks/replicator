//! Bitfield datastructure

use std::collections::BTreeSet;
pub trait Bitfield {
    /// Get the value at `index`
    fn get(&self, index: u64) -> bool;
    /// Set `index` to `value`
    fn set(&mut self, index: u64, value: bool);
    /// Set range from `index` to `index + length` (exclusive) to `value`
    fn set_range(&mut self, index: u64, length: u64, value: bool);
}

#[derive(Debug, Clone)]
pub struct DumbBitfield {
    data: BTreeSet<u64>,
}

impl DumbBitfield {
    pub fn new() -> Self {
        Self {
            data: BTreeSet::new(),
        }
    }
}

impl Bitfield for DumbBitfield {
    fn get(&self, index: u64) -> bool {
        self.data.contains(&index)
    }

    fn set(&mut self, index: u64, value: bool) {
        match value {
            true => {
                self.data.insert(index);
            }
            false => {
                self.data.remove(&index);
            }
        };
    }

    fn set_range(&mut self, index: u64, length: u64, value: bool) {
        for i in index..(index + length) {
            self.set(i, value)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Bitfield, DumbBitfield};

    #[test]
    fn bf() {
        let mut x = DumbBitfield::new();
        for i in 0..20 {
            assert!(!x.get(i));
        }
        x.set(10, true);
        assert!(x.get(10));
        x.set_range(4, 3, true);
        for i in 4..(4 + 3) {
            assert!(x.get(i));
        }
        for i in 0..3 {
            assert!(!x.get(i));
        }
        for i in 7..9 {
            assert!(!x.get(i));
        }
        for i in 11..20 {
            assert!(!x.get(i));
        }
    }
}
