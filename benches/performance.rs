use criterion::{criterion_group, criterion_main, Criterion};
use dichotomy::*;

fn bench_1() {
    const BUF_SIZE: usize = 1;
    const DATA: &[u8] = b"hello world";
    const ITERATIONS: usize = 1_000_000;

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

                //assert!(DATA == buf)
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
fn bench_32() {
    const BUF_SIZE: usize = 32;
    const DATA: &[u8] = b"hello world";
    const ITERATIONS: usize = 1_000_000;

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

                //assert!(DATA == buf)
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
fn bench_64() {
    const BUF_SIZE: usize = 64;
    const DATA: &[u8] = b"hello world";
    const ITERATIONS: usize = 1_000_000;

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

                //assert!(DATA == buf)
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
fn bench_512() {
    const BUF_SIZE: usize = 512;
    const DATA: &[u8] = b"hello world";
    const ITERATIONS: usize = 1_000_000;

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

                //assert!(DATA == buf)
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
fn bench_1024() {
    const BUF_SIZE: usize = 1024;
    const DATA: &[u8] = b"hello world";
    const ITERATIONS: usize = 1_000_000;

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

                //assert!(DATA == buf)
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
fn bench_2048() {
    const BUF_SIZE: usize = 2048;
    const DATA: &[u8] = b"hello world";
    const ITERATIONS: usize = 1_000_000;

    let (mut producer, mut consumer) = Buffer::<BUF_SIZE>::new();
    std::thread::scope(|scope| {
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

                //assert!(DATA == buf)
            }
        });
    });
}

fn criterion_benchmark(c: &mut Criterion) {
    {
        let mut group = c.benchmark_group("sample-size-500");
        group.sample_size(500);
        group.bench_function("---1b-buffer", |b| b.iter(|| bench_1()));
        group.bench_function("--32b-buffer", |b| b.iter(|| bench_32()));
        group.bench_function("--64b-buffer", |b| b.iter(|| bench_64()));
        group.bench_function("-512b-buffer", |b| b.iter(|| bench_512()));
        group.bench_function("1024b-buffer", |b| b.iter(|| bench_1024()));
        group.bench_function("2048b-buffer", |b| b.iter(|| bench_2048()));
    }
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
