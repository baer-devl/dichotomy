//! Dichotomy is a lock-free binary ring buffer
//!
//! The buffer itself does not use any locks like `Mutex` or `RwLock`, nor make use of atomics. The
//! two parts, `Producer` and `Consumer` are guranteed to have mutual exclusive access to the part
//! of the buffer they need access to. Thus makes the buffer very performant as it does not need to
//! handle locks.
//!
//! There are two tags used to handle the internal access to the buffer called `zero_tag` and
//! `last_tag`. Both will hold an index and a state. The index will give the position of the start
//! and the end of written data. The state ensures that the `Consumer` part does have the
//! information if there is more to read from. The `zero_tag` can only be written by the `Consumer`
//! and the `last_tag` can only be written by the `Producer`. This and the fact that the processort
//! _should_ be able to read an usize in a single instruction, makes it safe to access those values
//! without locks.
//!
//! # Deep dive
//! Lets first focus on the index part of the tags.
//! The `Producer` holds its index in the `last_tag` which marks the beginning of the free buffer.
//! If there is space left in the buffer, it will write into it and update the index.
//! The very same is true for the `Consumer`, who holds its index in the `zero_tag`, indicating the
//! beginning of readable data. If the data gets consumed, it will update the index and free space
//! on the buffer.
//! If both indicies are the same, this could either mean that the buffer is empty of full. To make
//! a distinction of those two possibilities, we are using the state part of the tags which is an
//! incrementing number changed by each write to the buffer.
//! If the `Consumer` check if the buffer has data to read from, it will check if the tags will
//! differ. If that is the case, the index of both values will be used to calculate the readable
//! size of the data. Each time the `Consumer` reads data from the buffer, it will update its state
//! to the last known state of the `Producer` tag. This ensures that there will never be a false
//! empty buffer because the state will always be different after the `Producer` wrote new data.
//! Here are some examples of possible states of the two tags:
//!
//! ## Empty buffer
//! |--------X------------|
//!
//! 'X' representing both indicies of the tags which pointing to the same position. In this special
//! case, the state of both tags have to be the same. Thus makes it easy to check, if both tags are
//! the same, the buffer is empty.
//!
//! ## Filled buffer
//! |----H**********T-----|
//!
//! 'H' representing the index of `head_tag` and 'T' representing the index of `tail_tag`. As both
//! indicies are different, it is safe to assume that there is data to read from and free space to
//! write to. Both, the free space and the filled spaces can easily calculated.
//!
//! ## Full buffer
//! |*****************X***|
//!
//! 'X' representing both indicies of the tags pointing to the same position. However, this time
//! the state of the tags differ which indicates that the buffer must be full.
//!
//! # Safety
//! The whole safety of this implementation relies on the fact that the processor can read an usize
//! value with a single instruction. However, there are processor implementations which do not work
//! like this. Given that they are rare we still recommend to double check if your processor can
//! read a single usize in one instruction.
//!
//! # Limits
//! As we are using the usize value as two different parts, the maximum amount of elements in this
//! buffer is limited by the architecture it runs on. For a 64 bit architecture it will be 32 bits.
//! On a 32 bit architecture this will be 16 bits. We handle those two variants in our
//! implementation.
//!
//!
//! # Features
//! This is the part where we talk about features.
use std::{
    cell::UnsafeCell,
    io::{Read, Write},
    sync::Arc,
};

#[cfg(test)]
mod tests;

