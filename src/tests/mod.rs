use crate::Buffer;
use rand::{self, Rng};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

mod consumer;
mod producer;

fn get_random_values(buffer: &mut [u8]) {
    let mut rand = rand::thread_rng();
    rand.fill(buffer);
}

#[test]
fn write_read_random_value_empty_buffer() {
    const DATA: usize = 10;
    const SIZE: usize = 32;

    let (mut producer, mut consumer) = Buffer::<SIZE>::new();

    // produce values
    let mut written = 0;
    let mut data = [0u8; DATA];
    get_random_values(&mut data);

    while written < DATA {
        let bytes = producer.write(&data[written..]).unwrap();
        written += bytes;
    }

    // read alues
    let mut buf = [0u8; DATA];
    let mut read = 0;
    while read < DATA {
        let bytes = consumer.read(&mut buf[read..]).unwrap();
        read += bytes;
    }

    assert!(&buf[..read] == data);
}

#[test]
fn write_read_random_value_wrapping_buffer() {
    const DATA: usize = 24;
    const SIZE: usize = 32;

    let (mut producer, mut consumer) = Buffer::<SIZE>::new();

    // produce value
    let mut written = 0;
    let mut data = [0u8; DATA];
    get_random_values(&mut data);

    while written < DATA {
        let bytes = producer.write(&data[written..]).unwrap();
        written += bytes;
    }

    // read value
    let mut buf = [0u8; DATA];
    let mut read = 0;
    while read < DATA {
        let bytes = consumer.read(&mut buf[read..]).unwrap();
        read += bytes;
    }
    assert!(&buf[..read] == data);

    // produce wrapping value
    get_random_values(&mut data);

    let mut written = 0;
    while written < DATA {
        let bytes = producer.write(&data[written..]).unwrap();
        written += bytes;
    }

    // read value
    let mut buf = [0u8; DATA];
    let mut read = 0;
    while read < DATA {
        let bytes = consumer.read(&mut buf[read..]).unwrap();
        read += bytes;
    }
    assert!(&buf[..read] == data);
}

#[test]
fn write_read_random_values_byte_size_buffer() {
    const DATA: usize = 10;
    const SIZE: usize = 1;
    const ITERATIONS: usize = 1_000;

    let values: Arc<Mutex<VecDeque<[u8; DATA]>>> = Arc::new(Mutex::new(VecDeque::new()));
    let values_cpy = values.clone();
    let (mut producer, mut consumer) = Buffer::<SIZE>::new();

    // consumer thread
    let t = std::thread::spawn(move || {
        let mut buf = [0u8; DATA];
        for _ in 0..ITERATIONS {
            let mut read = 0;
            while read < DATA {
                let bytes = consumer.read(&mut buf[read..]).unwrap();
                read += bytes;
            }
            // wait till producer put the value on the queue
            while values_cpy.lock().unwrap().is_empty() {}
            let data = values_cpy.lock().unwrap().pop_front().unwrap();
            assert!(&buf[..read] == data);
        }
    });

    // produce values
    for _ in 0..ITERATIONS {
        let mut written = 0;
        let mut data = [0u8; DATA];
        get_random_values(&mut data);

        while written < DATA {
            let bytes = producer.write(&data[written..]).unwrap();
            written += bytes;
        }
        values.lock().unwrap().push_back(data);
    }

    t.join().unwrap();
}

#[test]
fn write_read_random_values_wrapping_multiple_times() {
    const DATA: usize = 13;
    const SIZE: usize = 32;
    const ITERATIONS: usize = 1_000;

    let values: Arc<Mutex<VecDeque<[u8; DATA]>>> = Arc::new(Mutex::new(VecDeque::new()));
    let values_cpy = values.clone();
    let (mut producer, mut consumer) = Buffer::<SIZE>::new();

    // consumer thread
    let t = std::thread::spawn(move || {
        let mut buf = [0u8; DATA];
        for _ in 0..ITERATIONS {
            let mut read = 0;
            while read < DATA {
                let bytes = consumer.read(&mut buf[read..]).unwrap();
                read += bytes;
            }
            // wait till producer put the value on the queue
            while values_cpy.lock().unwrap().is_empty() {}
            let data = values_cpy.lock().unwrap().pop_front().unwrap();
            assert!(&buf[..read] == data);
        }
    });

    // produce values
    for _ in 0..ITERATIONS {
        let mut written = 0;
        let mut data = [0u8; DATA];
        get_random_values(&mut data);

        while written < DATA {
            let bytes = producer.write(&data[written..]).unwrap();
            written += bytes;
        }
        values.lock().unwrap().push_back(data);
    }

    t.join().unwrap();
}

