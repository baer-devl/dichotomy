use dichotomy::Buffer;
use std::io::{Read, Write};

const BUF_SIZE: usize = 32;
const DATA: &[u8] = b"hello world";
const ITERATIONS: usize = 100_000_000;

fn main() {
    let (mut producer, mut consumer) = Buffer::<BUF_SIZE>::new();
    std::thread::scope(|scope| {
        // consumer
        scope.spawn(|| {
            let mut buf = [0u8; DATA.len()];
            for _ in 0..ITERATIONS {
                let mut read = 0;
                while read < DATA.len() {
                    if let Ok(bytes) = consumer.read(&mut buf[read..]) {
                        read += bytes;
                    }
                }

                assert!(DATA == buf)
            }
        });

        // producer
        scope.spawn(|| {
            for _ in 0..ITERATIONS {
                let mut written = 0;
                while written < DATA.len() {
                    if let Ok(bytes) = producer.write(&DATA[written..]) {
                        written += bytes;
                    }
                }
            }
        });
    });
}
