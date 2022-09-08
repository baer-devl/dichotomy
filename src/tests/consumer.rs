use crate::Buffer;

#[test]
fn len_empty_buffer() {
    const SIZE: usize = 32;

    let (_producer, consumer) = Buffer::<SIZE, u8>::new();
    assert_eq!(0, consumer.len());
}

#[test]
fn len_full_buffer() {
    const SIZE: usize = 32;

    let (mut producer, consumer) = Buffer::<SIZE, u8>::new();
    let mut buf = [0u8; SIZE];
    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();

    assert_eq!(SIZE, consumer.len());
}

#[test]
fn len_partial_full_buffer() {
    const SIZE: usize = 32;
    const DATA: usize = 24;

    let (mut producer, consumer) = Buffer::<SIZE, u8>::new();
    let mut buf = [0u8; DATA];
    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();

    assert_eq!(DATA, consumer.len());
}

#[test]
fn len_partial_full_wrapping_buffer() {
    const SIZE: usize = 32;
    const DATA: usize = 24;

    let (mut producer, mut consumer) = Buffer::<SIZE, u8>::new();
    let mut buf = [0u8; DATA];
    let mut buf_read = [0u8; DATA];
    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();
    consumer.read(&mut buf_read).unwrap();

    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();
    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();

    assert_eq!(SIZE, consumer.len());
}

#[test]
fn len_full_wrapping_buffer() {
    const SIZE: usize = 32;
    const DATA: usize = 24;

    let (mut producer, mut consumer) = Buffer::<SIZE, u8>::new();
    let mut buf = [0u8; DATA];
    let mut buf_read = [0u8; DATA];
    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();
    consumer.read(&mut buf_read).unwrap();

    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();
    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();

    assert_eq!(SIZE, consumer.len());
}

#[test]
fn is_empty_empty_buffer() {
    const SIZE: usize = 32;

    let (_producer, consumer) = Buffer::<SIZE, u8>::new();
    assert!(consumer.is_empty());
}

#[test]
fn is_empty_full_buffer() {
    const SIZE: usize = 32;

    let (mut producer, consumer) = Buffer::<SIZE, u8>::new();
    let mut buf = [0u8; SIZE];
    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();

    assert!(!consumer.is_empty());
}

#[test]
fn is_empty_partial_full_buffer() {
    const SIZE: usize = 32;
    const DATA: usize = 24;

    let (mut producer, consumer) = Buffer::<SIZE, u8>::new();
    let mut buf = [0u8; DATA];
    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();

    assert!(!consumer.is_empty());
}

#[test]
fn is_empty_partial_full_wrapping_buffer() {
    const SIZE: usize = 32;
    const DATA: usize = 24;

    let (mut producer, mut consumer) = Buffer::<SIZE, u8>::new();
    let mut buf = [0u8; DATA];
    let mut buf_read = [0u8; DATA];
    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();
    consumer.read(&mut buf_read).unwrap();

    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();

    assert!(!consumer.is_empty());
}

#[test]
fn is_empty_full_wrapping_buffer() {
    const SIZE: usize = 32;
    const DATA: usize = 24;

    let (mut producer, mut consumer) = Buffer::<SIZE, u8>::new();
    let mut buf = [0u8; DATA];
    let mut buf_read = [0u8; DATA];
    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();
    consumer.read(&mut buf_read).unwrap();

    super::get_random_values(&mut buf);
    producer.write(&buf).unwrap();
    super::get_random_values(&mut buf[..SIZE - DATA]);
    producer.write(&buf[..SIZE - DATA]).unwrap();

    assert!(!consumer.is_empty());
}

#[test]
fn is_abandoned_existing() {
    const SIZE: usize = 32;
    let (_producer, consumer) = Buffer::<SIZE, u8>::new();

    assert!(!consumer.is_abandoned());
}

#[test]
fn is_abandoned_dropped() {
    const SIZE: usize = 32;
    let (producer, consumer) = Buffer::<SIZE, u8>::new();
    drop(producer);

    assert!(consumer.is_abandoned());
}
