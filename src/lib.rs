use helper::BufferHelper;
use std::{
    cell::UnsafeCell,
    io::{Read, Write},
    sync::Arc,
};

mod helper;

#[cfg(test)]
mod tests;

/// Writable part of the buffer
///
/// # Safety
/// It is safe to move this instance into other threads as the `Arc` ensures that the underlaying
/// buffer does exist. However, if the `Consumer` part is dropped and the buffer is full, it will
/// not be possible to write more data to the buffer.
pub struct Producer<const N: usize> {
    buffer: Arc<Buffer<N>>,
}

/// This is safe because we are using `Arc` to share the underlaying buffer
unsafe impl<const N: usize> Send for Producer<N> {}
/// This is safe because we are using `Arc` to share the underlaying buffer
unsafe impl<const N: usize> Sync for Producer<N> {}

impl<const N: usize> Producer<N> {
    /// Return the current length of the buffer
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Return if the buffer is currently full
    pub fn is_full(&self) -> bool {
        !self.buffer.is_empty()
    }

    /// Write until the whole data is written to the buffer
    ///
    /// # Safety
    /// Although this is working as intended, keep in mind that this will use a spin-loop until the
    /// data is written. If you cannot gurantee that the `Consumer` part will consume the buffer in
    /// reasonable time, you should _not_ use this function!
    /// Instead, use `write` and handle the blocking part yourself.
    #[cfg(feature = "blocking")]
    pub fn write_blocking(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.buffer.write_all(data)
    }

    /// Return if `Consumer` part still exists
    pub fn does_consumer_still_exist(&self) -> bool {
        Arc::weak_count(&self.buffer) > 1
    }
}

/// Implement `Write` to make it easy to write to the buffer
impl<const N: usize> Write for Producer<N> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buffer.write(buf)
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
pub struct Consumer<const N: usize> {
    buffer: Arc<Buffer<N>>,
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

    /// Read until the provided buffer is full
    ///
    /// # Safety
    /// Although this is working as intended, keep in mind that this will use a spin-loop until the
    /// data is read. If you cannot gurantee that the `Producer` part will write to the buffer in
    /// reasonable time, you should _not_ use this function!
    /// Instead, use `read` and handle the blocking part yourself.
    #[cfg(feature = "blocking")]
    pub fn read_blocking(&mut self, buffer: &mut [u8]) -> std::io::Result<()> {
        self.buffer.read_exact(buffer)
    }

    /// Return if `Producer` part still exists
    /// FIXME: should we internal check if the buffer parts still exists?
    pub fn does_producer_still_exist(&self) -> bool {
        Arc::weak_count(&self.buffer) > 1
    }
}

/// Implement `Read` to make it easy to read from the buffer
impl<const N: usize> Read for Consumer<N> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.buffer.read(buf)
    }
}

