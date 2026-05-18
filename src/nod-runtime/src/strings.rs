//! `<byte-string>` — UTF-8-encoded heap-allocated string.
//!
//! Layout:
//!
//! ```text
//!   [Wrapper 8B] [len: u32] [_pad: u32] [bytes ...] [pad to 8B align]
//! ```
//!
//! The `len` is the byte length, NOT a codepoint count. Bytes are
//! stored inline starting at offset 16. Sprint 10 only writes UTF-8;
//! `<unicode-string>` (UTF-16) lands in Sprint 27.

use crate::classes::{ClassId, ClassTable};
use crate::heap::Heap;
use crate::word::Word;
use crate::wrapper::Wrapper;

/// In-memory layout of a `<byte-string>`. The `bytes` field is the
/// header for the inline byte payload; the actual byte run follows
/// the struct in memory. Always access via the accessor methods.
#[repr(C)]
pub struct ByteString {
    pub wrapper: Wrapper,
    pub len: u32,
    pub _pad: u32,
    // bytes follow inline; size = len, padded to 8-byte alignment.
}

impl ByteString {
    /// Read the inline byte payload.
    ///
    /// # Safety
    ///
    /// `self` must point at a real `<byte-string>` allocation produced
    /// by `Heap::alloc_byte_string` (or a structurally identical pool).
    /// The returned slice borrows from `self`; the caller must not
    /// mutate the underlying memory through any other reference for
    /// the lifetime of the borrow.
    pub unsafe fn bytes(&self) -> &[u8] {
        let base = (self as *const ByteString as *const u8).wrapping_add(size_of::<ByteString>());
        // SAFETY: documented above — caller asserts the layout invariant.
        unsafe { std::slice::from_raw_parts(base, self.len as usize) }
    }

    /// If the inline bytes are valid UTF-8, return them as `&str`.
    ///
    /// # Safety
    ///
    /// Same as `bytes`.
    pub unsafe fn as_str(&self) -> Option<&str> {
        // SAFETY: forwarded to `bytes`.
        let b = unsafe { self.bytes() };
        std::str::from_utf8(b).ok()
    }
}

impl Heap {
    /// Allocate a `<byte-string>` and copy `s.as_bytes()` into the
    /// inline payload. Returns a pointer-tagged `Word`. The class on
    /// the wrapper is `classes.byte_string()`.
    pub fn alloc_byte_string(&self, s: &str, classes: &ClassTable) -> Word {
        let bytes = s.as_bytes();
        // Payload = 4 (len) + 4 (pad) + bytes.len; the heap rounds up
        // to 8-byte alignment.
        let payload_bytes = 8 + bytes.len();
        let w = self.alloc_object(classes.byte_string(), payload_bytes);
        // SAFETY: `alloc_object` returned a freshly initialised wrapper
        // plus a zeroed payload. We now overwrite the first 8 bytes of
        // payload (len, pad) and copy `bytes` into the inline body.
        unsafe {
            let p = w.as_mut_ptr::<u8>().expect("alloc_byte_string returned pointer-tagged Word");
            let bs = p as *mut ByteString;
            (*bs).len = bytes.len() as u32;
            (*bs)._pad = 0;
            if !bytes.is_empty() {
                let dst = p.add(size_of::<ByteString>());
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
            }
        }
        w
    }
}

/// Decode `w` to a `&ByteString` if its wrapper class matches
/// `<byte-string>`. Returns `None` for any other shape.
///
/// # Safety
///
/// `w`, if pointer-tagged, must point at a valid 8-byte-aligned heap
/// object whose first cell is a `Wrapper`. The wrapper class is the
/// gate: only `<byte-string>`-classed objects are dereferenced as
/// `ByteString`. Sprint 11 will add a heap-membership check; today
/// the caller (the tracer, the format-out shim) is responsible.
pub unsafe fn try_byte_string(w: Word, byte_string: ClassId) -> Option<&'static ByteString> {
    let p = w.as_ptr::<u8>()?;
    // SAFETY: wrapper-first invariant — every heap object's first 8
    // bytes are a Wrapper.
    let wrapper: Wrapper = unsafe { *(p as *const Wrapper) };
    if wrapper.class() == byte_string {
        // SAFETY: class match implies the object's layout is ByteString.
        // The `'static` lifetime here is a lie shrunk by callers — the
        // pointer is only valid for the heap's lifetime, but the heap
        // is process-lived in Sprint 10.
        Some(unsafe { &*(p as *const ByteString) })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_round_trip_ascii() {
        let heap = Heap::new();
        let ct = ClassTable::new();
        let w = heap.alloc_byte_string("hello", &ct);
        assert!(w.is_pointer());
        let wrap = heap.wrapper_of(w).expect("inside heap");
        assert_eq!(wrap.class(), ct.byte_string());
        // SAFETY: `w` came straight back from `alloc_byte_string`.
        let bs = unsafe { try_byte_string(w, ct.byte_string()) }.expect("class matches");
        assert_eq!(bs.len, 5);
        // SAFETY: `bs` points at the live allocation.
        let bytes = unsafe { bs.bytes() };
        assert_eq!(bytes, b"hello");
        // SAFETY: forwarded.
        assert_eq!(unsafe { bs.as_str() }, Some("hello"));
    }

    #[test]
    fn alloc_empty_string() {
        let heap = Heap::new();
        let ct = ClassTable::new();
        let w = heap.alloc_byte_string("", &ct);
        // SAFETY: same as above.
        let bs = unsafe { try_byte_string(w, ct.byte_string()) }.expect("class matches");
        assert_eq!(bs.len, 0);
        // SAFETY: same as above.
        assert_eq!(unsafe { bs.bytes() }, b"");
    }

    #[test]
    fn alloc_utf8_string() {
        let heap = Heap::new();
        let ct = ClassTable::new();
        let s = "héllo, 世界";
        let w = heap.alloc_byte_string(s, &ct);
        // SAFETY: same as above.
        let bs = unsafe { try_byte_string(w, ct.byte_string()) }.expect("class matches");
        assert_eq!(bs.len as usize, s.len());
        // SAFETY: same as above.
        assert_eq!(unsafe { bs.as_str() }, Some(s));
    }
}
