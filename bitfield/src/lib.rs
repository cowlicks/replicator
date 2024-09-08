//! Bitfield datastructure
pub trait Bitfield {
    /// Get the value at `index`
    fn get(&self, index: u64) -> bool;
    /// Set `index` to `value`
    fn set(&mut self, index: u64, value: bool);
    /// Set range from `index` to `index + length` (exclusive) to `value`
    fn set_range(&mut self, index: u64, length: u64, value: bool);
}

pub struct DumbestBitfield {
    data: Vec<bool>,
}

fn u64_to_usize_fixme(x: u64) -> usize {
    usize::try_from(x).expect("TODO")
}

impl DumbestBitfield {
    pub fn new() -> Self {
        Self { data: vec![] }
    }

    fn maybe_extend(&mut self, to_length: u64, value: bool) {
        let to_length = u64_to_usize_fixme(to_length);
        if self.data.len() <= to_length {
            self.data.extend(&vec![value; to_length - len + 1]);
        }
    }
}

impl Bitfield for DumbestBitfield {
    fn get(&self, index: u64) -> bool {
        self.data
            .get(u64_to_usize_fixme(index))
            .cloned()
            .unwrap_or(false)
    }

    fn set(&mut self, index: u64, value: bool) {
        self.maybe_extend(index, false);
        let index = u64_to_usize_fixme(index);
        self.data[index] = value;
    }

    fn set_range(&mut self, index: u64, length: u64, value: bool) {
        self.maybe_extend(index, false);
        let index = u64_to_usize_fixme(index);
        let length = u64_to_usize_fixme(length);
        (&mut self.data[index..(index + length)]).fill(value)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Bitfield, DumbestBitfield};

    #[test]
    fn bf() {
        let mut x = DumbestBitfield::new();
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