/// Dichotomy is a lock-free binary ring buffer
///
/// The buffer itself does not use any locks like `Mutex` or `RwLock`, nor make use of atomics. The
/// two parts, `Producer` and `Consumer` are guranteed to have mutual exclusive access to the part
/// of the buffer they need access to. Thus makes the buffer very performant as it does not need to
/// handle locks.
///
/// There are two tags used to handle the internal access to the buffer called `zero_tag` and
/// `last_tag`. Both will hold an index and a state. The index will give the position of the start
/// and the end of written data. The state ensures that the `Consumer` part does have the
/// information if there is more to read from. The `zero_tag` can only be written by the `Consumer`
/// and the `last_tag` can only be written by the `Producer`. This and the fact that the processort
/// _should_ be able to read an usize in a single instruction, makes it safe to access those values
/// without locks.
///
/// # Deep dive
/// Lets first focus on the index part of the tags.
/// The `Producer` holds its index in the `last_tag` which marks the beginning of the free buffer.
/// If there is space left in the buffer, it will write into it and update the index.
/// The very same is true for the `Consumer`, who holds its index in the `zero_tag`, indicating the
/// beginning of readable data. If the data gets consumed, it will update the index and free space
/// on the buffer.
/// If both indicies are the same, this could either mean that the buffer is empty of full. To make
/// a distinction of those two possibilities, we are using the state part of the tags which is an
/// incrementing number changed by each write to the buffer.
/// If the `Consumer` check if the buffer has data to read from, it will check if the tags will
/// differ. If that is the case, the index of both values will be used to calculate the readable
/// size of the data. Each time the `Consumer` reads data from the buffer, it will update its state
/// to the last known state of the `Producer` tag. This ensures that there will never be a false
/// empty buffer because the state will always be different after the `Producer` wrote new data.
/// Here are some examples of possible states of the two tags:
///
/// ## Empty buffer
/// |--------X------------|
/// 'X' representing both indicies of the tags which pointing to the same position. In this special
/// case, the state of both tags have to be the same. Thus makes it easy to check, if both tags are
/// the same, the buffer is empty.
///
/// ## Filled buffer
/// |----Z**********L-----|
/// 'Z' representing the index of `zero_tag` and 'L' representing the index of `last_tag`. As both
/// indicies are different, it is safe to assume that there is data to read from and free space to
/// write to. Both, the free space and the filled spaces can easily calculated.
///
/// ## Full buffer
/// |*****************X***|
/// 'X' representing both indicies of the tags pointing to the same position. However, this time
/// the state of the tags differ which indicates that the buffer must be full.
///
/// # Safety
/// The whole safety of this implementation relies on the fact that the processor can read an usize
/// value with a single instruction. However, there are processor implementations which do not work
/// like this. Given that they are rare we still recommend to double check if your processor can
/// read a single usize in one instruction.
/// As we are using the usize value as two different parts, the maximum amount of elements in this
/// buffer is limited by the architecture it runs on. For a 64 bit architecture it will be 32 bits.
/// On a 32 bit architecture this will be 16 bits. We handle those two variants in our
/// implementation.
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
    /// For a 64 bit architecture the maximum size will be `u32::MAX` and for a 32 bit architecture
    /// it will be `u16::MAX`.
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
    ///         consumer.read_blocking(&mut buf[..DATA.len()]).unwrap();
    ///     }
    /// });
    ///
    /// for _ in 0..10 {
    ///     producer.write_blocking(DATA).unwrap();
    /// }
    /// ```
    #[cfg(target_arch = "x86_64")]
    pub fn new() -> (Producer<N>, Consumer<N>) {
        // ensure that the size does not exceed the maximum allowed size for a 64 bit system
        assert!(N <= u32::MAX as usize);

        let buffer = Self {
            buffer: UnsafeCell::new([0u8; N]),
            zero_tag: UnsafeCell::default(),
            last_tag: UnsafeCell::default(),
        };

        buffer.split()
    }

    #[cfg(target_arch = "x86")]
    pub fn new() -> (Producer<N>, Consumer<N>) {
        // ensure that the size does not exceed the maximum allowed size for a 32 bit system
        assert!(N <= u16::MAX as usize);

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
}

// internal calls for producer / consumer
/// Internal functions which gets exposed by the `Producer` and `Consumer` respectively
impl<const N: usize> Buffer<N> {
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
        match self.get_readable_buffer() {
            Ok((index, state_end, mut buffer)) => {
                // read to buffer
                let bytes = buffer.read(buf)?;

                // ensure we read something
                if bytes > 0 {
                    // set new value
                    let state = state_end;
                    let index = (index + bytes) % N;
                    let start = index << 32 | state;

                    // new value
                    unsafe { *self.zero_tag.get() = start };

                    Ok(bytes)
                } else {
                    Ok(0)
                }
            }
            Err(_last_tag) => Ok(0),
        }
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
        match self.get_writeable_buffer() {
            Ok((index, state, mut buffer)) => {
                // write from buffer
                let bytes = buffer.write(buf)?;

                // ensure we wrote something
                if bytes > 0 {
                    // update end value
                    let state = state + 1;
                    let index = (index + bytes) % N;
                    let end = index << 32 | state;

                    // set new value
                    unsafe { *self.last_tag.get() = end };

                    // return if we wrote all bytes
                    Ok(bytes)
                } else {
                    Ok(0)
                }
            }
            Err(_last_tag) => Ok(0),
        }
    }
}

/// Internal Read/Write implementations used by `Producer` and `Consumer` respectively
impl<const N: usize> Buffer<N> {
    /// Write to the buffer until the provided data is on the buffer
    ///
    /// This function will block until the provided data will be written to the buffer.
    ///
    /// # Safety
    /// If the `Consumer` gets dropped while we are still trying to write to the buffer, this function
    /// will _never_ return. Use with caution!
    ///
    /// # Performance
    /// Internally, this will use a spin loop to block which should only be used if you can ensure that
    /// the `Consumer` will free the buffer in reasonable time.
    #[cfg(feature = "blocking")]
    #[inline]
    fn write_all(&self, buf: &[u8]) -> std::io::Result<()> {
        let mut written = 0;

        loop {
            match self.get_writeable_buffer() {
                Ok((index, state, mut buffer)) => {
                    // write from buffer
                    let bytes = buffer.write(&buf[written..])?;

                    // ensure we wrote something
                    if bytes > 0 {
                        // update end value
                        let state = state + 1;
                        let index = (index + bytes) % N;
                        let end = index << 32 | state;

                        // set new value
                        unsafe { *self.last_tag.get() = end };

                        // return if we wrote all bytes
                        written += bytes;
                        if written == buf.len() {
                            return Ok(());
                        }
                    }
                }
                Err(data_start) => {
                    // if consumer change the index we can write again
                    while unsafe { *self.zero_tag.get() == data_start } {
                        std::hint::spin_loop();
                    }
                }
            }
        }
    }

