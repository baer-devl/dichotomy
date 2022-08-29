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
use helper::BufferHelper;
use std::{
    cell::UnsafeCell,
    io::{Read, Write},
    marker::PhantomData,
    sync::Arc,
};

/// Ring buffer abstraction helper
mod helper;

#[cfg(test)]
mod tests;

/// Writable part of the buffer
///
/// # Safety
/// It is safe to move this instance into other threads as the `Arc` ensures that the underlaying
/// buffer does exist. However, if the `Consumer` part is dropped and the buffer is full, it will
/// not be possible to write more data to the buffer.
pub struct Producer<'a, const N: usize> {
    buffer: Arc<Buffer<'a, N>>,
    cache: BufferHelper,
    index: usize,
    state: usize,
}

/// This is safe because we are using `Arc` to share the underlaying buffer
unsafe impl<const N: usize> Send for Producer<'_, N> {}
/// This is safe because we are using `Arc` to share the underlaying buffer
unsafe impl<const N: usize> Sync for Producer<'_, N> {}

impl<const N: usize> Producer<'_, N> {
    /// Return the current length of the buffer
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Return if the buffer is currently full
    pub fn is_full(&self) -> bool {
        self.buffer.is_full()
    }

    /// Return if `Consumer` part still exists
    pub fn is_consumer_available(&self) -> bool {
        Arc::strong_count(&self.buffer) > 1
    }
}

/// Implement `Write` to make it easy to write to the buffer
impl<const N: usize> Write for Producer<'_, N> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // update cache
        if buf.len() > self.cache.len() {
            match self.buffer.get_writeable_buffer() {
                Some((index, state, buffer)) => {
                    self.cache = buffer;
                    self.index = index;
                    self.state = state;
                }
                None => return Ok(0),
            }
        }

        // write from buffer
        let bytes = self.cache.write(buf)?;

        // ensure we wrote something
        if bytes > 0 {
            // update end value
            self.state += 1;
            self.index = (self.index + bytes) % N;
            let end = self.index << 32 | self.state;

            // set new value
            unsafe { *self.buffer.last_tag.get() = end };

            Ok(bytes)
        } else {
            Ok(0)
        }
    }

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
pub struct Consumer<'a, const N: usize> {
    buffer: Arc<Buffer<'a, N>>,
    cache: BufferHelper,
    index: usize,
    state: usize,
}

/// This is safe because we are using `Arc` to share the underlaying buffer
unsafe impl<const N: usize> Send for Consumer<'_, N> {}
/// This is safe because we are using `Arc` to share the underlaying buffer
unsafe impl<const N: usize> Sync for Consumer<'_, N> {}

impl<const N: usize> Consumer<'_, N> {
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
}

/// Implement `Read` to make it easy to read from the buffer
impl<const N: usize> Read for Consumer<'_, N> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // update cache
        if buf.len() > self.cache.len() {
            match self.buffer.get_readable_buffer() {
                Some((index, state_end, buffer)) => {
                    self.cache = buffer;
                    self.index = index;
                    self.state = state_end;
                }
                None => return Ok(0),
            }
        }

        // read to buffer
        let bytes = self.cache.read(buf)?;

        // ensure we read something
        if bytes > 0 {
            // set new value
            self.index = (self.index + bytes) % N;
            let start = self.index << 32 | self.state;

            // new value
            unsafe { *self.buffer.zero_tag.get() = start };

            Ok(bytes)
        } else {
            Ok(0)
        }
    }
}

/// Lock-free binary buffer without locks or atomic operations for synchronization
pub struct Buffer<'a, const N: usize> {
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

    _marker: PhantomData<&'a ()>,
}

#[cfg(target_pointer_width = "64")]
impl<'a, const N: usize> Buffer<'a, N> {
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
    pub fn new() -> (Producer<'a, N>, Consumer<'a, N>) {
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
            _marker: PhantomData::default(),
        };

        buffer.split()
    }
}

/// Functions which are used by both, the `Producer` and the `Consumer`
impl<const N: usize> Buffer<'_, N> {
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

    /// Return if the buffer is currently full
    #[inline]
    fn is_full(&self) -> bool {
        let zero_tag = unsafe { *self.zero_tag.get() };
        let last_tag = unsafe { *self.last_tag.get() };

        // check if buffer is empty
        if zero_tag == last_tag {
            return false;
        }

        let zero_index = zero_tag >> 32;
        let zero_state = zero_tag & 0x00ff_ffff;

        let last_index = last_tag >> 32;
        let last_state = last_tag & 0x00ff_ffff;

        // check if buffer is full
        if zero_index == last_index && zero_state != last_state {
            true
        } else {
            false
        }
    }
}

// internal calls for producer / consumer
/// Internal functions which gets exposed by the `Producer` and `Consumer` respectively
impl<const N: usize> Buffer<'_, N> {
    /// Read from the buffer
    ///
    /// After checking if there is data on the buffer, it will read until there is no data left or
    /// the provided buffer is full and will return it.
    /// If there was no data to read from in the first place, it will return 0.
    ///
    /// After a successfull read, we have to update the index and the last known state from the
    /// `last_tag` to ensure consistency.
    #[inline]
    fn read(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        unimplemented!()
    }

    /// Write to the buffer
    ///
    /// If there is space left to write to, it will write the data provided to the buffer until
    /// either the buffer is full or the provided data is written to the buffer.
    /// If there is no space left to write to, it will return 0.
    ///
    /// After a successfull write, the index gets updated as well as the state gets incremented.
    /// This ensures that there is never a false positive by checking if the indicies are the same
    /// which would either mean the buffer is full or empty. If the Producer wrote data until the
    /// buffer was full, the state differs from the Consumer state - therefore the buffer must be
    /// full.
    #[inline]
    fn write(&self, buf: &[u8]) -> std::io::Result<usize> {
        unimplemented!()
    }
}

