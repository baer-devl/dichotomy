//! Dichotomy is a lock-free byte ring buffer
//!
//! The buffer itself does not use any locks like [Mutex](std::sync::Mutex) or
//! [RwLock](std::sync::RwLock), nor make use of atomics to synchronize the internal buffer access.
//! The two parts, [Producer] and [Consumer] are guranteed to have mutual exclusive access to the
//! buffer area they need access to. Thus makes the buffer very performant as it does not need to
//! handle locks.
//!
//! There are two tags used to handle the internal access to the buffer called `head_tag` and
//! `tail_tag`. Both will hold an index and a state. The index will give the position of the start
//! and the end of written data. The state ensures that the [Consumer] does have the information if
//! there is more to read from.
//! The `head_tag` can only be written by the [Consumer] and the `tail_tag` can only be written by
//! the [Producer]. This and the fact that the processor _should_ be able to read an `usize` in a
//! single instruction, makes it safe to access those values without locks.
//!
//! # How tags are used
//! Lets first focus on the index part of the tags.
//! The [Producer] holds its index in the `tail_tag` which marks the beginning of the writable
//! buffer. If there is space left in the buffer, it will write into it and update the index.
//! The very same is true for the [Consumer], who holds its index in the `head_tag`, indicating the
//! beginning of the readable data. If the data gets consumed, it will update the index and free
//! space on the buffer.
//! If both indicies are the same, this could either mean that the buffer is empty of full. To make
//! a distinction of those two possibilities, we are using the state part of the tags which is an
//! incrementing number changed by each write to the buffer.
//! If the [Consumer] checks if the buffer has data to read from, it will check if the tags will
//! differ. If that is the case, the index of both values will be used to calculate the readable
//! size of the data. Each time the [Consumer] reads data from the buffer, it will update its state
//! to the last known state of the [Producer] tag. This ensures that there will never be a false
//! empty buffer because the state will always be different after the [Producer] wrote new data.
//! Here are some examples of possible states of the two tags:
//!
//! ## Empty buffer
//! ```
//! |--------X------------|
//! ```
//! 'X' representing both indicies of the tags which pointing to the same position. In this special
//! case, the state of both tags have to be the same. Thus makes it easy to check, if both tags are
//! the same the buffer is empty.
//!
//! ## Filled buffer
//! ```
//! |----H**********T-----|
//! |**T---------------H**|
//! ```
//! 'H' representing the index of `head_tag` and 'T' representing the index of `tail_tag`. As both
//! indicies are different, it is safe to assume that there is data to read from and free space to
//! write to. Both, the writeable and the readable space can easily be calculated.
//!
//! ## Full buffer
//! ```
//! |*****************X***|
//! ```
//! 'X' representing both indicies of the tags pointing to the same position. However, this time
//! the state of the tags differ which indicates that the buffer must be full as the [Producer] had
//! written data onto the buffer and the [Consumer] does not know about it yet.
//!
//! # Safety
//! The whole safety of this implementation relies on the fact that the processor can read/write an
//! `usize` value with a single instruction. However, there are processor implementations which do
//! not work like this. Given that they are rare we still recommend to double check if your
//! processor can read/write an `usize` in a single instruction.
//!
//! # Limits
//! As we are using holding an index and a state in a single `usize`, the maximum amount of
//! elements in this buffer is limited by the architecture it runs on. For a 64 bit architecture it
//! will be 32 bits, on a 32 bit architecture this will be 16 bits and on a 16bit architecture this
//! will only be 8 bits. We handle those variants in our implementation.
use arc::Arc;
use core::{cell::UnsafeCell, fmt::Debug, mem::ManuallyDrop, ptr::NonNull};

/// Minimal implementation of `Arc`
///
/// The reason for this is to reduce the dependency on `std` as this lib will be used in a `no_std`
/// environment.
mod arc;
#[cfg(test)]
mod tests;

/// Result to handle errors when writing or reading to the buffer
pub type Result<T> = core::result::Result<T, Error>;

