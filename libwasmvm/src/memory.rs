use std::mem;
use std::slice;

/// A view into an externally owned byte slice (Go `[]byte`).
/// Use this for the current call only. A view cannot be copied for safety reasons.
/// If you need a copy, use [`ByteSliceView::to_owned`].
///
/// Go's nil value is fully supported, such that we can differentiate between nil and an empty slice.
#[repr(C)]
pub struct ByteSliceView {
    /// True if and only if the byte slice is nil in Go. If this is true, the other fields must be ignored.
    is_nil: bool,
    ptr: *const u8,
    len: usize,
}

impl ByteSliceView {
    /// ByteSliceViews are only constructed in Go. This constructor is a way to mimic the behaviour
    /// when testing FFI calls from Rust. It must not be used in production code.
    #[cfg(test)]
    pub fn new(source: &[u8]) -> Self {
        Self {
            is_nil: false,
            ptr: source.as_ptr(),
            len: source.len(),
        }
    }

    /// ByteSliceViews are only constructed in Go. This constructor is a way to mimic the behaviour
    /// when testing FFI calls from Rust. It must not be used in production code.
    #[cfg(test)]
    pub fn nil() -> Self {
        Self {
            is_nil: true,
            ptr: std::ptr::null::<u8>(),
            len: 0,
        }
    }

    /// Provides a reference to the included data to be parsed or copied elsewhere
    /// This is safe as long as the `ByteSliceView` is constructed correctly.
    pub fn read(&self) -> Option<&[u8]> {
        if self.is_nil {
            None
        } else {
            Some(unsafe { slice::from_raw_parts(self.ptr, self.len) })
        }
    }

    /// Creates an owned copy that can safely be stored and mutated.
    #[allow(dead_code)]
    pub fn to_owned(&self) -> Option<Vec<u8>> {
        self.read().map(|slice| slice.to_owned())
    }
}

/// A view into a `Option<&[u8]>`, created and maintained by Rust.
///
/// This can be copied into a []byte in Go.
#[repr(C)]
pub struct U8SliceView {
    /// True if and only if this is None. If this is true, the other fields must be ignored.
    is_none: bool,
    ptr: *const u8,
    len: usize,
}

impl U8SliceView {
    pub fn new(source: Option<&[u8]>) -> Self {
        match source {
            Some(data) => Self {
                is_none: false,
                ptr: data.as_ptr(),
                len: data.len(),
            },
            None => Self {
                is_none: true,
                ptr: std::ptr::null::<u8>(),
                len: 0,
            },
        }
    }
}