/// Splits a tag into its two parts, index and state
#[inline]
fn split_tag(tag: usize) -> (usize, usize) {
    (tag >> 32, tag & 0x00ff_ffff)
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

/// Writable part of the buffer
///
/// # Safety
/// It is safe to move this instance into other threads as the `Arc` ensures that the underlaying
/// buffer does exist. However, if the `Consumer` part is dropped and the buffer is full, it will
/// not be possible to write more data to the buffer.
pub struct Producer<const N: usize> {
    buffer: Arc<Buffer<N>>,
    tail_index: usize,
    tail_state: usize,
    length0: usize,
    length1: usize,
    length: usize,
    last_head_tag: usize,
}

/// This is safe because we are using `Arc` to share the underlaying buffer
unsafe impl<const N: usize> Send for Producer<N> {}
/// This is safe because we are using `Arc` to share the underlaying buffer
unsafe impl<const N: usize> Sync for Producer<N> {}

impl<const N: usize> Producer<N> {
    /// Return the current length of the buffer that is writeable
    pub fn len(&self) -> usize {
        N - self.buffer.len()
    }

    /// Return if the buffer is currently full
    pub fn is_full(&self) -> bool {
        self.buffer.len() == N
    }

    /// Return if `Consumer` part was dropped
    pub fn is_abandoned(&self) -> bool {
        Arc::strong_count(&self.buffer) == 1
    }

    /// Update the state of the buffer after data was written
    #[inline]
    fn update_buffer(&mut self, bytes: usize) {
        self.tail_state += 1;
        self.tail_index = (self.tail_index + bytes) % N;

        let tail_tag = self.tail_index << 32 | self.tail_state;

        // set new value
        unsafe { *self.buffer.tail_tag.get() = tail_tag };
    }

    /// Update the current cache
    ///
    /// This is expensive and should only be done as rarely as possible.
    #[inline]
    fn update_cache(&mut self) -> bool {
        // fast-path -> check if something changed since last check
        let head_tag = self.buffer.get_head_tag();
        if self.last_head_tag == head_tag {
            return false;
        }

        // get actual data
        let (head_index, _head_state) = split_tag(head_tag);
        let (tail_index, last_state) = self.buffer.get_tail_tag_splitted();

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

    /// Return the mutable pointer to the internal buffer with the correct offset
    #[inline]
    fn get_mut_ptr(&mut self, offset: isize) -> *mut u8 {
        unsafe { (*self.buffer.buffer.get()).as_mut_ptr().offset(offset) }
    }

    #[inline]
    fn write_first_buffer(&mut self, buf: &[u8], buf_len: usize) -> usize {
        let bytes0 = min(self.length0, buf_len);
        if bytes0 > 0 {
            // copy data
            let ptr = self.get_mut_ptr(self.tail_index as isize);
            unsafe { std::ptr::copy_nonoverlapping(buf.as_ptr(), ptr, bytes0) };

            // update state
            self.length0 -= bytes0;
            self.length -= bytes0;
        }

        bytes0
    }

    #[inline]
    fn write_second_buffer(&mut self, buf: &[u8], buf_len: usize, bytes0: usize) -> usize {
        let bytes1 = min(self.length1, buf_len - bytes0);
        if bytes1 > 0 {
            // copy data
            let ptr = self.get_mut_ptr(((self.tail_index + bytes0) % N) as isize);
            unsafe {
                std::ptr::copy_nonoverlapping(buf.as_ptr().offset(bytes0 as isize), ptr, bytes1)
            };

            // update state
            self.length1 -= bytes1;
            self.length -= bytes1;
        }

        bytes1
    }
}

/// Implement `Write` to make it easy to write to the buffer
#[cfg(not(features = "generic"))]
impl<const N: usize> Write for Producer<N> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let buf_len = buf.len();

        // check if we need to update our infos
        if self.length < buf_len {
            if !self.update_cache() && self.length == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::OutOfMemory,
                    "No space left",
                ));
            }
        }

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

    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Readable part of the buffer
///
/// # Safety
/// It is safe to move this instance into other threads as the `Arc` ensures that the underlaying
/// buffer does exist. However, if the `Producer` part is dropped and the buffer is empty, it will
/// not be possible to read more data from the buffer.
pub struct Consumer<const N: usize> {
    buffer: Arc<Buffer<N>>,
    head_index: usize,
    head_state: usize,
    length0: usize,
    length1: usize,
    length: usize,
    last_tail_tag: usize,
}

/// This is safe because we are using `Arc` to share the underlaying buffer
unsafe impl<const N: usize> Send for Consumer<N> {}
/// This is safe because we are using `Arc` to share the underlaying buffer
unsafe impl<const N: usize> Sync for Consumer<N> {}

impl<const N: usize> Consumer<N> {
    /// Return the current length of the buffer
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Return if the buffer is currently empty
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Return if `Producer` was dropped
    pub fn is_abandoned(&self) -> bool {
        Arc::strong_count(&self.buffer) == 1
    }