/// Error codes for read and write
///
/// Keep in mind that those error codes do _not_ check if the [Producer] or [Consumer] got dropped.
/// This is intentional and it needs to be handled by the implementation which is using this
/// buffer.
pub enum Error {
    /// Unable to write to buffer because it is full
    Full,
    /// Unable to read from the buffer because it is empty
    Empty,
}

impl Debug for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Full => write!(f, "Buffer is full"),
            Self::Empty => write!(f, "Buffer is empty"),
        }
    }
}

/// Split tag into its two parts: index and state
///
/// Handling 64 bit pointer width.
#[cfg(target_pointer_width = "64")]
#[inline]
fn split(tag: usize) -> (usize, usize) {
    (tag >> 32, tag & 0xffff_ffff)
}
/// Split tag into its two parts: index and state
///
/// Handling 32 bit pointer width.
#[cfg(target_pointer_width = "32")]
#[inline]
fn split(tag: usize) -> (usize, usize) {
    (tag >> 16, tag & 0xffff)
}
/// Split tag into its two parts: index and state
///
/// Handling 16 bit pointer width.
#[cfg(target_pointer_width = "16")]
#[inline]
fn split(tag: usize) -> (usize, usize) {
    (tag >> 8, tag & 0x00ff)
}

/// Merge index and state into a tag
///
/// Handling 64 bit pointer width.
#[cfg(target_pointer_width = "64")]
#[inline]
fn merge(index: usize, state: usize) -> usize {
    index << 32 | state
}

/// Merge index and state into a tag
///
/// Handling 32 bit pointer width.
#[cfg(target_pointer_width = "32")]
#[inline]
fn merge(index: usize, state: usize) -> usize {
    index << 16 | state
}

/// Merge index and state into a tag
///
/// Handling 16 bit pointer width.
#[cfg(target_pointer_width = "16")]
#[inline]
fn merge(index: usize, state: usize) -> usize {
    index << 8 | state
}

/// Return the smallest value
#[inline]
fn min(a: usize, b: usize) -> usize {
    if a > b {
        b
    } else {
        a
    }
}

/// Provide write operation to the buffer
///
/// # Safety
/// The internal buffer is always guranteed to exist until both, [Producer] and [Consumer] are
/// dropped. However, if the [Consumer] is dropped and the buffer is full, it will not be possible
/// to write any data to the buffer again.
pub struct Producer<const N: usize> {
    /// Pointer to internal buffer to avoid access through `Arc`
    buffer_ptr: NonNull<u8>,
    /// Internal buffer
    buffer: Arc<Buffer<N>>,
    /// Cached tail-index
    tail_index: usize,
    /// Cached tail-state
    tail_state: usize,
    /// Length of first cached continious writeable buffer
    length0: usize,
    /// Length of second cached continious writeable buffer
    length1: usize,
    /// Length of cached writeable buffer
    /// As this will be checked regulary its kept in a single field
    length: usize,
    /// Last known state of head-tag
    /// This is used to determine if we can update the cache
    last_head_tag: usize,
}

/// This is safe because we are using an atomic reference counter to share the internal buffer
unsafe impl<const N: usize> Send for Producer<N> {}
/// This is safe because [Producer] and [Consumer] have mutual exclusive access to the buffer
unsafe impl<const N: usize> Sync for Producer<N> {}

impl<const N: usize> Producer<N> {
    /// Return the wrtiable length of the buffer
    ///
    /// Keep in mind that this is an expensive operation as it will not use the cached values.
    pub fn len(&self) -> usize {
        N - self.buffer.len()
    }

    /// Return if the buffer is full
    ///
    /// Keep in mind that this is an expensive operation as it will not use the cached values.
    pub fn is_full(&self) -> bool {
        self.buffer.len() == N
    }

    /// Return if [Consumer] was dropped
    ///
    /// Keep in mind that this is an expensive operation.
    pub fn is_abandoned(&self) -> bool {
        self.buffer.is_last()
    }

