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
            // set the lower bits to 1
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
            match self.get_empty_buffer2(producer) {
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
                Err((index_start, state_start)) => {
                    // if consumer change the index we can write again
                    //println!("WRITE: ENTER spin-loop");
                    while unsafe {
                        *self.data_start.get() >> 32 == index_start
                            && *self.data_start.get() << 32 >> 32 == state_start
                    } {
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
            match self.get_consumable_buffer2(consumer) {
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
                Err((index_end, state_end)) => {
                    // if producer wrote new data we can read again
                    //println!("READ:  ENTER spin-loop");
                    while unsafe {
                        *self.data_end.get() >> 32 == index_end
                            && *self.data_end.get() << 32 >> 32 == state_end
                    } {
                        std::hint::spin_loop()
                    }
                    //println!("READ:  EXIT spin-loop");
                }
            }
        }
    }

    /// Read to the given buffer and fill the free buffer
    ///
    /// # Safety
    /// The `ProducerToken` will ensure exclusive access to this function.
    /// We only accessing the free space of the buffer, so the consumer part still can access the
    /// other part.
    /// We also only change `data_end` here, nowhere else so this should be fine as well.
    ///
    /// If there is not enough space it will fail.
    pub fn produce(
        &self,
        producer: &mut ProducerToken<'producer>,
        buf: &[u8],
    ) -> std::io::Result<usize> {
        // get access to empty buffer and write to it
        let end = unsafe { *self.data_end.get() };
        let mut buffer = self.get_empty_buffer(producer)?;
        let buffer_len = buffer.len();

        // check if we can write whole bytes into our buffer
        // FIXME: this should not be handled by the buffer -> maybe another function which handles
        // this
        if buf.len() > buffer_len {
            return Err(std::io::Error::new(
                std::io::ErrorKind::OutOfMemory,
                "there is not enough space left",
            ));
        }

        let bytes = buffer.write(buf)?;

        // check if buffer is full
        if buffer_len == bytes {
            // this make it clear that the buffer is full
            unsafe { *self.data_end.get() = N + 1 };
        } else {
            unsafe { *self.data_end.get() = (end + bytes) % N };
        }

        Ok(bytes)
    }

    /// Read the data from the buffer and allow to remove it if the caller returns the consumed
    /// bytes
    ///
    /// # Safety
    /// The `ConsumerToken` ensures the exclusive acces to this function.
    /// We only accessing the filles space, so the producer part is not accessed at the same time.
    /// We also only change `data_start` here, nowhere else.
    ///
    /// # Panics
    /// If the `consumed` bytes is more than the available bytes, this will panic
    pub fn consume<'a, F>(&self, consumer: &'a mut ConsumerToken<'consumer>, fun: F)
    where
        F: FnOnce(Buffer) -> usize,
    {
        let start = unsafe { *self.data_start.get() };
        // this is fine because if we are using this value it is guranteed that the buffer is full
        // and thus this value will not change
        let end = unsafe { *self.data_end.get() };

        let buffer = self.get_consumable_buffer(consumer);
        let buffer_len = buffer.len();
        let consumed = fun(buffer);

        // update start of buffer after validity check
        assert!(consumed <= buffer_len);

        // check if something was consumed
        if consumed > 0 {
            // check if buffer was full
            if end > N {
                // |S*****************|
                // |*****************S|
                // # Safety
                // the consumer token will not gurantee us exclusive access to the `data_end`
                // value. however, this is safe because we only can change the value if the buffer
                // is full and in this case this value cannot be changed by anybody else!
                // thus its safe to change it
                unsafe { *self.data_end.get() = start };
            }
            unsafe { *self.data_start.get() = (start + consumed) % N };
        }
    }

    pub fn read<'a, F>(&self, consumer: &'a ConsumerToken<'consumer>, fun: F)
    where
        F: FnOnce(Buffer),
    {
        let buffer = self.get_consumable_buffer(consumer);
        fun(buffer);
    }

    fn get_empty_buffer2<'a>(
        &self,
        _: &'a mut ProducerToken<'producer>,
    ) -> Result<(usize, usize, Buffer), (usize, usize)> {
        // this could be changing after the read - however, the only result would be that the
        // accessable empty buffer would be smaller than it really would be
        let data_start = unsafe { *self.data_start.get() };
        // this value is controlled by the producer token exclusivly and will not change as long as
        // the returned `Buffer` object exists
        let data_end = unsafe { *self.data_end.get() };

        let index_start = data_start >> 32;
        let state_start = data_start << 32 >> 32;

        let index = data_end >> 32;
        let state = data_end << 32 >> 32;

        // check if buffer is full
        if index == index_start && state != state_start {
            return Err((index_start, state_start));
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
            Err((index_start, state_start))
        }
    }

    fn get_consumable_buffer2<'a>(
        &self,
        _: &'a ConsumerToken<'consumer>,
    ) -> Result<(usize, usize, usize, usize, Buffer), (usize, usize)> {
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
        let state = data_start << 32 >> 32;

        let index_end = data_end >> 32;
        let state_end = data_end << 32 >> 32;

        // check if buffer is empty
        if index == index_end && state == state_end {
            return Err((index_end, state_end));
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
            Err((index_end, state_end))
        }
    }

    /// Return all free buffer as writable
    ///
    /// # Safety
    /// We holding the exclusive producer token so we can ensure that we are the only one accessing
    /// the empty data.
    fn get_empty_buffer<'a>(&self, _: &'a mut ProducerToken<'producer>) -> std::io::Result<Buffer> {
        // this could be changing after the read - however, the only result would be that the
        // accessable empty buffer would be smaller than it really would be
        let start = unsafe { *self.data_start.get() };
        // this value is controlled by the producer token exclusivly and will not change as long as
        // the returned `Buffer` object exists
        let end = unsafe { *self.data_end.get() };

        // S=E -> empty
        // E>N -> full
        // check if we are full
        if end > N {
            // do not update buffer
            // FIXME: maybe just return empty buffer?
            return Err(std::io::Error::new(
                std::io::ErrorKind::OutOfMemory,
                "Buffer is full",
            ));
        }

        // check if free data is wrapping
        if start > end {
            // |***E--------S***|
            let buffer0 = &mut unsafe { &mut *self.buffer.get() }[end..start];
            let buffer1 = &mut [];

            Ok(Buffer(buffer0, buffer1))
        } else {
            // |--S*****E-------|
            let buffer0 = &mut unsafe { &mut *self.buffer.get() }[end..];
            let buffer1 = &mut unsafe { &mut *self.buffer.get() }[..start];

            Ok(Buffer(buffer0, buffer1))
        }
    }

    /// Return all assigned buffer as writable
    ///
    /// # Safety
    /// We relaxed the token to be `read` as we ensure to have exclusive token if we consume from
    /// it anyways
    fn get_consumable_buffer<'a>(&self, _: &'a ConsumerToken<'consumer>) -> Buffer {
        // this value is only changed by the consumer and as we are holding the exclusive token
        // when we are accessing it over the `consume` function, we can ensure it will not change.
        // however, if this gets called via the `read` function we also can ensure this will not be
        // changed as a mutual access would also mean that there are no read-only references to be
        // used
        let start = unsafe { *self.data_start.get() };
        // this value could change after we read from it. however, this only will give us a smaller
        // consumable buffer to read from and would be resolved if we would call it again
        let end = unsafe { *self.data_end.get() };

        // check if buffer is full
        if end > N {
            // |*******S********|
            let buffer0 = &mut unsafe { &mut *self.buffer.get() }[start..];
            let buffer1 = &mut unsafe { &mut *self.buffer.get() }[..start];

            return Buffer(buffer0, buffer1);
        }

        // check if data assigned is wrapping
        if start > end {
            // |***E--------S***|
            let buffer0 = &mut unsafe { &mut *self.buffer.get() }[start..];
            let buffer1 = &mut unsafe { &mut *self.buffer.get() }[..end];

            Buffer(buffer0, buffer1)
        } else {
            // |--S*****E-------|
            let buffer0 = &mut unsafe { &mut *self.buffer.get() }[start..end];
            let buffer1 = &mut [];

            Buffer(buffer0, buffer1)
        }
    }
}

