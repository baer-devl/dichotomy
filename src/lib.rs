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
//! |----Z**********L-----|
//!
//! 'Z' representing the index of `zero_tag` and 'L' representing the index of `last_tag`. As both
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

/// Ring buffer abstraction helper
//mod helper;

#[cfg(test)]
mod tests;

#[inline]
fn split_tag(tag: usize) -> (usize, usize) {
    (tag >> 32, tag & 0x00ff_ffff)
}

/// Writable part of the buffer
///
/// # Safety
/// It is safe to move this instance into other threads as the `Arc` ensures that the underlaying
/// buffer does exist. However, if the `Consumer` part is dropped and the buffer is full, it will
/// not be possible to write more data to the buffer.
pub struct Producer<const N: usize> {
    buffer: Arc<Buffer<N>>,
    last_index: usize,
    last_state: usize,
    length0: usize,
    length1: usize,
    length: usize,
    known_zero_tag: usize,
}

/// This is safe because we are using `Arc` to share the underlaying buffer
unsafe impl<const N: usize> Send for Producer<N> {}
/// This is safe because we are using `Arc` to share the underlaying buffer
unsafe impl<const N: usize> Sync for Producer<N> {}

impl<const N: usize> Producer<N> {
    /// Return the current length of the buffer ready to write to
    pub fn len(&self) -> usize {
        N - self.buffer.len()
    }

    /// Return if the buffer is currently full
    pub fn is_full(&self) -> bool {
        self.buffer.len() == N
    }

    /// Return if `Consumer` part still exists
    pub fn is_consumer_available(&self) -> bool {
        Arc::strong_count(&self.buffer) > 1
    }

    #[inline]
    fn update_buffer(&mut self, bytes: usize) {
        self.last_state += 1;
        self.last_index = (self.last_index + bytes) % N;

        let last_tag = self.last_index << 32 | self.last_state;

        // set new value
        unsafe { *self.buffer.last_tag.get() = last_tag };
    }

    #[inline]
    fn update_cache(&mut self) -> bool {
        // fast-path -> check if something changed since last check
        let zero_tag = self.buffer.get_zero_tag();
        if self.known_zero_tag == zero_tag {
            return false;
            /*return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "No space left",
            ));*/
        }

        // get actual data
        let (zero_index, _zero_state) = split_tag(zero_tag);
        let (last_index, last_state) = self.buffer.get_last_tag_splitted();

        // update lengths
        if zero_index > last_index {
            // |***L--------Z***|
            self.length0 = zero_index - last_index;
            self.length1 = 0;
            self.length = self.length0;
        } else {
            // |--Z*****L-------|
            self.length0 = N - last_index;
            self.length1 = zero_index;
            self.length = self.length0 + self.length1;
        };

        // update rest
        self.last_index = last_index;
        self.last_state = last_state;
        self.known_zero_tag = zero_tag;

        true
    }

    #[inline]
    fn write_first_buffer(&mut self, buf: &[u8], buf_len: usize) -> usize {
        let bytes0 = std::cmp::min(self.length0, buf_len);
        if bytes0 > 0 {
            // copy data
            unsafe {
                let ptr = (*self.buffer.buffer.get()).as_mut_ptr();
                std::ptr::copy_nonoverlapping(
                    buf.as_ptr(),
                    ptr.offset(self.last_index as isize),
                    bytes0,
                );
            }

            // update state
            self.length0 -= bytes0;
            self.length -= bytes0;
        }

        bytes0
    }

    #[inline]
    fn write_second_buffer(&mut self, buf: &[u8], buf_len: usize, bytes0: usize) -> usize {
        let bytes1 = std::cmp::min(self.length1, buf_len - bytes0);
        if bytes1 > 0 {
            // copy data
            unsafe {
                let ptr = (*self.buffer.buffer.get()).as_mut_ptr();
                std::ptr::copy_nonoverlapping(
                    buf.as_ptr().offset(bytes0 as isize),
                    ptr.offset(((self.last_index + bytes0) % N) as isize),
                    bytes1,
                );
            }

            // update state
            self.length1 -= bytes1;
            self.length -= bytes1;
        }

        bytes1
    }
}