    /// Create a new instance with the correct initial values
    #[inline]
    fn new(buffer: Arc<Buffer<N>>) -> Self {
        Producer {
            buffer_ptr: buffer.buffer,
            buffer,
            tail_index: 0,
            tail_state: 0,
            length0: N,
            length1: 0,
            length: N,
            last_head_tag: 0,
        }
    }

    /// Return a mutable pointer from the internal buffer with the given offset
    #[inline]
    fn get_mut_ptr(&mut self, offset: usize) -> *mut u8 {
        unsafe { self.buffer_ptr.as_ptr().add(offset) }
    }

    /// Update the state of the buffer after data was written
    ///
    /// This will update the local cached values as well as the value from the internal buffer.
    #[inline]
    fn update_buffer(&mut self, bytes: usize) {
        // update cached values
        self.tail_state += 1;
        self.tail_index = (self.tail_index + bytes) % N;

        let tail_tag = merge(self.tail_index, self.tail_state);

        // set new uncached value to the buffer
        unsafe { *self.buffer.tail_tag.get() = tail_tag };
    }

    /// Update local cache
    ///
    /// Get the values from the internal buffer and update the cache. If there was no change since
    /// the last check, we will return false indicating that no value changed.
    #[inline]
    fn update_cache(&mut self) -> bool {
        // fast-path -> check if something changed since last check
        let head_tag = self.buffer.get_head_tag();
        if self.last_head_tag == head_tag {
            false
        } else {
            // get actual data
            let (head_index, _head_state) = split(head_tag);
            let (tail_index, last_state) = split(self.buffer.get_tail_tag());

            // update lengths
            if head_index > tail_index {
                // |***L--------Z***|
                self.length0 = head_index - tail_index;
                self.length1 = 0;
                self.length = self.length0;
            } else {
                // |--Z*****L-------|
                self.length0 = N - tail_index;
                self.length1 = head_index;
                self.length = self.length0 + self.length1;
            };

            // update rest
            self.tail_index = tail_index;
            self.tail_state = last_state;
            self.last_head_tag = head_tag;

            true
        }
    }

    /// Write to the frist cached buffer
    ///
    /// This will write to the internal buffer until there is no space left or the buffer ends.
    /// Wrapped buffer writes are done in the `write_second_buffer` function.
    #[inline]
    fn write_first_buffer(&mut self, buf: &[u8], buf_len: usize) -> usize {
        let bytes0 = min(self.length0, buf_len);
        if bytes0 > 0 {
            // copy data
            let ptr = self.get_mut_ptr(self.tail_index);
            unsafe { core::ptr::copy_nonoverlapping(buf.as_ptr(), ptr, bytes0) };

            // update state
            self.length0 -= bytes0;
            self.length -= bytes0;
        }

        bytes0
    }

    /// Write to the second cached buffer
    ///
    /// This is only called if the cached buffer is wrapping around the internal buffer and the
    /// cache was not updated since.
    #[inline]
    fn write_second_buffer(&mut self, buf: &[u8], buf_len: usize, bytes0: usize) -> usize {
        let bytes1 = min(self.length1, buf_len - bytes0);
        if bytes1 > 0 {
            // calculate offset - try to avoid modulo operation
            let offset = if bytes0 == 0 {
                self.tail_index
            } else {
                (self.tail_index + bytes0) % N
            };

            // copy data
            let ptr = self.get_mut_ptr(offset);
            unsafe { core::ptr::copy_nonoverlapping(buf.as_ptr().add(bytes0), ptr, bytes1) };

            // update state
            self.length1 -= bytes1;
            self.length -= bytes1;
        }

        bytes1
    }
}

impl<const N: usize> Producer<N> {
    /// Non blocking write to the buffer
    ///
    /// If [Consumer] got dropped it is still safe to write to the buffer until it is full.
    /// However, the data will never be read from the buffer and all writes will return an
    /// error after the buffer is full. The implementation needs to take care of that!
    ///
    /// # Returns
    /// On success, the amount of written data is returned. If the buffer is full, an error will be
    /// returned.
    #[inline]
    pub fn write(&mut self, buf: &[u8]) -> Result<usize> {
        let buf_len = buf.len();

        // check if we need to update our infos
        if self.length < buf_len && !self.update_cache() && self.length == 0 {
            Err(Error::Full)
        } else {
            //  -> check first slice
            let bytes0 = self.write_first_buffer(buf, buf_len);

            // if there are still bytes left in length0 - we are done
            if self.length0 > 0 {
                self.update_buffer(bytes0);
                return Ok(bytes0);
            }

            //  -> check second slice
            let bytes1 = self.write_second_buffer(buf, buf_len, bytes0);

            // update state
            self.update_buffer(bytes0 + bytes1);

            Ok(bytes0 + bytes1)
        }
    }
}