#[cfg(test)]
mod test {
    use crate::{ConsumerToken, ProducerToken, RingBuffer};
    use std::{io::Read, process::exit, sync::Mutex};

    #[test]
    fn multi_read_only_access() {
        ProducerToken::new(|mut producer| {
            ConsumerToken::new(|consumer| {
                let buffer = RingBuffer::<1024>::new();

                // fill buffer with some data
                buffer.produce(&mut producer, b"hello world").unwrap();

                // spawn a new consumer thread to read only
                std::thread::scope(|scope| {
                    scope.spawn(|| {
                        for n in 0..10 {
                            buffer.read(&consumer, |mut buffer| {
                                let mut b = [0u8; 512];
                                let l = buffer.read(&mut b).unwrap();

                                println!("r0-{}: {}", n, String::from_utf8_lossy(&b[..l]));
                            })
                        }
                    });
                    scope.spawn(|| {
                        for n in 0..10 {
                            buffer.read(&consumer, |mut buffer| {
                                let mut b = [0u8; 512];
                                let l = buffer.read(&mut b).unwrap();

                                println!("r1-{}: {}", n, String::from_utf8_lossy(&b[..l]));
                            })
                        }
                    });
                    scope.spawn(|| {
                        for n in 0..10 {
                            buffer.read(&consumer, |mut buffer| {
                                let mut b = [0u8; 512];
                                let l = buffer.read(&mut b).unwrap();

                                println!("r2-{}: {}", n, String::from_utf8_lossy(&b[..l]));
                            })
                        }
                    });
                });
            });
        });
    }

