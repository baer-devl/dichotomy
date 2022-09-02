use crate::Buffer;
use std::io::{Read, Write};

#[test]
fn len_empty_buffer() {
    const SIZE: usize = 32;

    let (producer, _consumer) = Buffer::<SIZE>::new();
    assert_eq!(SIZE, producer.len());
}

#[test]
fn len_full_buffer() {
    const SIZE: usize = 32;

    let (mut producer, _consumer) = Buffer::<SIZE>::new();
    let mut buf = [0u8; SIZE];
    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();

    assert_eq!(0, producer.len());
}

#[test]
fn len_partial_full_buffer() {
    const SIZE: usize = 32;
    const DATA: usize = 24;

    let (mut producer, _consumer) = Buffer::<SIZE>::new();
    let mut buf = [0u8; DATA];
    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();

    assert_eq!(SIZE - DATA, producer.len());
}

#[test]
fn len_partial_full_wrapping_buffer() {
    const SIZE: usize = 32;
    const DATA: usize = 24;

    let (mut producer, mut consumer) = Buffer::<SIZE>::new();
    let mut buf = [0u8; DATA];
    let mut buf_read = [0u8; DATA];
    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();
    consumer.read(&mut buf_read).unwrap();

    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();

    assert_eq!(SIZE - DATA, producer.len());
}

#[test]
fn len_full_wrapping_buffer() {
    const SIZE: usize = 32;
    const DATA: usize = 24;

    let (mut producer, mut consumer) = Buffer::<SIZE>::new();
    let mut buf = [0u8; DATA];
    let mut buf_read = [0u8; DATA];
    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();
    consumer.read(&mut buf_read).unwrap();

    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();
    super::get_random_values(&mut buf[..SIZE - DATA]);
    producer.write(&buf[..SIZE - DATA]).unwrap();

    assert_eq!(0, producer.len());
}

#[test]
fn is_full_empty_buffer() {
    const SIZE: usize = 32;

    let (producer, _consumer) = Buffer::<SIZE>::new();
    assert!(!producer.is_full());
}

#[test]
fn is_full_full_buffer() {
    const SIZE: usize = 32;

    let (mut producer, _consumer) = Buffer::<SIZE>::new();
    let mut buf = [0u8; SIZE];
    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();

    assert!(producer.is_full());
}

#[test]
fn is_full_partial_full_buffer() {
    const SIZE: usize = 32;
    const DATA: usize = 24;

    let (mut producer, _consumer) = Buffer::<SIZE>::new();
    let mut buf = [0u8; DATA];
    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();

    assert!(!producer.is_full());
}

#[test]
fn is_full_partial_full_wrapping_buffer() {
    const SIZE: usize = 32;
    const DATA: usize = 24;

    let (mut producer, mut consumer) = Buffer::<SIZE>::new();
    let mut buf = [0u8; DATA];
    let mut buf_read = [0u8; DATA];
    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();
    consumer.read(&mut buf_read).unwrap();

    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();

    assert!(!producer.is_full());
}

#[test]
fn is_full_full_wrapping_buffer() {
    const SIZE: usize = 32;
    const DATA: usize = 24;

    let (mut producer, mut consumer) = Buffer::<SIZE>::new();
    let mut buf = [0u8; DATA];
    let mut buf_read = [0u8; DATA];
    super::get_random_values(&mut buf);
    println!("len: {}", producer.buffer.len());
    let b = producer.write(&buf).unwrap();
    println!("len: {} {}", b, producer.buffer.len());

    let b = consumer.read(&mut buf_read).unwrap();
    println!("len: {} {}", b, producer.buffer.len());

    super::get_random_values(&mut buf);
    let b = producer.write(&buf).unwrap();
    println!("len: {} {}", b, producer.buffer.len());
    super::get_random_values(&mut buf[..SIZE - DATA]);
    let b = producer.write(&buf[..SIZE - DATA]).unwrap();
    println!("len: {} {}", b, producer.buffer.len());

    assert!(producer.is_full());
}

#[test]
fn is_abandoned_existing() {
    const SIZE: usize = 32;
    let (producer, _consumer) = Buffer::<SIZE>::new();

    assert!(!producer.is_abandoned());
}

#[test]
fn is_abandoned_dropped() {
    const SIZE: usize = 32;
    let (producer, consumer) = Buffer::<SIZE>::new();
    drop(consumer);

    assert!(producer.is_abandoned());
}