/// An optional Vector type that requires explicit creation and destruction
/// and can be sent via FFI.
/// It can be created from `Option<Vec<u8>>` and be converted into `Option<Vec<u8>>`.
///
/// This type is always created in Rust and always dropped in Rust.
/// If Go code want to create it, it must instruct Rust to do so via the
/// [`new_unmanaged_vector`] FFI export. If Go code wants to consume its data,
/// it must create a copy and instruct Rust to destroy it via the
/// [`destroy_unmanaged_vector`] FFI export.
///
/// An UnmanagedVector is immutable.
///
/// ## Ownership
///
/// Ownership is the right and the obligation to destroy an `UnmanagedVector`
/// exactly once. Both Rust and Go can create an `UnmanagedVector`, which gives
/// then ownership. Sometimes it is necessary to transfer ownership.
///
/// ### Transfer ownership from Rust to Go
///
/// When an `UnmanagedVector` was created in Rust using [`UnmanagedVector::new`], [`UnmanagedVector::default`]
/// or [`new_unmanaged_vector`], it can be passted to Go as a return value (see e.g. [load_wasm][crate::load_wasm]).
/// Rust then has no chance to destroy the vector anymore, so ownership is transferred to Go.
/// In Go, the data has to be copied to a garbage collected `[]byte`. Then the vector must be destroyed
/// using [`destroy_unmanaged_vector`].
///
/// ### Transfer ownership from Go to Rust
///
/// When Rust code calls into Go (using the vtable methods), return data or error messages must be created
/// in Go. This is done by calling [`new_unmanaged_vector`] from Go, which copies data into a newly created
/// `UnmanagedVector`. Since Go created it, it owns it. The ownership is then passed to Rust via the
/// mutable return value pointers. On the Rust side, the vector is destroyed using [`UnmanagedVector::consume`].
///
/// ## Examples
///
/// Transferring ownership from Rust to Go using return values of FFI calls:
///
/// ```
/// # use wasmvm::{cache_t, ByteSliceView, UnmanagedVector};
/// #[no_mangle]
/// pub extern "C" fn save_wasm_to_cache(
///     cache: *mut cache_t,
///     wasm: ByteSliceView,
///     error_msg: Option<&mut UnmanagedVector>,
/// ) -> UnmanagedVector {
///     # let checksum: Vec<u8> = Default::default();
///     // some operation producing a `let checksum: Vec<u8>`
///
///     UnmanagedVector::new(Some(checksum)) // this unmanaged vector is owned by the caller
/// }
/// ```
///
/// Transferring ownership from Go to Rust using return value pointers:
///
/// ```rust
/// # use cosmwasm_vm::{BackendResult, GasInfo};
/// # use wasmvm::{Db, GoResult, U8SliceView, UnmanagedVector};
/// fn db_read(db: &Db, key: &[u8]) -> BackendResult<Option<Vec<u8>>> {
///
///     // Create a None vector in order to reserve memory for the result
///     let mut result = UnmanagedVector::default();
///
///     // …
///     # let mut error_msg = UnmanagedVector::default();
///     # let mut used_gas = 0_u64;
///
///     let go_result: GoResult = (db.vtable.read_db)(
///         db.state,
///         db.gas_meter,
///         &mut used_gas as *mut u64,
///         U8SliceView::new(Some(key)),
///         // Go will create a new UnmanagedVector and override this address
///         &mut result as *mut UnmanagedVector,
///         &mut error_msg as *mut UnmanagedVector,
///     )
///     .into();
///
///     // Some gas processing and error handling
///     # let gas_info = GasInfo::free();
///
///     // We now own the new UnmanagedVector written to the pointer and must destroy it
///     let value = result.consume();
///     (Ok(value), gas_info)
/// }
/// ```
///
///
/// If you want to mutate data, you need to comsume the vector and create a new one:
///
/// ```rust
/// # use wasmvm::{UnmanagedVector};
/// # let input = UnmanagedVector::new(Some(vec![0xAA]));
/// let mut mutable: Vec<u8> = input.consume().unwrap_or_default();
/// assert_eq!(mutable, vec![0xAA]);
///
/// // `input` is now gone and we cam do everything we want to `mutable`,
/// // including operations that reallocate the underylying data.
///
/// mutable.push(0xBB);
/// mutable.push(0xCC);
///
/// assert_eq!(mutable, vec![0xAA, 0xBB, 0xCC]);
///
/// let output = UnmanagedVector::new(Some(mutable));
///
/// // `output` is ready to be passed around
/// ```
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct UnmanagedVector {
    /// True if and only if this is None. If this is true, the other fields must be ignored.
    is_none: bool,
    ptr: *mut u8,
    len: usize,
    cap: usize,
}

impl UnmanagedVector {
    /// Consumes this optional vector for manual management.
    /// This is a zero-copy operation.
    pub fn new(source: Option<Vec<u8>>) -> Self {
        match source {
            Some(data) => {
                let mut data = mem::ManuallyDrop::new(data);
                Self {
                    is_none: false,
                    ptr: data.as_mut_ptr(),
                    len: data.len(),
                    cap: data.capacity(),
                }
            }
            None => Self {
                is_none: true,
                ptr: std::ptr::null_mut::<u8>(),
                len: 0,
                cap: 0,
            },
        }
    }

