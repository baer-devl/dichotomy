pub use buffer::BufferHelper;
use std::{
    cell::UnsafeCell,
    io::{Read, Write},
};

mod buffer;

// if producer is dropped, consumer still can read all data from the buffer - this would not work
// the other way around
pub struct Producer<'a, const N: usize> {
    buffer: &'a Buffer<N>,
}

unsafe impl<'a, const N: usize> Send for Producer<'a, N> {}
// FIXME: this should be fine as the marker will only allow for exclusive access
unsafe impl<'a, const N: usize> Sync for Producer<'a, N> {}

impl<'a, const N: usize> Producer<'a, N> {
    pub fn write_all(&self, buffer: &[u8]) -> std::io::Result<()> {
        self.buffer.write_all(buffer)
    }
}

pub struct Consumer<'a, const N: usize> {
    buffer: &'a Buffer<N>,
}

unsafe impl<'a, const N: usize> Send for Consumer<'a, N> {}
// FIXME: this should be fine as the marker will only allow for exclusive access
unsafe impl<'a, const N: usize> Sync for Consumer<'a, N> {}

impl<'a, const N: usize> Consumer<'a, N> {
    pub fn read_exact(&self, buffer: &mut [u8]) -> std::io::Result<()> {
        self.buffer.read_exact(buffer)
    }
}

pub struct Buffer<const N: usize> {
    // binary buffer which can be accessed by both, the producer and consumer
    // however, they are accessing different areas and thus it is safe to operate on the same
    // buffer
    buffer: UnsafeCell<[u8; N]>,

    // marks the begin of consumable data
    // this is only changed by the consumer
    zero_tag: UnsafeCell<usize>,

    // marks the end of consumable data
    // this is only changed by the producer
    last_tag: UnsafeCell<usize>,
}

// FIXME: this should be fine as we do not change data without the marker which gives us exclusive
// access
unsafe impl<const N: usize> Send for Buffer<N> {}
// FIXME: this should be fine as the marker will only allow for exclusive access
unsafe impl<const N: usize> Sync for Buffer<N> {}

impl<const N: usize> Default for Buffer<N> {
    fn default() -> Self {
        // FIXME ensure we are in a 64bit architecutre
        // 32bit arch would only support up to 64KB!
        // 64bit arch would support up to 4GB
        assert!(usize::MAX as u64 == u64::MAX);
        // `N` MUST be smaller than `u32::MAX` because we need an additional flag to indicate that
        // the buffer is full
        assert!(N < u32::MAX as usize);

        Self {
            buffer: UnsafeCell::new([0u8; N]),
            zero_tag: UnsafeCell::default(),
            last_tag: UnsafeCell::default(),
        }
    }
}

