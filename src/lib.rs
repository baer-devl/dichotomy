pub use buffer::Buffer;
use marker::InvariantLifetime;
pub use marker::{ConsumerToken, ProducerToken};
use std::{
    cell::UnsafeCell,
    io::Write,
    sync::atomic::{AtomicU32, AtomicUsize, Ordering},
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
    // however, they are accessing different areas and thus it should be fine
    buffer: UnsafeCell<[u8; N]>,

    // marks the begin of consumable data in the buffer
    // the first 4 bytes are the start of data
    // the last 4 bytes are the length of data
    indicies: AtomicUsize,
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
            indicies: AtomicUsize::default(),
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
    pub fn available(&self) -> usize {
        N - self.indicies.load(Ordering::SeqCst) << 32 >> 32
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
        _: &'a mut ProducerToken<'producer>,
        buf: &[u8],
    ) -> std::io::Result<usize> {
        // get access to empty buffer and write to it
        let mut buffer = self.get_empty_buffer()?;
        let bytes = buffer.write(buf)?;

        // update length of data in buffer
        self.indicies
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |idx| {
                let start = (idx >> 32) as u32;
                let mut len = (idx << 32 >> 32) as u32;

                // FIXME: resolv panic
                len = len.checked_add(bytes as u32).unwrap();
                Some((start as usize) << 32 | len as usize)
            })
            .map_err(|err| {
                println!("[FAIL] - produce");
                err
            })
            .unwrap();
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
    pub fn consume<'a, F>(&self, _: &'a mut ConsumerToken<'consumer>, fun: F)
    where
        F: FnOnce(Buffer) -> usize,
    {
        let buffer = self.get_consumable_buffer();
        let consumed = fun(self.get_consumable_buffer());

        // update start of buffer after validity check
        assert!(consumed <= buffer.len());

        // this should panic as we cannot assume a safe state if those operations panic
        self.indicies
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |idx| {
                let mut start = (idx >> 32) as u32;
                let mut len = (idx << 32 >> 32) as u32;

                // consume will read data in the buffer object which can be consumed by another
                // thread as well
                //
                // add another field: reserved
                // |--R+++S******E----|
                //
                // 1) consume always consume the buffer by increasing start / decreasing length
                //
                // 2) after returning function - increase reset value (clear buffer)
                //
                //
                println!("{} <= {}", consumed, len);

                // FIXME: resolv panic
                start = (start + consumed as u32) % N as u32;
                len -= consumed as u32;

                /*println!(
                    "{} {} -> {} {}",
                    start,
                    len,
                    (start as usize) << 32 | len as usize,
                    (start as usize) << 32 | (len as usize)
                );*/

                Some((start as usize) << 32 | len as usize)
            })
            .map_err(|err| {
                println!("[FAIL - consume {}]", err);
                err
            })
            .unwrap();
    }

    /// Return all free buffer as writable
    fn get_empty_buffer(&self) -> std::io::Result<Buffer> {
        let mut buffer = None;
        let _ = self
            .indicies
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |idx| {
                let start = idx >> 32;
                let len = idx << 32 >> 32;

                // check if we are full
                if len == N {
                    // do not update buffer
                    return None;
                }

                // check if free data is wrapping
                if start + len > N {
                    // |***E--------S***|
                    let buffer0 =
                        &mut unsafe { &mut *self.buffer.get() }[(start + len) % N..N - len];
                    let buffer1 = &mut [];

                    buffer = Some(Buffer(buffer0, buffer1));
                } else {
                    // |--S*****E-------|
                    let buffer0 = &mut unsafe { &mut *self.buffer.get() }[start + len..];
                    let buffer1 = &mut unsafe { &mut *self.buffer.get() }[..start];

                    buffer = Some(Buffer(buffer0, buffer1));
                }

                None
            });

        if let Some(buffer) = buffer {
            Ok(buffer)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::OutOfMemory,
                "Buffer is full",
            ))
        }
    }

    /// Return all assigned buffer as writable
    fn get_consumable_buffer(&self) -> Buffer {
        let mut buffer = None;
        let _ = self
            .indicies
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |idx| {
                let start = idx >> 32;
                let len = idx << 32 >> 32;

                // check if empty
                if len == 0 {
                    // do not update buffer
                    return None;
                }

                // check if data assigned is wrapping
                if start + len > N {
                    // |***E--------S***|
                    let buffer0 = &mut unsafe { &mut *self.buffer.get() }[start..];
                    let buffer1 = &mut unsafe { &mut *self.buffer.get() }[..(start + len) % N];

                    buffer = Some(Buffer(buffer0, buffer1));
                } else {
                    // |--S*****E-------|
                    let buffer0 = &mut unsafe { &mut *self.buffer.get() }[start..start + len];
                    let buffer1 = &mut [];

                    buffer = Some(Buffer(buffer0, buffer1));
                }

                None
            });

        if let Some(buffer) = buffer {
            buffer
        } else {
            Buffer(&mut [], &mut [])
        }
    }
}

#[cfg(test)]
mod test {
    use crate::{ConsumerToken, ProducerToken, RingBuffer};
    use std::{io::Read, sync::Mutex};

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
                            buffer.consume(&mut consumer.lock().unwrap(), |mut buffer| {
                                let mut b = [0u8; 512];
                                let l = buffer.read(&mut b).unwrap();

                                println!("c0-{}: {}", n, l / 11);
                                l
                            })
                        }
                    });
                    scope.spawn(|| {
                        for n in 0..10 {
                            buffer.consume(&mut consumer.lock().unwrap(), |mut buffer| {
                                let mut b = [0u8; 512];
                                let l = buffer.read(&mut b).unwrap();

                                println!("c1-{}: {}", n, l / 11);
                                l
                            })
                        }
                    });
                    scope.spawn(|| {
                        for n in 0..10 {
                            buffer.consume(&mut consumer.lock().unwrap(), |mut buffer| {
                                let mut b = [0u8; 512];
                                let l = buffer.read(&mut b).unwrap();

                                println!("c2-{}: {}", n, l / 11);
                                l
                            })
                        }
                    });
                    scope.spawn(|| {
                        for n in 0..10 {
                            buffer.consume(&mut consumer.lock().unwrap(), |mut buffer| {
                                let mut b = [0u8; 512];
                                let l = buffer.read(&mut b).unwrap();

                                println!("c3-{}: {}", n, l / 11);
                                l
                            })
                        }
                    });

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
                            println!("p2-{}: wrote", n);
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
                assert!(buffer.available() == 0);
                buffer.consume(&mut consumer, |_buffer| 64);
                assert!(buffer.available() == 64);
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
                assert!(buffer.available() == 0);
                buffer.consume(&mut consumer, |_buffer| 64);
                assert!(buffer.available() == 64);

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
                assert!(buffer.available() == 0);
                buffer.consume(&mut consumer, |_buffer| 64);
                assert!(buffer.available() == 64);
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