/// Implement `Write` to make it easy to write to the buffer
impl<const N: usize> Write for Producer<N> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let buf_len = buf.len();

        // check if we need to update our infos
        if self.length < buf_len {
            if !self.update_cache() && self.length == 0 {
                return Ok(0);
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
    zero_index: usize,
    zero_state: usize,
    length0: usize,
    length1: usize,
    length: usize,
    known_last_tag: usize,
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

    /// Return if the buffer is currently full
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Return if `Producer` part still exists
    pub fn is_producer_available(&self) -> bool {
        Arc::strong_count(&self.buffer) > 1
    }

    #[inline]
    fn update_buffer(&mut self, bytes: usize) {
        self.zero_state = self.known_last_tag & 0x00ff_ffff;
        self.zero_index = (self.zero_index + bytes) % N;

        let zero_tag = self.zero_index << 32 | self.zero_state;

        // set new value
        unsafe { *self.buffer.zero_tag.get() = zero_tag };
    }

    #[inline]
    fn update_cache(&mut self) -> bool {
        // fast-path -> check if something changed since last check
        let last_tag = self.buffer.get_last_tag();
        if self.known_last_tag == last_tag {
            return false;
            /*return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "No data to read",
            ));*/
        }

        // get actual data
        let (zero_index, zero_state) = self.buffer.get_zero_tag_splitted();
        let (last_index, last_state) = split_tag(last_tag);

        // update lengths
        if zero_index == last_index && zero_state != last_state {
            // |****Z***********|
            self.length0 = N - zero_index;
            self.length1 = zero_index;
            self.length = N;
        } else if zero_index > last_index {
            // |***L--------Z***|
            self.length0 = N - zero_index;
            self.length1 = last_index;
            self.length = self.length0 + self.length1;
        } else {
            // |--Z*****L-------|
            self.length0 = last_index - zero_index;
            self.length1 = 0;
            self.length = self.length0;
        };

        // update rest
        self.zero_index = zero_index;
        self.zero_state = zero_state;
        self.known_last_tag = last_tag;

        true
    }
}

/// Implement `Read` to make it easy to read from the buffer
impl<const N: usize> Read for Consumer<N> {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let buf_len = buf.len();

        // check if we need to update our infos
        // TODO: maybe use length0 here and update all if first one is empty (depending on how much
        // the update process costs
        if self.length < buf_len {
            if !self.update_cache() && self.length == 0 {
                return Ok(0);
            }
        }

        //  -> check first slice
        let bytes0 = std::cmp::min(self.length0, buf_len);
        if bytes0 > 0 {
            // copy data
            unsafe {
                let ptr = (*self.buffer.buffer.get()).as_ptr();
                std::ptr::copy_nonoverlapping(
                    ptr.offset(self.zero_index as isize),
                    buf.as_mut_ptr(),
                    bytes0,
                )
            };

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
        let bytes1 = std::cmp::min(self.length1, buf_len - bytes0);
        if bytes1 > 0 {
            // copy data
            unsafe {
                let ptr = (*self.buffer.buffer.get()).as_ptr();
                std::ptr::copy_nonoverlapping(
                    ptr.offset(((self.zero_index + bytes0) % N) as isize),
                    buf.as_mut_ptr().offset(bytes0 as isize),
                    bytes1,
                )
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
pub struct Buffer<const N: usize> {
    /// Internal buffer accessed by `Producer` and `Consumer`
    ///
    /// Both parts will _always_ have mutual exclusive access their corresponding parts which makes
    /// it safe to access this buffer at the same time.
    buffer: UnsafeCell<[u8; N]>,

    /// Marks the beginning of the readable data and the last known state of the `last_tag` state
    ///
    /// This field can only be changed by the `Consumer`. The `Producer` is only allowed to read.
    zero_tag: UnsafeCell<usize>,

    /// Marks the beginning of the writeable data and the last state
    ///
    /// This field can only be changed by the `Producer`. The `Consumer` is only allowed to read.
    last_tag: UnsafeCell<usize>,
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
            zero_tag: UnsafeCell::default(),
            last_tag: UnsafeCell::default(),
        };

        buffer.split()
    }
}

/// Functions which are used by both, the `Producer` and the `Consumer`
impl<const N: usize> Buffer<N> {
    /// Return the current length of the data on the buffer
    #[inline]
    fn len(&self) -> usize {
        let zero_tag = unsafe { *self.zero_tag.get() };
        let last_tag = unsafe { *self.last_tag.get() };

        // check if buffer is empty
        if zero_tag == last_tag {
            return 0;
        }

        let zero_index = zero_tag >> 32;
        let zero_state = zero_tag & 0x00ff_ffff;

        let last_index = last_tag >> 32;
        let last_state = last_tag & 0x00ff_ffff;

        // check if buffer is full
        if zero_index == last_index && zero_state != last_state {
            return N;
        }

        // check if data is wrapping
        if zero_index <= last_index {
            // |---S*****E-------|
            last_index - zero_index
        } else {
            // |*E----------S****|
            N - zero_index + last_index
        }
    }

    /// Return if the buffer is currently empty
    #[inline]
    fn is_empty(&self) -> bool {
        let zero_tag = unsafe { *self.zero_tag.get() };
        let last_tag = unsafe { *self.last_tag.get() };

        // check if buffer is empty
        zero_tag == last_tag
    }

    #[inline]
    fn get_zero_tag(&self) -> usize {
        unsafe { *self.zero_tag.get() }
    }
    #[inline]
    fn get_last_tag(&self) -> usize {
        unsafe { *self.last_tag.get() }
    }
    #[inline]
    fn get_zero_tag_splitted(&self) -> (usize, usize) {
        split_tag(self.get_zero_tag())
    }
    #[inline]
    fn get_last_tag_splitted(&self) -> (usize, usize) {
        split_tag(self.get_last_tag())
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
            zero_index: 0,
            zero_state: 0,
            length0: 0,
            length1: 0,
            length: 0,
            known_last_tag: 0,
        };

        let producer = Producer {
            buffer,
            last_index: 0,
            last_state: 0,
            length0: N,
            length1: 0,
            length: N,
            known_zero_tag: 0,
        };

        (producer, consumer)
    }
}
