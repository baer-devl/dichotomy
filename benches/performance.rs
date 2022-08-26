use criterion::{criterion_group, criterion_main, Criterion};
use dual_access_ringbuffer::*;

fn read_buffer() {
    const DATA: &[u8] = b"hello world";

    ProducerToken::new(|mut producer| {
        ConsumerToken::new(|mut consumer| {
            let buffer = RingBuffer::<64>::new();
            buffer.write_all(&mut producer, DATA).unwrap();

            // read the whole buffer
            let mut buf = [0u8; DATA.len()];
            buffer.read_exact(&mut consumer, &mut buf).unwrap();
        });
    });
}

fn multi_read_buffer() {
    const DATA: &[u8] = b"hello world";

    ProducerToken::new(|mut producer| {
        ConsumerToken::new(|consumer| {
            let buffer = RingBuffer::<64>::new();
            buffer.write_all(&mut producer, DATA).unwrap();

            // read the whole buffer
            for _ in 0..100 {
                let mut buf = [0u8; DATA.len()];
                buffer.read(&consumer, &mut buf).unwrap();
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
                    for _ in 0..ITERATIONS {
                        buffer.write_all(&mut producer, DATA).unwrap()
                    }
                });

                // consumer
                scope.spawn(|| {
                    let mut buf = [0u8; DATA.len()];
                    for _ in 0..ITERATIONS {
                        buffer.read_exact(&mut consumer, &mut buf).unwrap();
                        assert!(DATA == buf)
                    }
                });
            });
        });
    });
}

fn criterion_benchmark(c: &mut Criterion) {
    {
        let mut group = c.benchmark_group("sample size of 100");
        group.sample_size(100);
        group.bench_function("read-buffer", |b| b.iter(|| read_buffer()));
        group.bench_function("multi-read-buffer", |b| b.iter(|| multi_read_buffer()));
        group.bench_function("async-rw-1m", |b| b.iter(|| async_multi_rw_1m()));
    }
    {
        let mut group = c.benchmark_group("sample size of 500");
        group.sample_size(500);
        group.bench_function("read-buffer", |b| b.iter(|| read_buffer()));
        group.bench_function("multi-read-buffer", |b| b.iter(|| multi_read_buffer()));
        group.bench_function("async-rw-1m", |b| b.iter(|| async_multi_rw_1m()));
    }
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
