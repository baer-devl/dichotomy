pub use buffer::Buffer;
use marker::InvariantLifetime;
pub use marker::{ConsumerToken, ProducerToken};
use std::{
    cell::UnsafeCell,
    io::{Read, Write},
};

mod buffer;
mod marker;

/// Dual consumption ring buffer
pub struct RingBuffer<'producer, 'consumer, const N: usize> {
    // used to ensure exclusive access to the producer related variables
    _marker_producer: InvariantLifetime<'producer>,
    // used to ensure exclusive access to the consumer related variables
    _marker_consumer: InvariantLifetime<'consumer>,

    // binary buffer which can be accessed by both, the producer and consumer
    // however, they are accessing different areas and thus it is safe to operate on the same
    // buffer
    buffer: UnsafeCell<[u8; N]>,

    // marks the begin of consumable data
    // this is only changed by the consumer
    data_start: UnsafeCell<usize>,

    // marks the end of consumable data
    // this is only changed by the producer
    data_end: UnsafeCell<usize>,
}

// FIXME: this should be fine as we do not change data without the marker which gives us exclusive
// access
unsafe impl<'producer, 'consumer, const N: usize> Send for RingBuffer<'producer, 'consumer, N> {}
// FIXME: this should be fine as the marker will only allow for exclusive access
unsafe impl<'producer, 'consumer, const N: usize> Sync for RingBuffer<'producer, 'consumer, N> {}

impl<'producer, 'consumer, const N: usize> Default for RingBuffer<'producer, 'consumer, N> {
    fn default() -> Self {
        // FIXME ensure we are in a 64bit architecutre
        // 32bit arch would only support up to 64KB!
        // 64bit arch would support up to 4GB
        assert!(usize::MAX as u64 == u64::MAX);
        // `N` MUST be smaller than `u32::MAX` because we need an additional flag to indicate that
        // the buffer is full
        assert!(N < u32::MAX as usize);

        Self {
            _marker_producer: InvariantLifetime::default(),
            _marker_consumer: InvariantLifetime::default(),
            buffer: UnsafeCell::new([0u8; N]),
            data_start: UnsafeCell::default(),
            data_end: UnsafeCell::default(),
        }
    }
}

impl<'producer, 'consumer, const N: usize> RingBuffer<'producer, 'consumer, N> {
    /// Create a new `RingBuffer` instance
    ///
    /// # Example
    /// ```
    /// use dual_access_ringbuffer::{RingBuffer, ProducerToken, ConsumerToken};
    ///
    /// ProducerToken::new(|mut producer| {
    ///     ConsumerToken::new(|mut consumer| {
    ///         let buffer = RingBuffer::<1024>::new();
    ///         std::thread::scope(|scope| {
    ///             // spawn producer thread
    ///             scope.spawn(|| {
    ///                 for _ in 0..10 {
    ///                     buffer.produce(&mut producer, b"hello world").unwrap();
    ///                 }
    ///             });
    ///             // spawn consumer thread
    ///             scope.spawn(|| {
    ///                 for _ in 0..10 {
    ///                     buffer.consume(&mut consumer, |buffer| buffer.len())
    ///                 }
    ///             });
    ///         });
    ///     });
    /// });
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the capacity of the buffer
    pub fn capacity(&self) -> usize {
        N
    }

    /// Return the available capacity of the buffer which can be written to
    pub fn len(&self, _: &ConsumerToken<'consumer>) -> usize {
        // getting this value is fine as we are holding the consumer token and it cannot be changed
        // until someone gets a mutual consumer token
        let start = unsafe { *self.data_start.get() };
        // reading this value is not safe
        // however, if it would change later on it would only show a wrong length of the buffer but
        // never less as it cannot be consumed as we hold the consumer token
        let end = unsafe { *self.data_end.get() };

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

    pub fn free(&self, _: &ProducerToken<'producer>) -> usize {
        // getting the start value is not safe
        // however, the worst that could happen is that the value will increase thus giving us a
        // bigger free space later in the calculation
        // this is no problem as we would give a smaller free space back than it might be at this
        // point
        let start = unsafe { *self.data_start.get() };
        // get the end value is safe as we hold the producer token which gurantees that it will not
        // change until we leave this function
        let end = unsafe { *self.data_end.get() };

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

    pub fn write_all(
        &self,
        producer: &mut ProducerToken<'producer>,
        buf: &[u8],
    ) -> std::io::Result<()> {
        let mut written = 0;

        loop {
            match self.get_writeable_buffer(producer) {
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
                        unsafe { *self.data_end.get() = end };

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
                    while unsafe { *self.data_start.get() == data_start } {
                        std::hint::spin_loop()
                    }
                    //println!("WRITE: EXIT spin-loop");
                }
            }
        }
    }

    pub fn read_exact(
        &self,
        consumer: &mut ConsumerToken<'consumer>,
        buf: &mut [u8],
    ) -> std::io::Result<()> {
        let mut read = 0;

        loop {
            match self.get_readable_buffer(consumer) {
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
                        unsafe { *self.data_start.get() = start };

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
                    while unsafe { *self.data_end.get() == data_end } {
                        std::hint::spin_loop()
                    }
                    //println!("READ:  EXIT spin-loop");
                }
            }
        }
    }