#[test]
fn write_read_random_values_multiple_10_1_1m() {
    const DATA: usize = 10;
    const SIZE: usize = 1;
    const ITERATIONS: usize = 1_000_000;

    let values: Arc<Mutex<VecDeque<[u8; DATA]>>> = Arc::new(Mutex::new(VecDeque::new()));
    let values_cpy = values.clone();
    let (mut producer, mut consumer) = Buffer::<SIZE>::new();

    // consumer thread
    let t = std::thread::spawn(move || {
        let mut buf = [0u8; DATA];
        for _ in 0..ITERATIONS {
            let mut read = 0;
            while read < DATA {
                let bytes = consumer.read(&mut buf[read..]).unwrap();
                read += bytes;
            }
            // wait till producer put the value on the queue
            while values_cpy.lock().unwrap().is_empty() {}
            let data = values_cpy.lock().unwrap().pop_front().unwrap();
            assert!(&buf[..read] == data);
        }
    });

    // produce values
    for _ in 0..ITERATIONS {
        let mut written = 0;
        let mut data = [0u8; DATA];
        get_random_values(&mut data);

        while written < DATA {
            let bytes = producer.write(&data[written..]).unwrap();
            written += bytes;
        }
        values.lock().unwrap().push_back(data);
    }

    t.join().unwrap();
}

#[test]
fn write_read_random_values_multiple_120_512_1m() {
    const DATA: usize = 120;
    const SIZE: usize = 512;
    const ITERATIONS: usize = 1_000_000;

    let values: Arc<Mutex<VecDeque<[u8; DATA]>>> = Arc::new(Mutex::new(VecDeque::new()));
    let values_cpy = values.clone();
    let (mut producer, mut consumer) = Buffer::<SIZE>::new();

    // consumer thread
    let t = std::thread::spawn(move || {
        let mut buf = [0u8; DATA];
        for _ in 0..ITERATIONS {
            let mut read = 0;
            while read < DATA {
                let bytes = consumer.read(&mut buf[read..]).unwrap();
                read += bytes;
            }
            // wait till producer put the value on the queue
            while values_cpy.lock().unwrap().is_empty() {}
            let data = values_cpy.lock().unwrap().pop_front().unwrap();
            assert!(&buf[..read] == data);
        }
    });

    // produce values
    for _ in 0..ITERATIONS {
        let mut written = 0;
        let mut data = [0u8; DATA];
        get_random_values(&mut data);

        while written < DATA {
            let bytes = producer.write(&data[written..]).unwrap();
            written += bytes;
        }
        values.lock().unwrap().push_back(data);
    }

    t.join().unwrap();
}

#[test]
fn write_read_random_values_multiple_1001_1024_1m() {
    const DATA: usize = 1001;
    const SIZE: usize = 1024;
    const ITERATIONS: usize = 1_000_000;

    let values: Arc<Mutex<VecDeque<[u8; DATA]>>> = Arc::new(Mutex::new(VecDeque::new()));
    let values_cpy = values.clone();
    let (mut producer, mut consumer) = Buffer::<SIZE>::new();

    // consumer thread
    let t = std::thread::spawn(move || {
        let mut buf = [0u8; DATA];
        for _ in 0..ITERATIONS {
            let mut read = 0;
            while read < DATA {
                let bytes = consumer.read(&mut buf[read..]).unwrap();
                read += bytes;
            }
            // wait till producer put the value on the queue
            while values_cpy.lock().unwrap().is_empty() {}
            let data = values_cpy.lock().unwrap().pop_front().unwrap();
            assert!(&buf[..read] == data);
        }
    });

    // produce values
    for _ in 0..ITERATIONS {
        let mut written = 0;
        let mut data = [0u8; DATA];
        get_random_values(&mut data);

        while written < DATA {
            let bytes = producer.write(&data[written..]).unwrap();
            written += bytes;
        }
        values.lock().unwrap().push_back(data);
    }

    t.join().unwrap();
}