    #[test]
    fn multi_write_access() {
        ProducerToken::new(|producer| {
            ConsumerToken::new(|consumer| {
                let producer = Mutex::new(producer);
                let consumer = Mutex::new(consumer);

                let buffer = RingBuffer::<1024>::new();
                // spawn a new consumer thread
                std::thread::scope(|scope| {
                    scope.spawn(|| {
                        for n in 0..10 {
                            buffer
                                .produce(&mut producer.lock().unwrap(), b"hello world")
                                .unwrap();
                            println!("p0-{}: wrote", n);
                        }
                    });
                    scope.spawn(|| {
                        for n in 0..10 {
                            buffer
                                .produce(&mut producer.lock().unwrap(), b"hello world")
                                .unwrap();
                            println!("p1-{}: wrote", n);
                        }
                    });
                    scope.spawn(|| {
                        for n in 0..10 {
                            buffer
                                .produce(&mut producer.lock().unwrap(), b"hello world")
                                .unwrap();
                            println!("p3-{}: wrote", n);
                        }
                    });

                    scope.spawn(|| {
                        for n in 0..10 {
                            buffer.consume(&mut consumer.lock().unwrap(), |mut buffer| {
                                let mut b = [0u8; 512];
                                let l = buffer.read(&mut b).unwrap();

                                println!("c0-{}: {}", n, String::from_utf8_lossy(&b[..l]));
                                l
                            })
                        }
                    });
                    scope.spawn(|| {
                        for n in 0..10 {
                            buffer.consume(&mut consumer.lock().unwrap(), |mut buffer| {
                                let mut b = [0u8; 512];
                                let l = buffer.read(&mut b).unwrap();

                                println!("c1-{}: {}", n, String::from_utf8_lossy(&b[..l]));
                                l
                            })
                        }
                    });
                    scope.spawn(|| {
                        for n in 0..10 {
                            buffer.consume(&mut consumer.lock().unwrap(), |mut buffer| {
                                let mut b = [0u8; 512];
                                let l = buffer.read(&mut b).unwrap();

                                println!("c2-{}: {}", n, String::from_utf8_lossy(&b[..l]));
                                l
                            })
                        }
                    });
                    scope.spawn(|| {
                        for n in 0..10 {
                            buffer.consume(&mut consumer.lock().unwrap(), |mut buffer| {
                                let mut b = [0u8; 512];
                                let l = buffer.read(&mut b).unwrap();

                                println!("c3-{}: {}", n, String::from_utf8_lossy(&b[..l]));
                                l
                            })
                        }
                    });
                });
            });
        });
    }

