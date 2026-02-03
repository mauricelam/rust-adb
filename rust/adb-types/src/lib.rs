use std::collections::VecDeque;
use std::io::Cursor;
use bytes::{Bytes, Buf};

/// A message header in the ADB protocol.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Amessage {
    pub command: u32,
    pub arg0: u32,
    pub arg1: u32,
    pub data_length: u32,
    pub data_check: u32,
    pub magic: u32,
}

/// A block of memory used for I/O, with an associated seek position.
///
/// In the original C++ implementation, this was a custom `Block` class.
/// In Rust, we use `std::io::Cursor<Vec<u8>>` to provide equivalent
/// functionality as requested.
///
/// Translation guide from C++ Block:
/// - `size()` -> `block.get_ref().len()`
/// - `data()` -> `block.get_ref().as_ptr()`
/// - `position()` -> `block.position()`
/// - `remaining()` -> `block.get_ref().len() - block.position() as usize`
/// - `rewind()` -> `block.set_position(0)`
pub type Block = Cursor<Vec<u8>>;

/// An ADB packet, consisting of a message header and a payload.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Apacket {
    pub msg: Amessage,
    pub payload: Block,
}

/// A sequence of buffers that represents a single contiguous stream of data.
///
/// This is implemented using a chain of `bytes::Bytes` objects for efficient
/// slicing and sharing, as requested.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IoVector {
    chain: VecDeque<Bytes>,
}

impl IoVector {
    /// Creates a new, empty `IoVector`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends data to the end of the `IoVector`.
    pub fn append(&mut self, data: Bytes) {
        if !data.is_empty() {
            self.chain.push_back(data);
        }
    }

    /// Returns the total number of bytes in the `IoVector`.
    pub fn size(&self) -> usize {
        self.chain.iter().map(|b| b.len()).sum()
    }

    /// Returns true if the `IoVector` contains no bytes.
    pub fn is_empty(&self) -> bool {
        self.chain.is_empty()
    }

    /// Drops the first `len` bytes from the `IoVector`.
    pub fn drop_front(&mut self, mut len: usize) {
        assert!(len <= self.size());
        while len > 0 {
            let front = self.chain.front_mut().expect("size was checked");
            if len >= front.len() {
                len -= front.len();
                self.chain.pop_front();
            } else {
                front.advance(len);
                len = 0;
            }
        }
    }

    /// Takes the first `len` bytes from the `IoVector` and returns them as a new `IoVector`.
    pub fn take_front(&mut self, mut len: usize) -> Self {
        assert!(len <= self.size());
        let mut res = Self::new();
        while len > 0 {
            let front = self.chain.front_mut().expect("size was checked");
            if len >= front.len() {
                len -= front.len();
                res.append(self.chain.pop_front().unwrap());
            } else {
                let taken = front.split_to(len);
                res.append(taken);
                len = 0;
            }
        }
        res
    }

    /// Logically trims the front of the `IoVector`.
    ///
    /// In the C++ implementation, this physically moved data to compact the first block.
    /// In this Rust implementation using `bytes::Bytes`, logical trimming is already
    /// handled by `drop_front` and `take_front`.
    pub fn trim_front(&mut self) {
        // No-op as Bytes are already logically trimmed.
    }

    /// Coalesces all buffers in the `IoVector` into a single `Vec<u8>`.
    pub fn coalesce(&self) -> Vec<u8> {
        let mut res = Vec::with_capacity(self.size());
        for b in &self.chain {
            res.extend_from_slice(b);
        }
        res
    }
}

