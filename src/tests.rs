#[cfg(all(test, feature = "blocking"))]
mod blocking {
    use crate::Buffer;

    #[test]
    fn intended() {
        const SIZE: usize = 32;
        const DATA: &[u8] = b"hello world";
        const ITERATIONS: usize = 100_000_000;

        let (mut producer, mut consumer) = Buffer::<SIZE>::new();

        let t = std::thread::spawn(move || {
            let mut buf = [0u8; DATA.len()];
            for _ in 0..ITERATIONS {
                consumer.read_blocking(&mut buf).unwrap();
                assert!(buf == DATA);
            }
        });

        for _ in 0..ITERATIONS {
            producer.write_blocking(DATA).unwrap()
        }

        t.join().unwrap();
    }
}

mod default {
    use crate::Buffer;
    use std::io::{Read, Write};

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
}