    /// Return the mutable pointer to the internal buffer with the correct offset
    #[inline]
    fn get_mut_ptr(&mut self, offset: isize) -> *mut u8 {
        unsafe { (*self.buffer.buffer.get()).as_mut_ptr().offset(offset) }
    }

    #[inline]
    fn update_buffer(&mut self, bytes: usize) {
        self.head_state = self.last_tail_tag & 0x00ff_ffff;
        self.head_index = (self.head_index + bytes) % N;

        let head_tag = self.head_index << 32 | self.head_state;

        // set new value
        unsafe { *self.buffer.head_tag.get() = head_tag };
    }

    #[inline]
    fn update_cache(&mut self) -> bool {
        // fast-path -> check if something changed since last check
        let tail_tag = self.buffer.get_tail_tag();
        if self.last_tail_tag == tail_tag {
            return false;
        }

        // get actual data
        let (head_index, head_state) = self.buffer.get_head_tag_splitted();
        let (tail_index, tail_state) = split_tag(tail_tag);

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

/// Implement `Read` to make it easy to read from the buffer
#[cfg(not(features = "generic"))]
impl<const N: usize> Read for Consumer<N> {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let buf_len = buf.len();
        // check if we need to update our infos
        if self.length < buf_len {
            if !self.update_cache() && self.length == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::OutOfMemory,
                    "No data to read",
                ));
            }
        }

        //  -> check first slice
        let bytes0 = min(self.length0, buf_len);
        if bytes0 > 0 {
            // copy data
            let ptr = self.get_mut_ptr(self.head_index as isize);
            unsafe { std::ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), bytes0) };

            // update state
            self.length0 -= bytes0;
            self.length -= bytes0;

            // if there are still bytes left in length0 - we are done
            if self.length0 > 0 {
                self.update_buffer(bytes0);
                return Ok(bytes0);
            }
        }
        //  -> check second slice
        let bytes1 = min(self.length1, buf_len - bytes0);
        if bytes1 > 0 {
            // copy data
            let ptr = self.get_mut_ptr(((self.head_index + bytes0) % N) as isize);
            unsafe {
                std::ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr().offset(bytes0 as isize), bytes1)
            };

            // update state
            self.length1 -= bytes1;
            self.length -= bytes1;
        }

        self.update_buffer(bytes0 + bytes1);
        Ok(bytes0 + bytes1)
    }
}

/// Lock-free binary buffer without locks or atomic operations for synchronization
#[cfg(not(features = "generic"))]
pub struct Buffer<const N: usize> {
    /// Internal buffer accessed by `Producer` and `Consumer`
    ///
    /// Both parts will _always_ have mutual exclusive access their corresponding parts which makes
    /// it safe to access this buffer at the same time.
    #[cfg(not(feature = "dynamic"))]
    buffer: UnsafeCell<[u8; N]>,
    #[cfg(feature = "dynamic")]
    buffer: UnsafeCell<Vec<u8>>,

    /// Marks the beginning of the readable data and the last known state of the `tail_tag` state
    ///
    /// This field can only be changed by the `Consumer`. The `Producer` is only allowed to read.
    head_tag: UnsafeCell<usize>,

    /// Marks the beginning of the writeable data and the last state
    ///
    /// This field can only be changed by the `Producer`. The `Consumer` is only allowed to read.
    tail_tag: UnsafeCell<usize>,
}
#[cfg(features = "generic")]
pub struct Buffer<const N: usize, T: Send + Sync> {
    /// Internal buffer accessed by `Producer` and `Consumer`
    ///
    /// Both parts will _always_ have mutual exclusive access their corresponding parts which makes
    /// it safe to access this buffer at the same time.
    #[cfg(not(feature = "dynamic"))]
    buffer: UnsafeCell<[T; N]>,
    #[cfg(feature = "dynamic")]
    buffer: UnsafeCell<Vec<T>>,

    /// Marks the beginning of the readable data and the last known state of the `tail_tag` state
    ///
    /// This field can only be changed by the `Consumer`. The `Producer` is only allowed to read.
    head_tag: UnsafeCell<usize>,