    pub fn read<'a>(
        &self,
        _consumer: &'a ConsumerToken<'consumer>,
        _buffer: &mut [u8],
    ) -> std::io::Result<usize> {
        unimplemented!()
    }

    fn get_writeable_buffer<'a>(
        &self,
        _: &'a mut ProducerToken<'producer>,
    ) -> Result<(usize, usize, Buffer), usize> {
        // this could be changing after the read - however, the only result would be that the
        // accessable empty buffer would be smaller than it really would be
        let data_start = unsafe { *self.data_start.get() };
        // this value is controlled by the producer token exclusivly and will not change as long as
        // the returned `Buffer` object exists
        let data_end = unsafe { *self.data_end.get() };

        let index_start = data_start >> 32;
        let state_start = data_start & 0xffffff;

        let index = data_end >> 32;
        let state = data_end & 0xffffff;

        // check if buffer is full
        if index == index_start && state != state_start {
            return Err(data_start);
        }

        // check if free data is wrapping
        let buffer = if index_start > index {
            // |***E--------S***|
            let buffer0 = &mut unsafe { &mut *self.buffer.get() }[index..index_start];
            let buffer1 = &mut [];

            Buffer(buffer0, buffer1)
        } else {
            // |--S*****E-------|
            let buffer0 = &mut unsafe { &mut *self.buffer.get() }[index..];
            let buffer1 = &mut unsafe { &mut *self.buffer.get() }[..index_start];

            Buffer(buffer0, buffer1)
        };

        if buffer.len() > 0 {
            Ok((index, state, buffer))
        } else {
            Err(data_start)
        }
    }

    fn get_readable_buffer<'a>(
        &self,
        _: &'a ConsumerToken<'consumer>,
    ) -> Result<(usize, usize, usize, usize, Buffer), usize> {
        // this value could change after we read from it. however, this only will give us a smaller
        // consumable buffer to read from and would be resolved if we would call it again
        let data_end = unsafe { *self.data_end.get() };
        // this value is only changed by the consumer and as we are holding the exclusive token
        // when we are accessing it over the `consume` function, we can ensure it will not change.
        // however, if this gets called via the `read` function we also can ensure this will not be
        // changed as a mutual access would also mean that there are no read-only references to be
        // used
        let data_start = unsafe { *self.data_start.get() };

        let index = data_start >> 32;
        let state = data_start & 0xffffff;

        let index_end = data_end >> 32;
        let state_end = data_end & 0xffffff;

        // check if buffer is empty
        if index == index_end && state == state_end {
            return Err(data_end);
        }

        // check if buffer is full
        if index == index_end && state != state_end {
            // |*******S********|
            let buffer0 = &mut unsafe { &mut *self.buffer.get() }[index..];
            let buffer1 = &mut unsafe { &mut *self.buffer.get() }[..index];

            return Ok((index, state, index_end, state_end, Buffer(buffer0, buffer1)));
        }

        // check if data assigned is wrapping
        let buffer = if index > index_end {
            // |***E--------S***|
            let buffer0 = &mut unsafe { &mut *self.buffer.get() }[index..];
            let buffer1 = &mut unsafe { &mut *self.buffer.get() }[..index_end];

            Buffer(buffer0, buffer1)
        } else {
            // |--S*****E-------|
            let buffer0 = &mut unsafe { &mut *self.buffer.get() }[index..index_end];
            let buffer1 = &mut [];

            Buffer(buffer0, buffer1)
        };

        if buffer.len() > 0 {
            Ok((index, state, index_end, state_end, buffer))
        } else {
            Err(data_end)
        }
    }
}

#[cfg(test)]
mod test {
    use crate::{ConsumerToken, ProducerToken, RingBuffer};

    #[test]
    fn silly() {
        const SIZE: usize = 32;
        const DATA: &[u8] = b"hello world";
        const ITERATIONS: usize = 100_000_000;

        ProducerToken::new(|mut producer| {
            ConsumerToken::new(|mut consumer| {
                let buffer = RingBuffer::<SIZE>::new();
                std::thread::scope(|scope| {
                    scope.spawn(|| {
                        let mut buf = [0u8; DATA.len()];
                        for _ in 0..ITERATIONS {
                            buffer.read_exact(&mut consumer, &mut buf).unwrap();
                            assert!(buf == DATA);
                        }
                    });
                    scope.spawn(|| {
                        for _ in 0..ITERATIONS {
                            buffer.write_all(&mut producer, DATA).unwrap()
                        }
                    });
                });
            });
        });
    }
}