/// This functions will only be used from the `Buffer` internally
impl<'a, const N: usize> Buffer<'a, N> {
    /// Consume `Buffer` to create a `Producer` and a `Consumer`
    ///
    /// This is the only place where we are using atomics over the `Arc` implementation. This will
    /// not give us any performance penalties as this is only be used to ensure that the `Buffer`
    /// object does exist.
    fn split(self) -> (Producer<'a, N>, Consumer<'a, N>) {
        let buffer = Arc::new(self);
        let consumer = Consumer {
            buffer: buffer.clone(),
            cache: BufferHelper(&mut [], &mut []),
            index: 0,
            state: 0,
        };
        let (index, state, cache) = buffer.get_writeable_buffer().unwrap();
        let producer = Producer {
            buffer,
            cache,
            index,
            state,
        };

        (producer, consumer)
    }

    /// Returns a writeable `BufferHelper` with the empty space of the buffer
    ///
    /// To ensure that we do not overwrite any data from the part of the buffer where already data
    /// is hold, we read from the `zero_tag` first. In the worst case, this will give us a smaller
    /// available space as it might change while reading the `last_tag`. However, on the next call
    /// we do have the updated value and we can continue.
    fn get_writeable_buffer(&self) -> Option<(usize, usize, BufferHelper)> {
        // the read should be safe because its a single instruction to read a usize value
        let zero_tag = unsafe { *self.zero_tag.get() };
        // this is safe as the Producer is the only one who change this value
        let last_tag = unsafe { *self.last_tag.get() };

        let zero_index = zero_tag >> 32;
        let zero_state = zero_tag & 0x00ff_ffff;

        let last_index = last_tag >> 32;
        let last_state = last_tag & 0x00ff_ffff;

        // check if buffer is full
        if last_index == zero_index && last_state != zero_state {
            return None;
        }

        // check if free data is wrapping
        let buffer = if zero_index > last_index {
            // |***E--------S***|
            let buffer0 =
                &mut unsafe { &mut *self.buffer.get() }[last_index..zero_index] as *mut [u8];
            let buffer1 = &mut [] as *mut [u8];

            BufferHelper(buffer0, buffer1)
        } else {
            // |--S*****E-------|
            let buffer0 = &mut unsafe { &mut *self.buffer.get() }[last_index..] as *mut [u8];
            let buffer1 = &mut unsafe { &mut *self.buffer.get() }[..zero_index] as *mut [u8];

            BufferHelper(buffer0, buffer1)
        };

        if buffer.len() > 0 {
            Some((last_index, last_state, buffer))
        } else {
            None
        }
    }

    /// Returns a readable `BufferHelper` with the available data from the buffer
    ///
    /// To ensure that we do not read from the empty space of the buffer, we read from the
    /// `last_tag` first. In the worst case, this will give us a smaller consumable buffer as it
    /// might change while reading the `zero_tag`. However, on the next call we do have the updated
    /// value and we can continue.
    fn get_readable_buffer(&self) -> Option<(usize, usize, BufferHelper)> {
        // the read should be safe because its a single instruction to read a usize value
        let last_tag = unsafe { *self.last_tag.get() };
        // this is safe as the Consumer is the only one who change this value
        let zero_tag = unsafe { *self.zero_tag.get() };

        // check if buffer is empty
        if last_tag == zero_tag {
            return None;
        }

        let zero_index = zero_tag >> 32;
        let zero_state = zero_tag & 0x00ff_ffff;

        let last_index = last_tag >> 32;
        let last_state = last_tag & 0x00ff_ffff;

        // check if buffer is full
        if zero_index == last_index && zero_state != last_state {
            // |*******S********|
            let buffer0 = &mut unsafe { &mut *self.buffer.get() }[zero_index..] as *mut [u8];
            let buffer1 = &mut unsafe { &mut *self.buffer.get() }[..zero_index] as *mut [u8];

            return Some((zero_index, last_state, BufferHelper(buffer0, buffer1)));
        }

        // check if data assigned is wrapping
        let buffer = if zero_index > last_index {
            // |***E--------S***|
            let buffer0 = &mut unsafe { &mut *self.buffer.get() }[zero_index..] as *mut [u8];
            let buffer1 = &mut unsafe { &mut *self.buffer.get() }[..last_index] as *mut [u8];

            BufferHelper(buffer0, buffer1)
        } else {
            // |--S*****E-------|
            let buffer0 =
                &mut unsafe { &mut *self.buffer.get() }[zero_index..last_index] as *mut [u8];
            let buffer1 = &mut [] as *mut [u8];

            BufferHelper(buffer0, buffer1)
        };

        if buffer.len() > 0 {
            Some((zero_index, last_state, buffer))
        } else {
            None
        }
    }
}