    #[test]
    fn dual_access() {
        ProducerToken::new(|producer| {
            let producer = Mutex::new(producer);
            ConsumerToken::new(|consumer| {
                let consumer = Mutex::new(consumer);

                let buffer = RingBuffer::<1024>::new();
                // spawn a new consumer thread
                std::thread::scope(|scope| {
                    scope.spawn(|| {
                        for _ in 0..10 {
                            buffer.consume(&mut consumer.lock().unwrap(), |buffer| buffer.len())
                        }
                    });
                    scope.spawn(|| {
                        for _ in 0..10 {
                            buffer
                                .produce(&mut producer.lock().unwrap(), b"hello world")
                                .unwrap();
                        }
                    });
                });
            });

            ConsumerToken::new(|consumer| {
                let consumer = Mutex::new(consumer);

                let buffer = RingBuffer::<1024>::new();
                // spawn a new consumer thread
                std::thread::scope(|scope| {
                    scope.spawn(|| {
                        for _ in 0..10 {
                            buffer.consume(&mut consumer.lock().unwrap(), |buffer| buffer.len())
                        }
                    });
                    scope.spawn(|| {
                        for _ in 0..10 {
                            buffer
                                .produce(&mut producer.lock().unwrap(), b"hello world")
                                .unwrap();
                        }
                    });
                });
            });
        });
    }

    #[test]
    fn overflow() {
        ProducerToken::new(|mut producer| {
            ConsumerToken::new(|_consumer| {
                let buffer = RingBuffer::<64>::new();
                let data = [b'A'; 32];

                assert_eq!(buffer.produce(&mut producer, &data).unwrap(), data.len());
                assert_eq!(buffer.produce(&mut producer, &data).unwrap(), data.len());

                // this should fail as buffer is full
                assert!(buffer.produce(&mut producer, &data).is_err());
            });
        });
    }

    #[test]
    fn consume_full_buffer() {
        ProducerToken::new(|mut producer| {
            ConsumerToken::new(|mut consumer| {
                let buffer = RingBuffer::<64>::new();
                let data = [b'A'; 32];

                assert_eq!(buffer.produce(&mut producer, &data).unwrap(), data.len());
                assert_eq!(buffer.produce(&mut producer, &data).unwrap(), data.len());

                // consume the whole buffer
                assert!(buffer.len(&consumer) == 64);
                buffer.consume(&mut consumer, |_buffer| 64);
                assert!(buffer.len(&consumer) == 0);
            });
        });
    }

