use crate::Buffer;
use std::io::{Read, Write};

/// Standard way of using `dichotomy` buffer
#[test]
fn intended() {
    const SIZE: usize = 32;
    const DATA: &[u8] = b"hello world";
    const ITERATIONS: usize = 100_000_000;

    let (mut producer, mut consumer) = Buffer::<SIZE>::new();

    let t = std::thread::spawn(move || {
        let mut buf = [0u8; DATA.len()];
        for _ in 0..ITERATIONS {
            let mut read = 0;
            while read < DATA.len() {
                let bytes = consumer.read(&mut buf[read..]).unwrap();
                read += bytes;
            }
            assert!(buf == DATA);
        }
    });

    for _ in 0..ITERATIONS {
        let mut written = 0;
        while written < DATA.len() {
            let bytes = producer.write(&DATA[written..]).unwrap();
            written += bytes;
        }
    }

    t.join().unwrap();
}

// TODO: different modules for specialized test
//  - standard
//  - edge-cases (max allowed size, small buffer size, ..)
//  - panics (MUST panic tests)
//  - misc (producer|consumer exists, ..)
//  - producer
//  - consumer
//  - use random generated values

// Producer
//pub fn len(&self) -> usize
//pub fn is_full(&self) -> bool
//pub fn is_consumer_available(&self) -> bool

// Consumer
//pub fn len(&self) -> usize
//pub fn is_empty(&self) -> bool
//pub fn is_producer_available(&self) -> bool
