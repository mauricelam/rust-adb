use std::cmp::min;
use std::collections::VecDeque;
use std::ops::{Deref, DerefMut};

/// A block of memory used for I/O.
///
/// This is a wrapper around a `Vec<u8>` that adds a `position` field to allow for sequential
/// reads and writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    data: Vec<u8>,
    position: usize,
}

impl Block {
    /// Creates a new, empty `Block`.
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            position: 0,
        }
    }

    /// Creates a new `Block` with the given capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: Vec::with_capacity(capacity),
            position: 0,
        }
    }

    /// Creates a new `Block` from a slice of bytes.
    pub fn from_slice(slice: &[u8]) -> Self {
        Self {
            data: slice.to_vec(),
            position: 0,
        }
    }

    /// Returns the number of bytes remaining in the block after the current position.
    pub fn remaining(&self) -> usize {
        self.data.len() - self.position
    }

    /// Fills this block from another block.
    ///
    /// The number of bytes copied is the minimum of the remaining space in each block.
    pub fn fill_from(&mut self, from: &mut Block) -> usize {
        let size = min(self.remaining(), from.remaining());
        let from_slice = &from.data[from.position..from.position + size];
        self.data[self.position..self.position + size].copy_from_slice(from_slice);
        self.position += size;
        from.position += size;
        size
    }

    /// Resets the position of the block to the beginning.
    pub fn rewind(&mut self) {
        self.position = 0;
    }

    /// Returns the current position of the block.
    pub fn position(&self) -> usize {
        self.position
    }
}

impl Default for Block {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for Block {
    type Target = Vec<u8>;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl DerefMut for Block {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

/// An ADB message header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Amessage {
    pub command: u32,
    pub arg0: u32,
    pub arg1: u32,
    pub data_length: u32,
    pub data_check: u32,
    pub magic: u32,
}

/// An ADB packet, consisting of a message header and a payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Apacket {
    pub msg: Amessage,
    pub payload: Block,
}

/// A sequence of `Block`s that represents a single buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IoVector {
    chain: VecDeque<Block>,
    chain_length: usize,
    begin_offset: usize,
}

impl IoVector {
    /// Creates a new, empty `IoVector`.
    pub fn new() -> Self {
        Self {
            chain: VecDeque::new(),
            chain_length: 0,
            begin_offset: 0,
        }
    }

    /// Appends a `Block` to the `IoVector`.
    pub fn append(&mut self, mut block: Block) {
        if block.is_empty() {
            return;
        }
        block.rewind();
        self.chain_length += block.len();
        self.chain.push_back(block);
    }

    /// Returns the total size of the `IoVector`.
    pub fn size(&self) -> usize {
        self.chain_length - self.begin_offset
    }

    /// Returns `true` if the `IoVector` is empty.
    pub fn is_empty(&self) -> bool {
        self.size() == 0
    }

    /// Drops `len` bytes from the front of the `IoVector`.
    pub fn drop_front(&mut self, len: usize) {
        if len == 0 {
            return;
        }
        assert!(len <= self.size());
        if len == self.size() {
            self.chain.clear();
            self.chain_length = 0;
            self.begin_offset = 0;
            return;
        }

        let mut dropped = 0;
        while dropped < len {
            let next = {
                let front = self.chain.front().unwrap();
                front.len() - self.begin_offset
            };
            if dropped + next <= len {
                let front = self.chain.pop_front().unwrap();
                self.chain_length -= front.len();
                self.begin_offset = 0;
                dropped += next;
            } else {
                let taken = len - dropped;
                self.begin_offset += taken;
                break;
            }
        }
    }

    /// Takes `len` bytes from the front of the `IoVector` and returns them in a new `IoVector`.
    pub fn take_front(&mut self, len: usize) -> Self {
        if len == 0 {
            return Self::new();
        }
        if len == self.size() {
            return std::mem::replace(self, Self::new());
        }
        assert!(len < self.size());

        let mut res = Self::new();
        let mut len_to_take = len;

        while len_to_take > 0 {
            let (available_in_block, block_len) = {
                let front = self.chain.front().unwrap();
                (front.len() - self.begin_offset, front.len())
            };

            let to_take = std::cmp::min(len_to_take, available_in_block);

            {
                let front = self.chain.front().unwrap();
                let slice_to_take = &front.data[self.begin_offset..self.begin_offset + to_take];
                res.append(Block::from_slice(slice_to_take));
            }

            self.begin_offset += to_take;
            len_to_take -= to_take;

            if self.begin_offset == block_len {
                let popped_block = self.chain.pop_front().unwrap();
                self.chain_length -= popped_block.len();
                self.begin_offset = 0;
            }
        }
        res
    }

