pub use buffer::Buffer;
use marker::InvariantLifetime;
pub use marker::{ConsumerToken, ProducerToken};
use std::{cell::UnsafeCell, io::Write};

mod buffer;
mod marker;

/// Dual consumption ring buffer
pub struct RingBuffer<'producer, 'consumer, const N: usize> {
    // used to ensure exclusive access to the producer related variables
    _marker_producer: InvariantLifetime<'producer>,
    // used to ensure exclusive access to the consumer related variables
    _marker_consumer: InvariantLifetime<'consumer>,

    // binary buffer which can be accessed by both, the producer and consumer
    // however, they are accessing different areas and thus it should be fine
    buffer: UnsafeCell<[u8; N]>,

    data_start: UnsafeCell<usize>,
    data_end: UnsafeCell<usize>,
}

// FIXME: this should be fine as we do not change data without the marker which gives us exclusive
// access
unsafe impl<'producer, 'consumer, const N: usize> Send for RingBuffer<'producer, 'consumer, N> {}
// FIXME: this should be fine as the marker will only allow for exclusive access
unsafe impl<'producer, 'consumer, const N: usize> Sync for RingBuffer<'producer, 'consumer, N> {}

impl<'producer, 'consumer, const N: usize> Default for RingBuffer<'producer, 'consumer, N> {
    fn default() -> Self {
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
    pub fn len(&self) -> usize {
        let start = unsafe { *self.data_start.get() };
        let end = unsafe { *self.data_end.get() };

        // check if buffer is full
        if end > N {
            return N;
        }

        // check if buffer is empty
        if start == end {
            return 0;
        }

        // check if data is wrapping
        if start < end {
            // |---S*****E-------|
            end - start
        } else {
            // |*E----------S****|
            N - start + end
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
    pub fn produce<'a>(
        &self,
        producer: &'a mut ProducerToken<'producer>,
        buf: &[u8],
    ) -> std::io::Result<usize> {
        // get access to empty buffer and write to it
        let end = unsafe { *self.data_end.get() };
        let mut buffer = self.get_empty_buffer(producer)?;
        let buffer_len = buffer.len();
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

    /// Return all free buffer as writable
    fn get_empty_buffer<'a>(&self, _: &'a mut ProducerToken<'producer>) -> std::io::Result<Buffer> {
        let start = unsafe { *self.data_start.get() };
        let end = unsafe { *self.data_end.get() };

        // S=E -> empty
        // E>N -> full
        // check if we are full
        if end > N {
            // do not update buffer
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
        let start = unsafe { *self.data_start.get() };
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
    use std::{io::Read, sync::Mutex};

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
                assert!(buffer.len() == 64);
                buffer.consume(&mut consumer, |_buffer| 64);
                assert!(buffer.len() == 0);
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
                assert!(buffer.len() == 64);
                buffer.consume(&mut consumer, |_buffer| 64);
                assert!(buffer.len() == 0);

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
                assert!(buffer.len() == 64);
                buffer.consume(&mut consumer, |_buffer| 64);
                assert!(buffer.len() == 0);
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