    /// Read from the buffer until the provided buffer is full
    ///
    /// This function will block until the provided buffer is full.
    ///
    /// # Safety
    /// If the `Producer` gets dropped while we are still trying to read from the buffer, this
    /// function will _never_ return. Use with caution!
    ///
    /// # Performance
    /// Internally, this will use a spin loop to block which should only be used if you can ensure
    /// that the `Producer` will write onto the buffer in reasonable time.
    #[cfg(feature = "blocking")]
    #[inline]
    fn read_exact(&self, buf: &mut [u8]) -> std::io::Result<()> {
        let mut read = 0;

        loop {
            match self.get_readable_buffer() {
                Ok((zero_index, last_state, mut buffer)) => {
                    // read to buffer
                    let bytes = buffer.read(&mut buf[read..])?;

                    // ensure we read something
                    if bytes > 0 {
                        // set new value
                        let state = last_state;
                        let index = (zero_index + bytes) % N;
                        let start = index << 32 | state;

                        // new value
                        unsafe { *self.zero_tag.get() = start };

                        // return if we read all bytes from the buffer
                        read += bytes;
                        if read == buf.len() {
                            return Ok(());
                        }
                    }
                }
                Err(last_tag) => {
                    // if producer wrote new data we can read again
                    while unsafe { *self.last_tag.get() == last_tag } {
                        std::hint::spin_loop();
                    }
                }
            }
        }
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
        };
        let producer = Producer { buffer };

        (producer, consumer)
    }

    /// Returns a writeable `BufferHelper` with the empty space of the buffer
    ///
    /// To ensure that we do not overwrite any data from the part of the buffer where already data
    /// is hold, we read from the `zero_tag` first. In the worst case, this will give us a smaller
    /// available space as it might change while reading the `last_tag`. However, on the next call
    /// we do have the updated value and we can continue.
    fn get_writeable_buffer(&self) -> Result<(usize, usize, BufferHelper), usize> {
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
            return Err(zero_tag);
        }

        // check if free data is wrapping
        let buffer = if zero_index > last_index {
            // |***E--------S***|
            let buffer0 = &mut unsafe { &mut *self.buffer.get() }[last_index..zero_index];
            let buffer1 = &mut [];

            BufferHelper(buffer0, buffer1)
        } else {
            // |--S*****E-------|
            let buffer0 = &mut unsafe { &mut *self.buffer.get() }[last_index..];
            let buffer1 = &mut unsafe { &mut *self.buffer.get() }[..zero_index];

            BufferHelper(buffer0, buffer1)
        };

        if buffer.len() > 0 {
            Ok((last_index, last_state, buffer))
        } else {
            Err(zero_tag)
        }
    }

    /// Returns a readable `BufferHelper` with the available data from the buffer
    ///
    /// To ensure that we do not read from the empty space of the buffer, we read from the
    /// `last_tag` first. In the worst case, this will give us a smaller consumable buffer as it
    /// might change while reading the `zero_tag`. However, on the next call we do have the updated
    /// value and we can continue.
    fn get_readable_buffer(&self) -> Result<(usize, usize, BufferHelper), usize> {
        // the read should be safe because its a single instruction to read a usize value
        let last_tag = unsafe { *self.last_tag.get() };
        // this is safe as the Consumer is the only one who change this value
        let zero_tag = unsafe { *self.zero_tag.get() };

        // check if buffer is empty
        if last_tag == zero_tag {
            return Err(last_tag);
        }

        let zero_index = zero_tag >> 32;
        let zero_state = zero_tag & 0x00ff_ffff;

        let last_index = last_tag >> 32;
        let last_state = last_tag & 0x00ff_ffff;

        // check if buffer is full
        if zero_index == last_index && zero_state != last_state {
            // |*******S********|
            let buffer0 = &mut unsafe { &mut *self.buffer.get() }[zero_index..];
            let buffer1 = &mut unsafe { &mut *self.buffer.get() }[..zero_index];

            return Ok((zero_index, last_state, BufferHelper(buffer0, buffer1)));
        }

        // check if data assigned is wrapping
        let buffer = if zero_index > last_index {
            // |***E--------S***|
            let buffer0 = &mut unsafe { &mut *self.buffer.get() }[zero_index..];
            let buffer1 = &mut unsafe { &mut *self.buffer.get() }[..last_index];

            BufferHelper(buffer0, buffer1)
        } else {
            // |--S*****E-------|
            let buffer0 = &mut unsafe { &mut *self.buffer.get() }[zero_index..last_index];
            let buffer1 = &mut [];

            BufferHelper(buffer0, buffer1)
        };

        if buffer.len() > 0 {
            Ok((zero_index, last_state, buffer))
        } else {
            Err(last_tag)
        }
    }
}
