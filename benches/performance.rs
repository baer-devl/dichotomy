use criterion::{criterion_group, criterion_main, Criterion};
use dual_access_ringbuffer::*;
use std::io::Read;

fn read_buffer() {
    ProducerToken::new(|mut producer| {
        ConsumerToken::new(|mut consumer| {
            let buffer = RingBuffer::<64>::new();
            let data = b"hello world";
            buffer.produce(&mut producer, data).unwrap();

            // read the whole buffer
            buffer.consume(&mut consumer, |mut buffer| {
                let mut buf = [0u8; 16];
                buffer.read(&mut buf).unwrap()
            });
        });
    });
}

fn multi_read_buffer() {
    ProducerToken::new(|mut producer| {
        ConsumerToken::new(|consumer| {
            let buffer = RingBuffer::<64>::new();
            let data = b"hello world";
            buffer.produce(&mut producer, data).unwrap();

            // read the whole buffer
            for _ in 0..100 {
                buffer.read(&consumer, |mut buffer| {
                    let mut buf = [0u8; 16];
                    let _ = buffer.read(&mut buf).unwrap();
                });
            }
        });
    });
}

fn async_multi_rw_1m() {
    const BUF_SIZE: usize = 1024;
    const DATA: &[u8] = b"hello world";
    const ITERATIONS: usize = 1_000_000;

    ProducerToken::new(|mut producer| {
        ConsumerToken::new(|mut consumer| {
            let buffer = RingBuffer::<BUF_SIZE>::new();

            std::thread::scope(|scope| {
                // producer
                scope.spawn(|| {
                    let mut pushes = 0;
                    while pushes < ITERATIONS {
                        if let Ok(bytes) = buffer.produce(&mut producer, DATA) {
                            assert!(bytes == DATA.len());
                            pushes += 1;
                        } else {
                            // nothing to do - yield
                            std::thread::yield_now();
                        }
                    }
                });

                // consumer
                scope.spawn(|| {
                    let mut buf = [0u8; DATA.len()];
                    let mut reads = 0;
                    while reads < ITERATIONS {
                        buffer.consume(&mut consumer, |mut buffer| {
                            if buffer.read_exact(&mut buf).is_ok() {
                                reads += 1;
                                DATA.len()
                            } else {
                                // nothing to do - yield
                                std::thread::yield_now();
                                0
                            }
                        });
                    }
                });
            });
        });
    });
}

fn criterion_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("sample-500");
    group.sample_size(500);
    group.bench_function("read-buffer", |b| b.iter(|| read_buffer()));
    group.bench_function("multi-read-buffer", |b| b.iter(|| multi_read_buffer()));
    group.bench_function("async-rw-1m", |b| b.iter(|| async_multi_rw_1m()));
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
