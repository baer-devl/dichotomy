use criterion::{criterion_group, criterion_main, Criterion};
use dichotomy::*;
use std::io::{Read, Write};

fn async_multi_rw_1m() {
    const BUF_SIZE: usize = 1024;
    const DATA: &[u8] = b"hello world";
    const ITERATIONS: usize = 1_000_000;

    let (mut producer, mut consumer) = Buffer::<BUF_SIZE>::new();
    std::thread::scope(|scope| {
        // producer
        scope.spawn(|| {
            for _ in 0..ITERATIONS {
                let mut written = 0;
                while written < DATA.len() {
                    let bytes = producer.write(&DATA[written..]).unwrap();
                    written += bytes;
                }
            }
        });

        // consumer
        scope.spawn(|| {
            let mut buf = [0u8; DATA.len()];
            for _ in 0..ITERATIONS {
                let mut read = 0;
                while read < DATA.len() {
                    let bytes = consumer.read(&mut buf[read..]).unwrap();
                    read += bytes;
                }
                assert!(DATA == buf)
            }
        });
    });
}

fn criterion_benchmark(c: &mut Criterion) {
    {
        let mut group = c.benchmark_group("sample size of 100");
        group.sample_size(100);
        group.bench_function("async-rw-1m", |b| b.iter(|| async_multi_rw_1m()));
    }
    {
        let mut group = c.benchmark_group("sample size of 500");
        group.sample_size(500);
        group.bench_function("async-rw-1m", |b| b.iter(|| async_multi_rw_1m()));
    }
    {
        let mut group = c.benchmark_group("sample size of 5000");
        group.sample_size(5_000);
        group.bench_function("async-rw-1m", |b| b.iter(|| async_multi_rw_1m()));
    }
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