    /// Marks the beginning of the writeable data and the last state
    ///
    /// This field can only be changed by the `Producer`. The `Consumer` is only allowed to read.
    tail_tag: UnsafeCell<usize>,
}

#[cfg(target_pointer_width = "64")]
impl<const N: usize> Buffer<N> {
    /// Create a new `Buffer` instance
    ///
    /// Creating a new Buffer will give you the `Producer` and `Consumer` parts. Each of them can
    /// be moved to different threads and can be used without additional locks.
    /// The `Producer` can be used to write data onto the buffer, while the `Consumer` will be used
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
    /// use std::io::{Read, Write};
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
    #[cfg(not(feature = "dynamic"))]
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

        let buffer = Self {
            buffer: UnsafeCell::new([0u8; N]),
            head_tag: UnsafeCell::default(),
            tail_tag: UnsafeCell::default(),
        };

        buffer.split()
    }

    #[cfg(feature = "dynamic")]
    pub fn new(capacity: usize) -> (Producer<N>, Consumer<N>) {
        // ensure that the size does not exceed the maximum allowed size for a 64 bit system
        #[cfg(target_pointer_width = "64")]
        assert!(u32::try_from(N).is_ok());
        // ensure that the size does not exceed the maximum allowed size for a 32 bit system
        #[cfg(target_pointer_width = "32")]
        assert!(u16::try_from(N).is_ok());
        // ensure that the size does not exceed the maximum allowed size for a 16 bit system
        #[cfg(target_pointer_width = "16")]
        assert!(u8::try_from(N).is_ok());

        let buffer = Self {
            buffer: UnsafeCell::new(Vec::with_capacity(capacity)),
            head_tag: UnsafeCell::default(),
            tail_tag: UnsafeCell::default(),
        };

        buffer.split()
    }
}

/// Functions which are used by both, the `Producer` and the `Consumer`
impl<const N: usize> Buffer<N> {
    /// Return the current length of the data on the buffer
    #[inline]
    fn len(&self) -> usize {
        let head_tag = self.get_head_tag();
        let tail_tag = self.get_tail_tag();

        // check if buffer is empty
        if head_tag == tail_tag {
            return 0;
        }

        let (head_index, head_state) = split_tag(head_tag);
        let (tail_index, tail_state) = split_tag(tail_tag);

        // check if buffer is full
        if head_index == tail_index && head_state != tail_state {
            return N;
        }

        // check if data is wrapping
        if head_index <= tail_index {
            // |---H*****T-------|
            println!("{}: {} - {}", N, head_index, tail_index);
            tail_index - head_index
        } else {
            // |*T----------H****|
            println!("{}: {} - {}", N, head_index, tail_index);
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

    #[inline]
    fn get_head_tag(&self) -> usize {
        unsafe { *self.head_tag.get() }
    }

    #[inline]
    fn get_tail_tag(&self) -> usize {
        unsafe { *self.tail_tag.get() }
    }

    #[inline]
    fn get_head_tag_splitted(&self) -> (usize, usize) {
        split_tag(self.get_head_tag())
    }

    #[inline]
    fn get_tail_tag_splitted(&self) -> (usize, usize) {
        split_tag(self.get_tail_tag())
    }
}

/// This functions will only be used from the `Buffer` internally
impl<const N: usize> Buffer<N> {
    /// Consume `Buffer` to create a `Producer` and a `Consumer`
    ///
    /// This is the only place where we are using atomics over the `Arc` implementation. This will
    /// not give us any performance penalties as this is only be used to ensure that the `Buffer`
    /// object does exist.
    fn split(self) -> (Producer<N>, Consumer<N>) {
        let buffer = Arc::new(self);

        let consumer = Consumer {
            buffer: buffer.clone(),
            head_index: 0,
            head_state: 0,
            length0: 0,
            length1: 0,
            length: 0,
            last_tail_tag: 0,
        };

        let producer = Producer {
            buffer,
            tail_index: 0,
            tail_state: 0,
            length0: N,
            length1: 0,
            length: N,
            last_head_tag: 0,
        };

        (producer, consumer)
    }
}