/// Provide read operation to the buffer
///
/// # Safety
/// The internal buffer is always guranteed to exist until both, [Producer] and [Consumer] are
/// dropped. However, if the [Producer] part is dropped and the buffer is empty, it will not be
/// possible to read any data from the buffer again.
pub struct Consumer<const N: usize> {
    /// Pointer to internal buffer to avoid access through `Arc`
    buffer_ptr: NonNull<u8>,
    /// Internal buffer
    buffer: Arc<Buffer<N>>,
    /// Cached head-index
    head_index: usize,
    /// Cached head-state
    head_state: usize,
    /// Length of first cached continious readable buffer
    length0: usize,
    /// Length of second cached continious readable buffer
    length1: usize,
    /// Length of cached readable buffer
    /// As this will be checked regulary its kept in a single field
    length: usize,
    /// Last known state of tail-tag
    /// This is used to determine if we can update the cache
    last_tail_tag: usize,
}

/// This is safe because we are using an atomic reference counter to share the internal buffer
unsafe impl<const N: usize> Send for Consumer<N> {}
/// This is safe because [Producer] and [Consumer] have mutual exclusive access to the buffer
unsafe impl<const N: usize> Sync for Consumer<N> {}

impl<const N: usize> Consumer<N> {
    /// Return the readable length of the buffer
    ///
    /// Keep in mind that this is an expensive operation as it will not use the cached values.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Return if the buffer is empty
    ///
    /// Keep in mind that this is an expensive operation as it will not use the cached values.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Return if [Producer] was dropped
    ///
    /// Keep in mind that this is an expensive operation.
    pub fn is_abandoned(&self) -> bool {
        self.buffer.is_last()
    }

    /// Create a new instance with the correct initial values
    #[inline]
    fn new(buffer: Arc<Buffer<N>>) -> Self {
        Consumer {
            buffer_ptr: buffer.buffer,
            buffer,
            head_index: 0,
            head_state: 0,
            length0: 0,
            length1: 0,
            length: 0,
            last_tail_tag: 0,
        }
    }

    /// Return a const pointer to the internal buffer with the given offset
    #[inline]
    fn get_ptr(&mut self, offset: usize) -> *const u8 {
        unsafe { self.buffer_ptr.as_ptr().add(offset) }
    }

    /// Update the state of the buffer after data was read
    ///
    /// This iwll update the local cached values as well as the value from the internal buffer.
    #[inline]
    fn update_buffer(&mut self, bytes: usize) {
        self.head_state = self.last_tail_tag & 0x00ff_ffff;
        self.head_index = (self.head_index + bytes) % N;

        let head_tag = self.head_index << 32 | self.head_state;

        // set new value
        unsafe { *self.buffer.head_tag.get() = head_tag };
    }

    /// Update local cache
    ///
    /// Get the values from the internal buffer and update the cache. If there was no change since
    /// the last check, it will return false indicating that no value changed.
    #[inline]
    fn update_cache(&mut self) -> bool {
        // fast-path -> check if something changed since last check
        let tail_tag = self.buffer.get_tail_tag();
        if self.last_tail_tag == tail_tag {
            false
        } else {
            // get actual data
            let (head_index, head_state) = split(self.buffer.get_head_tag());
            let (tail_index, tail_state) = split(tail_tag);

            // update lengths
            if head_index == tail_index && head_state != tail_state {
                // |****Z***********|
                self.length0 = N - head_index;
                self.length1 = head_index;
                self.length = N;
            } else if head_index > tail_index {
                // |***L--------Z***|
                self.length0 = N - head_index;
                self.length1 = tail_index;
                self.length = self.length0 + self.length1;
            } else {
                // |--Z*****L-------|
                self.length0 = tail_index - head_index;
                self.length1 = 0;
                self.length = self.length0;
            };

            // update rest
            self.head_index = head_index;
            self.head_state = head_state;
            self.last_tail_tag = tail_tag;

            true
        }
    }