    pub fn is_none(&self) -> bool {
        self.is_none
    }

    pub fn is_some(&self) -> bool {
        !self.is_none()
    }

    /// Takes this UnmanagedVector and turns it into a regular, managed Rust vector.
    /// Calling this on two copies of UnmanagedVector leads to double free crashes.
    pub fn consume(self) -> Option<Vec<u8>> {
        if self.is_none {
            None
        } else {
            Some(unsafe { Vec::from_raw_parts(self.ptr, self.len, self.cap) })
        }
    }
}

impl Default for UnmanagedVector {
    fn default() -> Self {
        Self {
            is_none: true,
            ptr: std::ptr::null_mut::<u8>(),
            len: 0,
            cap: 0,
        }
    }
}

#[no_mangle]
pub extern "C" fn new_unmanaged_vector(
    nil: bool,
    ptr: *const u8,
    length: usize,
) -> UnmanagedVector {
    if nil {
        UnmanagedVector::new(None)
    } else if length == 0 {
        UnmanagedVector::new(Some(Vec::new()))
    } else {
        let external_memory = unsafe { slice::from_raw_parts(ptr, length) };
        let copy = Vec::from(external_memory);
        UnmanagedVector::new(Some(copy))
    }
}

#[no_mangle]
pub extern "C" fn destroy_unmanaged_vector(v: UnmanagedVector) {
    let _ = v.consume();
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn byte_slice_view_read_works() {
        let data = vec![0xAA, 0xBB, 0xCC];
        let view = ByteSliceView::new(&data);
        assert_eq!(view.read().unwrap(), &[0xAA, 0xBB, 0xCC]);

        let data = vec![];
        let view = ByteSliceView::new(&data);
        assert_eq!(view.read().unwrap(), &[] as &[u8]);

        let view = ByteSliceView::nil();
        assert_eq!(view.read().is_none(), true);
    }

    #[test]
    fn byte_slice_view_to_owned_works() {
        let data = vec![0xAA, 0xBB, 0xCC];
        let view = ByteSliceView::new(&data);
        assert_eq!(view.to_owned().unwrap(), vec![0xAA, 0xBB, 0xCC]);

        let data = vec![];
        let view = ByteSliceView::new(&data);
        assert_eq!(view.to_owned().unwrap(), Vec::<u8>::new());

        let view = ByteSliceView::nil();
        assert_eq!(view.to_owned().is_none(), true);
    }

    #[test]
    fn unmanaged_vector_is_some_works() {
        let x = UnmanagedVector::new(Some(vec![0x11, 0x22]));
        assert_eq!(x.is_some(), true);
        let x = UnmanagedVector::new(Some(vec![]));
        assert_eq!(x.is_some(), true);
        let x = UnmanagedVector::new(None);
        assert_eq!(x.is_some(), false);
    }

    #[test]
    fn unmanaged_vector_is_none_works() {
        let x = UnmanagedVector::new(Some(vec![0x11, 0x22]));
        assert_eq!(x.is_none(), false);
        let x = UnmanagedVector::new(Some(vec![]));
        assert_eq!(x.is_none(), false);
        let x = UnmanagedVector::new(None);
        assert_eq!(x.is_none(), true);
    }

    #[test]
    fn unmanaged_vector_consume_works() {
        let x = UnmanagedVector::new(Some(vec![0x11, 0x22]));
        assert_eq!(x.consume(), Some(vec![0x11u8, 0x22]));
        let x = UnmanagedVector::new(Some(vec![]));
        assert_eq!(x.consume(), Some(Vec::<u8>::new()));
        let x = UnmanagedVector::new(None);
        assert_eq!(x.consume(), None);
    }

    #[test]
    fn unmanaged_vector_defaults_to_none() {
        let x = UnmanagedVector::default();
        assert_eq!(x.consume(), None);
    }
}