    #[test]
    fn consume_full_buffer_and_fill_again() {
        ProducerToken::new(|mut producer| {
            ConsumerToken::new(|mut consumer| {
                let buffer = RingBuffer::<64>::new();

                let data = [b'A'; 32];
                assert_eq!(buffer.produce(&mut producer, &data).unwrap(), data.len());
                assert_eq!(buffer.produce(&mut producer, &data).unwrap(), data.len());

                // consume the whole buffer
                assert!(buffer.len(&consumer) == 64);
                buffer.consume(&mut consumer, |_buffer| 64);
                assert!(buffer.len(&consumer) == 0);

                let data = [b'A'; 8];
                assert_eq!(buffer.produce(&mut producer, &data).unwrap(), data.len());
                assert_eq!(buffer.produce(&mut producer, &data).unwrap(), data.len());
                assert_eq!(buffer.produce(&mut producer, &data).unwrap(), data.len());
                assert_eq!(buffer.produce(&mut producer, &data).unwrap(), data.len());
                assert_eq!(buffer.produce(&mut producer, &data).unwrap(), data.len());
                assert_eq!(buffer.produce(&mut producer, &data).unwrap(), data.len());
                assert_eq!(buffer.produce(&mut producer, &data).unwrap(), data.len());
                assert_eq!(buffer.produce(&mut producer, &data).unwrap(), data.len());

                // consume the whole buffer
                assert!(buffer.len(&consumer) == 64);
                buffer.consume(&mut consumer, |_buffer| 64);
                assert!(buffer.len(&consumer) == 0);
            });
        });
    }

    #[test]
    fn read_buffer_loop() {
        ProducerToken::new(|mut producer| {
            // create new `connections`
            for _ in 0..100 {
                ConsumerToken::new(|mut consumer| {
                    let buffer = RingBuffer::<64>::new();
                    let data = b"hello world";
                    assert_eq!(buffer.produce(&mut producer, data).unwrap(), data.len());

                    // read the whole buffer
                    buffer.consume(&mut consumer, |mut buffer| {
                        let mut buf = [0u8; 16];
                        let len = buffer.read(&mut buf).unwrap();

                        assert_eq!(len, data.len());
                        assert_eq!(buf[..len], data[..]);

                        0
                    });
                });
            }
        });
    }

    #[test]
    fn read_buffer() {
        ProducerToken::new(|mut producer| {
            ConsumerToken::new(|mut consumer| {
                let buffer = RingBuffer::<64>::new();
                let data = b"hello world";
                assert_eq!(buffer.produce(&mut producer, data).unwrap(), data.len());

                // read the whole buffer
                buffer.consume(&mut consumer, |mut buffer| {
                    let mut buf = [0u8; 16];
                    let len = buffer.read(&mut buf).unwrap();

                    assert_eq!(len, data.len());
                    assert_eq!(buf[..len], data[..]);

                    0
                });
            });
        });
    }

    #[test]
    fn read_full_buffer() {
        ProducerToken::new(|mut producer| {
            ConsumerToken::new(|mut consumer| {
                let buffer = RingBuffer::<64>::new();
                let data = [b'A'; 32];

                assert_eq!(buffer.produce(&mut producer, &data).unwrap(), data.len());
                assert_eq!(buffer.produce(&mut producer, &data).unwrap(), data.len());

                // read the whole buffer
                buffer.consume(&mut consumer, |mut buffer| {
                    let mut buf = [0u8; 64];
                    let len = buffer.read(&mut buf).unwrap();

                    assert_eq!(len, data.len() * 2);
                    assert_eq!(buf[..len / 2], data[..]);
                    assert_eq!(buf[len / 2..len], data[..]);

                    0
                });
            });
        });
    }

    #[test]
    fn read_buffer_two_times() {
        ProducerToken::new(|mut producer| {
            ConsumerToken::new(|mut consumer| {
                let buffer = RingBuffer::<64>::new();
                let data = b"hello world";
                assert_eq!(buffer.produce(&mut producer, data).unwrap(), data.len());

                // read the whole buffer
                buffer.consume(&mut consumer, |mut buffer| {
                    let mut buf = [0u8; 16];
                    let len = buffer.read(&mut buf).unwrap();

                    assert_eq!(len, data.len());
                    assert_eq!(buf[..len], data[..]);

                    0
                });

                // read the whole buffer a second time
                buffer.consume(&mut consumer, |mut buffer| {
                    let mut buf = [0u8; 16];
                    let len = buffer.read(&mut buf).unwrap();

                    assert_eq!(len, data.len());
                    assert_eq!(buf[..len], data[..]);

                    0
                });
            });
        });
    }