impl<'x, const N: usize> Buffer<N> {
    /// Create a new `RingBuffer` instance
    ///
    /// # Example
    /// ```
    /// use dichotomy::RingBuffer;
    ///
    /// const DATA: &[u8] = b"hello world";
    ///
    /// let buffer = RingBuffer::<1024>::new();
    /// std::thread::scope(|scope| {
    ///     // spawn producer thread
    ///     scope.spawn(|| {
    ///         for _ in 0..10 {
    ///             buffer.write_all(DATA).unwrap();
    ///         }
    ///     });
    ///     // spawn consumer thread
    ///     scope.spawn(|| {
    ///         let mut buf = [0u8; 32];
    ///         for _ in 0..10 {
    ///             buffer.read_exact(&mut buf[..DATA.len()]).unwrap();
    ///         }
    ///     });
    /// });
    /// ```
    pub fn new_not() -> Self {
        Self::default()
    }

    fn get_producer_consumer<'a>(&'a self) -> (Producer<'a, N>, Consumer<'a, N>) {
        (Producer { buffer: &self }, Consumer { buffer: &self })
    }

    pub fn into_producer_consumer<F, R>(self, fun: F) -> R
    where
        for<'token> F: FnOnce(Producer<'token, N>, Consumer<'token, N>) -> R,
    {
        let (producer, consumer) = self.get_producer_consumer();
        fun(producer, consumer)
    }

    pub fn new<F, R>(fun: F) -> R
    where
        for<'token> F: FnOnce(Producer<'token, N>, Consumer<'token, N>) -> R,
    {
        let buffer = Buffer::default();
        let (producer, consumer) = buffer.get_producer_consumer();
        fun(producer, consumer)
    }

    /// Return the capacity of the buffer
    pub fn capacity(&self) -> usize {
        N
    }

    /// Return the available capacity of the buffer which can be written to
    pub fn len(&self) -> usize {
        // getting this value is fine as we are holding the consumer token and it cannot be changed
        // until someone gets a mutual consumer token
        let start = unsafe { *self.zero_tag.get() };
        // reading this value is not safe
        // however, if it would change later on it would only show a wrong length of the buffer but
        // never less as it cannot be consumed as we hold the consumer token
        let end = unsafe { *self.last_tag.get() };

        // check if buffer is full
        if end > N {
            return N;
        }

        // check if data is wrapping
        if start <= end {
            // |---S*****E-------|
            end - start
        } else {
            // |*E----------S****|
            N - start + end
        }
    }

    pub fn free(&self) -> usize {
        // getting the start value is not safe
        // however, the worst that could happen is that the value will increase thus giving us a
        // bigger free space later in the calculation
        // this is no problem as we would give a smaller free space back than it might be at this
        // point
        let start = unsafe { *self.zero_tag.get() };
        // get the end value is safe as we hold the producer token which gurantees that it will not
        // change until we leave this function
        let end = unsafe { *self.last_tag.get() };

        // check if buffer is full
        if end > N {
            return 0;
        }

        // check if data is wrapping
        if start <= end {
            // |---S*****E-------|
            N - start + end
        } else {
            // |*E----------S****|
            start - end
        }
    }

    fn write_all(&self, buf: &[u8]) -> std::io::Result<()> {
        let mut written = 0;

        loop {
            match self.get_writeable_buffer() {
                Ok((index, state, mut buffer)) => {
                    // write from buffer
                    let bytes = buffer.write(&buf[written..])?;
                    /*println!(
                        "WRITE: {index}->{} [{state}]  buffer: {}",
                        (index + bytes) % N,
                        buffer.len()
                    );*/

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
                    //println!("WRITE: ENTER spin-loop");
                    while unsafe { *self.zero_tag.get() == data_start } {
                        std::hint::spin_loop()
                    }
                    //println!("WRITE: EXIT spin-loop");
                }
            }
        }
    }

    fn read_exact(&self, buf: &mut [u8]) -> std::io::Result<()> {
        let mut read = 0;

        loop {
            match self.get_readable_buffer() {
                Ok((index, _state, _index_end, state_end, mut buffer)) => {
                    // read to buffer
                    let bytes = buffer.read(&mut buf[read..])?;
                    /*println!(
                        "READ:  {index}->{} [{state}]  buffer: {}",
                        (index + bytes) % N,
                        buffer.len()
                    );*/

                    // ensure we read something
                    if bytes > 0 {
                        // set new value
                        let state = state_end;
                        let index = (index + bytes) % N;
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
                Err(data_end) => {
                    // if producer wrote new data we can read again
                    //println!("READ:  ENTER spin-loop");
                    while unsafe { *self.last_tag.get() == data_end } {
                        std::hint::spin_loop()
                    }
                    //println!("READ:  EXIT spin-loop");
                }
            }
        }
    }

    pub fn read<'a>(&self, _buffer: &mut [u8]) -> std::io::Result<usize> {
        unimplemented!()
    }

    fn get_writeable_buffer<'a>(&self) -> Result<(usize, usize, BufferHelper), usize> {
        // this could be changing after the read - however, the only result would be that the
        // accessable empty buffer would be smaller than it really would be
        let zero_tag = unsafe { *self.zero_tag.get() };
        // this value is controlled by the producer token exclusivly and will not change as long as
        // the returned `Buffer` object exists
        let last_tag = unsafe { *self.last_tag.get() };

        let zero_index = zero_tag >> 32;
        let zero_state = zero_tag & 0xffffff;

        let last_index = last_tag >> 32;
        let last_state = last_tag & 0xffffff;

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

    fn get_readable_buffer<'a>(&self) -> Result<(usize, usize, usize, usize, BufferHelper), usize> {
        // this value could change after we read from it. however, this only will give us a smaller
        // consumable buffer to read from and would be resolved if we would call it again
        let last_tag = unsafe { *self.last_tag.get() };
        // this value is only changed by the consumer and as we are holding the exclusive token
        // when we are accessing it over the `consume` function, we can ensure it will not change.
        // however, if this gets called via the `read` function we also can ensure this will not be
        // changed as a mutual access would also mean that there are no read-only references to be
        // used
        let zero_tag = unsafe { *self.zero_tag.get() };

        let zero_index = zero_tag >> 32;
        let zero_state = zero_tag & 0xffffff;

        let last_index = last_tag >> 32;
        let last_state = last_tag & 0xffffff;

        // check if buffer is empty
        if zero_index == last_index && zero_state == last_state {
            return Err(last_tag);
        }

        // check if buffer is full
        if zero_index == last_index && zero_state != last_state {
            // |*******S********|
            let buffer0 = &mut unsafe { &mut *self.buffer.get() }[zero_index..];
            let buffer1 = &mut unsafe { &mut *self.buffer.get() }[..zero_index];

            return Ok((
                zero_index,
                zero_state,
                last_index,
                last_state,
                BufferHelper(buffer0, buffer1),
            ));
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
            Ok((zero_index, zero_state, last_index, last_state, buffer))
        } else {
            Err(last_tag)
        }
    }
}

#[cfg(test)]
mod test {
    use crate::Buffer;

    #[test]
    fn silly() {
        const SIZE: usize = 32;
        const DATA: &[u8] = b"hello world";
        const ITERATIONS: usize = 100_000_000;

        Buffer::<SIZE>::new(|producer, consumer| {
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    for _ in 0..ITERATIONS {
                        producer.write_all(DATA).unwrap()
                    }
                });
                scope.spawn(|| {
                    let mut buf = [0u8; DATA.len()];
                    for _ in 0..ITERATIONS {
                        consumer.read_exact(&mut buf).unwrap();
                        assert!(buf == DATA);
                    }
                });
            });
        });
    }

    #[test]
    fn intended() {
        const SIZE: usize = 8;
        const DATA: &[u8] = b"hello world";
        const ITERATIONS: usize = 10_000_000;

        Buffer::<SIZE>::new(|producer, consumer| {
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    for _ in 0..ITERATIONS {
                        producer.write_all(DATA).unwrap()
                    }
                });
                scope.spawn(|| {
                    let mut buf = [0u8; DATA.len()];
                    for _ in 0..ITERATIONS {
                        consumer.read_exact(&mut buf).unwrap();
                        assert!(buf == DATA);
                    }
                });
            });
        });
    }
}
