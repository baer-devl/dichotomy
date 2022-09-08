use core::ops::Deref;
use core::ptr::NonNull;
use core::sync::atomic::AtomicUsize;

/// Actual data and a ref-counter to keep track of shared instances
///
/// This struct will be put on the heap and live as long as there are references left to it.
struct Inner<T: ?Sized> {
    /// Reference counter to keep track of clones from [Arc]
    ref_count: AtomicUsize,
    /// Pointer to data
    data: T,
}

/// Helper to share the internal buffer
///
/// Basically a lower spec [Arc](std::sync::Arc) implementation which does implement the bare
/// minimum to make it work. If all references to the underlying data got dropped, we drop the
/// internal buffer.
pub(crate) struct Arc<T> {
    /// Pointer to `Inner` holding the data until no references are available
    inner: NonNull<Inner<T>>,
}

impl<T> Arc<T> {
    /// Create a new atomic reference counter to keep track of references
    ///
    /// We need to have atomic references in order to keep track when both entities are dropped, we
    /// will drop the internal data if no references are left. This must be done manually, as we
    /// leaked the data onto the heap.
    pub(crate) fn new(data: T) -> Self {
        let inner = Inner {
            ref_count: AtomicUsize::new(1),
            data,
        };

        // leak inner so we can create multiple references to it
        let inner = unsafe { NonNull::new_unchecked(Box::leak(Box::new(inner))) };

        Self { inner }
    }

    /// Return if the caller is the last instance holding the reference of the internal buffer
    pub(crate) fn is_last(&self) -> bool {
        unsafe {
            self.inner
                .as_ref()
                .ref_count
                .load(core::sync::atomic::Ordering::Acquire)
                == 1
        }
    }
}

impl<T> Clone for Arc<T> {
    fn clone(&self) -> Self {
        // increment ref-count
        unsafe {
            self.inner
                .as_ref()
                .ref_count
                .fetch_add(1, core::sync::atomic::Ordering::Release)
        };

        Self { inner: self.inner }
    }
}

impl<T> Drop for Arc<T> {
    fn drop(&mut self) {
        let old_count = unsafe {
            self.inner
                .as_ref()
                .ref_count
                .fetch_sub(1, core::sync::atomic::Ordering::Release)
        };

        // check if this is the last reference and drop the buffer if so
        if old_count == 1 {
            let b = unsafe { Box::from_raw(self.inner.as_ptr()) };
            drop(b);
        }
    }
}

impl<T> Deref for Arc<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &self.inner.as_ref().data }
    }
}