    /// Read from the frist cached buffer
    ///
    /// This will read from the internal buffer until ther is no data left or the buffer ends.
    /// Wrapped buffer reads are done in the `read_second_buffer` function.
    #[inline]
    fn read_first_buffer(&mut self, buf: &mut [u8], buf_len: usize) -> usize {
        let bytes0 = min(self.length0, buf_len);
        if bytes0 > 0 {
            // copy data
            let ptr = self.get_ptr(self.head_index);
            unsafe { core::ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), bytes0) };

            // update state
            self.length0 -= bytes0;
            self.length -= bytes0;
        }

        bytes0
    }

    /// Read from the second cached buffer
    ///
    /// This is only called if the cached buffer is wrapping around the internal buffer and the
    /// cache was not updatred since.
    #[inline]
    fn read_second_buffer(&mut self, buf: &mut [u8], buf_len: usize, bytes0: usize) -> usize {
        let bytes1 = min(self.length1, buf_len - bytes0);
        if bytes1 > 0 {
            // calculate offset - try to avoid modulo operation
            let offset = if bytes0 == 0 {
                self.head_index
            } else {
                (self.head_index + bytes0) % N
            };

            // copy data
            let ptr = self.get_ptr(offset);
            unsafe { core::ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr().add(bytes0), bytes1) };

            // update state
            self.length1 -= bytes1;
            self.length -= bytes1;
        }

        bytes1
    }
}

/// Implement `Read` to make it easy to read from the buffer
impl<const N: usize> Consumer<N> {
    /// Non blocking read from the buffer
    ///
    /// If [Producer] got dropped it is still possible to read from the buffer until it is empty.
    /// However, the buffer will never be written again and all future reads will return an
    /// error indicating that the buffer is empty. The implementation needs to take care of that!
    ///
    /// # Returns
    /// On success, the amount of written data is returned. If the buffer is empty, an error will
    /// be returned.
    #[inline]
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let buf_len = buf.len();

        // check if we need to update our infos
        if self.length < buf_len && !self.update_cache() && self.length == 0 {
            Err(Error::Empty)
        } else {
            //  -> check first slice
            let bytes0 = self.read_first_buffer(buf, buf_len);

            // if there are still bytes left in length0 - we are done
            if self.length0 > 0 {
                self.update_buffer(bytes0);
                return Ok(bytes0);
            }

            //  -> check second slice
            let bytes1 = self.read_second_buffer(buf, buf_len, bytes0);

            // update state
            self.update_buffer(bytes0 + bytes1);

            Ok(bytes0 + bytes1)
        }
    }
}

/// Lock-free byte buffer without locks or atomic operations for synchronization
pub struct Buffer<const N: usize> {
    /// Internal buffer accessed by [Producer] and [Consumer]
    ///
    /// Both parts will _always_ have mutual exclusive access to their corresponding parts which
    /// makes it safe to access this buffer at the same time.
    buffer: NonNull<u8>,

    /// Holds the index of the beginning of the readable data and the last known state
    ///
    /// The state part is needed to ensure that we can distinguish if the buffer is full or emtpy.
    /// This field can only be changed by the [Consumer]. The [Producer] is only allowed to read.
    head_tag: UnsafeCell<usize>,

    /// Holds the index of the beginning of the writeable data and its state
    ///
    /// The state gets incremented each time the buffer got written. This makes it easy for the
    /// [Consumer] to track if there were changes.
    /// This field can only be changed by the [Producer]. The [Consumer] is only allowed to read.
    tail_tag: UnsafeCell<usize>,
}