    #[test]
    fn read_buffer_two_times_but_consume() {
        ProducerToken::new(|mut producer| {
            ConsumerToken::new(|mut consumer| {
                let buffer = RingBuffer::<64>::new();
                let data = b"hello world";
                assert_eq!(buffer.produce(&mut producer, data).unwrap(), data.len());

                // read the whole buffer
                buffer.consume(&mut consumer, |mut buffer| {
                    let mut buf = [0u8; 16];
                    let len = buffer.read(&mut buf).unwrap();

                    assert_eq!(len, data.len());
                    assert_eq!(buf[..len], data[..]);

                    len
                });

                // read the whole buffer a second time
                buffer.consume(&mut consumer, |mut buffer| {
                    let mut buf = [0u8; 16];
                    let len = buffer.read(&mut buf).unwrap();

                    assert_eq!(len, 0);

                    0
                });
            });
        });
    }

    #[test]
    fn read_buffer_multiple_times_wrapping() {
        ProducerToken::new(|mut producer| {
            ConsumerToken::new(|mut consumer| {
                let buffer = RingBuffer::<32>::new();
                const DATA: &[u8] = b"hello world";

                std::thread::scope(|scope| {
                    scope.spawn(|| {
                        let mut count = 0;
                        while count < 10 {
                            if let Ok(bytes) = buffer.produce(&mut producer, DATA) {
                                assert_eq!(bytes, DATA.len());
                                count += 1
                            }
                        }
                    });

                    scope.spawn(|| {
                        let mut count = 0;
                        while count < 10 {
                            buffer.consume(&mut consumer, |mut buffer| {
                                let mut buf = [0u8; DATA.len()];
                                let len = buffer.read(&mut buf).unwrap();

                                if len == DATA.len() {
                                    assert_eq!(&buf[..], DATA);
                                    count += 1;
                                    len
                                } else {
                                    0
                                }
                            });
                        }
                    });
                });
            });
        });
    }

    #[test]
    fn silly() {
        const SIZE: usize = 32;
        const DATA: &[u8] = b"hello world";
        const ITERATIONS: usize = 10_000_000;

        ProducerToken::new(|mut producer| {
            ConsumerToken::new(|mut consumer| {
                let buffer = RingBuffer::<SIZE>::new();
                std::thread::scope(|scope| {
                    scope.spawn(|| {
                        for _ in 0..ITERATIONS {
                            buffer.write_all(&mut producer, DATA).unwrap()
                        }
                        println!("producer done");
                    });
                    scope.spawn(|| {
                        let mut buf = [0u8; DATA.len()];
                        for _ in 0..ITERATIONS {
                            buffer.read_exact(&mut consumer, &mut buf).unwrap();
                            if buf != DATA {
                                scope.spawn(move || assert!(buf == DATA));
                                exit(1);
                            }
                        }
                        println!("consumer done");
                    });
                });
            });
        });
    }

    #[test]
    #[should_panic]
    fn consume_more_than_available_data() {
        ProducerToken::new(|mut producer| {
            ConsumerToken::new(|mut consumer| {
                let buffer = RingBuffer::<64>::new();
                let data = b"hello world";
                assert_eq!(buffer.produce(&mut producer, data).unwrap(), data.len());

                // read the whole buffer
                buffer.consume(&mut consumer, |_buffer| 32);
            });
        });
    }

    #[test]
    #[should_panic]
    fn consume_more_than_available_capacity() {
        ProducerToken::new(|mut producer| {
            ConsumerToken::new(|mut consumer| {
                let buffer = RingBuffer::<64>::new();
                let data = b"hello world";
                assert_eq!(buffer.produce(&mut producer, data).unwrap(), data.len());

                // read the whole buffer
                buffer.consume(&mut consumer, |_buffer| 256);
            });
        });
    }
}
