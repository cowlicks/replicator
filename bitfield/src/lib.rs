//! Bitfield datastructure
use range_set_blaze::prelude::RangeSetBlaze;

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
    data: RangeSetBlaze<u64>,
}

impl DumbBitfield {
    pub fn new() -> Self {
        Self {
            data: RangeSetBlaze::new(),
        }
    }
}

impl Bitfield for DumbBitfield {
    fn get(&self, index: u64) -> bool {
        self.data.contains(index)
    }

    fn set(&mut self, index: u64, value: bool) {
        match value {
            true => {
                self.data.insert(index);
            }
            false => {
                self.data.remove(index);
            }
        };
    }

    fn set_range(&mut self, index: u64, length: u64, value: bool) {
        if value == true {
            self.data.ranges_insert(index..(index + length));
        } else {
            for i in index..(index + length) {
                self.data.remove(i);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Bitfield, DumbBitfield};

    #[test]
    fn test_bitfield() {
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

    #[test]
    fn test_bitfield_set_range() {
        let mut y = DumbBitfield::new();
        y.set_range(2, 3, true);
        for i in [2, 3, 4] {
            assert!(y.get(i));
        }
        assert!(!y.get(5));
        y.set_range(3, 2, false);
        for i in [3, 4] {
            assert!(!y.get(i));
        }
    }
}