    /// Coalesces the `IoVector` into a single `Block`.
    pub fn coalesce(&self) -> Block {
        if self.is_empty() {
            return Block::new();
        }

        let mut result = Block::with_capacity(self.size());
        let (first, second) = self.chain.as_slices();
        if !first.is_empty() {
            result.extend_from_slice(&first[0][self.begin_offset..]);
        }
        for block in &first[1..] {
            result.extend_from_slice(block);
        }
        for block in second {
            result.extend_from_slice(block);
        }
        result
    }
}

impl Default for IoVector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_block(s: &str) -> Block {
        Block::from_slice(s.as_bytes())
    }

    #[test]
    fn empty() {
        let bc = IoVector::new();
        assert_eq!(bc.coalesce().len(), 0);
    }

    #[test]
    fn move_constructor() {
        let x = IoVector::new();
        let xsize = x.coalesce().len();
        let y = x;
        assert_eq!(xsize, y.coalesce().len());
    }

    #[test]
    fn single_block() {
        let block = create_block("x".repeat(100).as_str());
        let mut bc = IoVector::new();
        bc.append(block.clone());
        assert_eq!(100, bc.size());
        let coalesced = bc.coalesce();
        assert_eq!(block, coalesced);
    }

    #[test]
    fn single_block_split() {
        let mut bc = IoVector::new();
        bc.append(create_block("foobar"));
        let foo = bc.take_front(3);
        assert_eq!(3, foo.size());
        assert_eq!(3, bc.size());
        assert_eq!(create_block("foo"), foo.coalesce());
        assert_eq!(create_block("bar"), bc.coalesce());
    }

    #[test]
    fn aligned_split() {
        let mut bc = IoVector::new();
        bc.append(create_block("foo"));
        bc.append(create_block("bar"));
        bc.append(create_block("baz"));
        assert_eq!(9, bc.size());

        let foo = bc.take_front(3);
        assert_eq!(3, foo.size());
        assert_eq!(create_block("foo"), foo.coalesce());

        let bar = bc.take_front(3);
        assert_eq!(3, bar.size());
        assert_eq!(create_block("bar"), bar.coalesce());

        let baz = bc.take_front(3);
        assert_eq!(3, baz.size());
        assert_eq!(create_block("baz"), baz.coalesce());

        assert_eq!(0, bc.size());
    }

    #[test]
    fn misaligned_split() {
        let mut bc = IoVector::new();
        bc.append(create_block("foo"));
        bc.append(create_block("bar"));
        bc.append(create_block("baz"));
        bc.append(create_block("qux"));
        bc.append(create_block("quux"));

        let foob = bc.take_front(4);
        assert_eq!(4, foob.size());
        assert_eq!(create_block("foob"), foob.coalesce());

        let a = bc.take_front(1);
        assert_eq!(1, a.size());
        assert_eq!(create_block("a"), a.coalesce());

        let rba = bc.take_front(3);
        assert_eq!(3, rba.size());
        assert_eq!(create_block("rba"), rba.coalesce());

        let zquxquu = bc.take_front(7);
        assert_eq!(7, zquxquu.size());
        assert_eq!(create_block("zquxquu"), zquxquu.coalesce());

        assert_eq!(1, bc.size());
        assert_eq!(create_block("x"), bc.coalesce());
    }

    #[test]
    fn drop_front() {
        let mut vec = IoVector::new();
        vec.append(create_block("xx"));
        vec.append(create_block(&"y".repeat(1000)));
        assert_eq!(1002, vec.size());

        vec.drop_front(1);
        assert_eq!(1001, vec.size());

        vec.drop_front(1);
        assert_eq!(1000, vec.size());
    }

    #[test]
    fn take_front() {
        let mut vec = IoVector::new();
        assert!(vec.take_front(0).is_empty());

        vec.append(create_block("xx"));
        assert_eq!(2, vec.size());

        assert_eq!(1, vec.take_front(1).size());
        assert_eq!(1, vec.size());

        assert_eq!(1, vec.take_front(1).size());
        assert_eq!(0, vec.size());
    }
}