/// A weak pointer type.
///
/// The C++ implementation used a custom `enable_weak_from_this` and `weak_ptr`
/// tied to the `fdevent` looper. In Rust, `std::rc::Weak<T>` or `std::sync::Weak<T>`
/// are the standard replacements.
///
/// For ADB's single-threaded event loop logic, `std::rc::Weak<T>` is typically used.
pub type WeakPtr<T> = std::rc::Weak<T>;

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn create_bytes(s: &str) -> Bytes {
        Bytes::copy_from_slice(s.as_bytes())
    }

    #[test]
    fn empty() {
        let bc = IoVector::new();
        assert_eq!(bc.size(), 0);
        assert_eq!(bc.coalesce().len(), 0);
    }

    #[test]
    fn single_block() {
        let data = create_bytes(&"x".repeat(100));
        let mut bc = IoVector::new();
        bc.append(data.clone());
        assert_eq!(100, bc.size());
        let coalesced = bc.coalesce();
        assert_eq!(data.as_ref(), coalesced.as_slice());
    }

    #[test]
    fn single_block_split() {
        let mut bc = IoVector::new();
        bc.append(create_bytes("foobar"));
        let foo = bc.take_front(3);
        assert_eq!(3, foo.size());
        assert_eq!(3, bc.size());
        assert_eq!(b"foo", foo.coalesce().as_slice());
        assert_eq!(b"bar", bc.coalesce().as_slice());
    }

    #[test]
    fn aligned_split() {
        let mut bc = IoVector::new();
        bc.append(create_bytes("foo"));
        bc.append(create_bytes("bar"));
        bc.append(create_bytes("baz"));
        assert_eq!(9, bc.size());

        let foo = bc.take_front(3);
        assert_eq!(3, foo.size());
        assert_eq!(b"foo", foo.coalesce().as_slice());

        let bar = bc.take_front(3);
        assert_eq!(3, bar.size());
        assert_eq!(b"bar", bar.coalesce().as_slice());

        let baz = bc.take_front(3);
        assert_eq!(3, baz.size());
        assert_eq!(b"baz", baz.coalesce().as_slice());

        assert_eq!(0, bc.size());
    }

    #[test]
    fn misaligned_split() {
        let mut bc = IoVector::new();
        bc.append(create_bytes("foo"));
        bc.append(create_bytes("bar"));
        bc.append(create_bytes("baz"));
        bc.append(create_bytes("qux"));
        bc.append(create_bytes("quux"));

        let foob = bc.take_front(4);
        assert_eq!(4, foob.size());
        assert_eq!(b"foob", foob.coalesce().as_slice());

        let a = bc.take_front(1);
        assert_eq!(1, a.size());
        assert_eq!(b"a", a.coalesce().as_slice());

        let rba = bc.take_front(3);
        assert_eq!(3, rba.size());
        assert_eq!(b"rba", rba.coalesce().as_slice());

        let zquxquu = bc.take_front(7);
        assert_eq!(7, zquxquu.size());
        assert_eq!(b"zquxquu", zquxquu.coalesce().as_slice());

        assert_eq!(1, bc.size());
        assert_eq!(b"x", bc.coalesce().as_slice());
    }

    #[test]
    fn drop_front() {
        let mut vec = IoVector::new();
        vec.append(create_bytes("xx"));
        vec.append(create_bytes(&"y".repeat(1000)));
        assert_eq!(1002, vec.size());

        vec.drop_front(1);
        assert_eq!(1001, vec.size());

        vec.drop_front(1);
        assert_eq!(1000, vec.size());
    }

    #[test]
    fn take_front_test() {
        let mut vec = IoVector::new();
        assert!(vec.take_front(0).is_empty());

        vec.append(create_bytes("xx"));
        assert_eq!(2, vec.size());

        assert_eq!(1, vec.take_front(1).size());
        assert_eq!(1, vec.size());

        assert_eq!(1, vec.take_front(1).size());
        assert_eq!(0, vec.size());
    }

    #[test]
    fn trim_front() {
        let mut vec = IoVector::new();
        vec.append(create_bytes("foobar"));
        vec.drop_front(3);
        assert_eq!(3, vec.size());
        vec.trim_front();
        assert_eq!(3, vec.size());
        assert_eq!(b"bar", vec.coalesce().as_slice());
    }
}