impl<const N: usize> Buffer<N> {
    /// Create a new `Buffer` instance
    ///
    /// Creating a new Buffer will give you a [Producer] and [Consumer]. Each of them can be moved
    /// to different threads and do not need any additional locks.
    /// The [Producer] can be used to write data onto the buffer, while the [Consumer] will be used
    /// to read from it.
    ///
    /// # Panics
    /// Ensure that the size of the buffer do not exceed the maximum supported size for your
    /// architecture. If it does, it will panic!
    ///
    /// | Pointer width | buffer size |
    /// |:-------------:|:-----------:|
    /// | `64 bit`      | `32 bit`    |
    /// | `32 bit`      | `16 bit`    |
    /// | `16 bit`      |  `8 bit`    |
    ///
    ///
    /// # Example
    /// ```
    /// use dichotomy::Buffer;
    ///
    /// const DATA: &[u8] = b"hello world";
    ///
    /// let (mut producer, mut consumer) = Buffer::<1024>::new();
    /// std::thread::spawn(move || {
    ///     // spawn consumer thread
    ///     let mut buf = [0u8; 32];
    ///     for _ in 0..10 {
    ///         let mut read = 0;
    ///         while read < DATA.len() {
    ///             let bytes = consumer.read(&mut buf[..DATA.len()]).unwrap();
    ///             read += bytes;
    ///         }
    ///     }
    /// });
    ///
    /// for _ in 0..10 {
    ///     let mut written = 0;
    ///     while written < DATA.len() {
    ///         let bytes = producer.write(&DATA[written..]).unwrap();
    ///         written += bytes;
    ///     }
    /// }
    /// ```
    pub fn new() -> (Producer<N>, Consumer<N>) {
        // ensure that the size does not exceed the maximum allowed size for a 64 bit system
        #[cfg(target_pointer_width = "64")]
        assert!(u32::try_from(N).is_ok());
        // ensure that the size does not exceed the maximum allowed size for a 32 bit system
        #[cfg(target_pointer_width = "32")]
        assert!(u16::try_from(N).is_ok());
        // ensure that the size does not exceed the maximum allowed size for a 16 bit system
        #[cfg(target_pointer_width = "16")]
        assert!(u8::try_from(N).is_ok());

        let buffer =
            unsafe { NonNull::new_unchecked(ManuallyDrop::new(vec![0u8; N]).as_mut_ptr()) };

        let buffer = Self {
            buffer,
            head_tag: UnsafeCell::default(),
            tail_tag: UnsafeCell::default(),
        };

        let buffer = Arc::new(buffer);
        let consumer = Consumer::new(buffer.clone());
        let producer = Producer::new(buffer);

        (producer, consumer)
    }
}

/// Functions which are used by both, the [Producer] and the [Consumer]
impl<const N: usize> Buffer<N> {
    /// Return the length of the data on the buffer
    #[inline]
    fn len(&self) -> usize {
        let head_tag = self.get_head_tag();
        let tail_tag = self.get_tail_tag();

        // check if buffer is empty
        if head_tag == tail_tag {
            return 0;
        }

        let (head_index, head_state) = split(head_tag);
        let (tail_index, tail_state) = split(tail_tag);

        // check if buffer is full
        if head_index == tail_index && head_state != tail_state {
            return N;
        }

        // check if data is wrapping
        if head_index <= tail_index {
            // |---H*****T-------|
            tail_index - head_index
        } else {
            // |*T----------H****|
            N - head_index + tail_index
        }
    }

    /// Return if the buffer is currently empty
    #[inline]
    fn is_empty(&self) -> bool {
        let head_tag = self.get_head_tag();
        let tail_tag = self.get_tail_tag();

        // check if buffer is empty
        head_tag == tail_tag
    }

    /// Return the head-tag
    #[inline]
    fn get_head_tag(&self) -> usize {
        unsafe { core::ptr::read_volatile(self.head_tag.get()) }
    }

    /// Return the tail-tag
    #[inline]
    fn get_tail_tag(&self) -> usize {
        unsafe { core::ptr::read_volatile(self.tail_tag.get()) }
    }
}
