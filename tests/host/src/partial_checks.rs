//! Contract checks for addressed physical-row writes (LD-29Y03 §6-6-2).
use crate::{
    nice_view::FrameBuffer,
    tests::{Fault, panel},
};
use embedded_graphics::{pixelcolor::BinaryColor, prelude::*};
use std::{
    future::Future,
    task::{Context, Poll},
};

#[test]
fn nonconsecutive_physical_rows_keep_addresses_rotation_and_dummy_bytes() {
    let mut fb = FrameBuffer::new();
    fb.draw_iter([
        Pixel(Point::new(0, 0), BinaryColor::Off),
        Pixel(Point::new(17, 151), BinaryColor::Off),
        Pixel(Point::new(67, 159), BinaryColor::Off),
        // This portrait X is not selected, regardless of its portrait Y.
        Pixel(Point::new(1, 17), BinaryColor::Off),
    ])
    .unwrap();
    let bytes = fb.frame_rows(true, (1 << 67) | (1 << 17) | 1 | (1 << 127));
    assert_eq!(bytes.len(), 68); // command + 3 * (address + 20 pixels + dummy) + dummy
    assert_eq!(bytes[0], 3);
    assert_eq!(bytes[1], 1);
    assert_eq!(&bytes[2..21], &[255; 19]);
    assert_eq!(bytes[21], 0x7f); // portrait Y=0 -> physical pixel 159, LSB-first
    assert_eq!(bytes[22], 0);
    assert_eq!(bytes[23], 18);
    assert_eq!(bytes[24], 255);
    assert_eq!(bytes[25], 0xfe); // portrait Y=151 -> physical pixel 8
    assert_eq!(&bytes[26..44], &[255; 18]);
    assert_eq!(bytes[44], 0);
    assert_eq!(bytes[45], 68);
    assert_eq!(bytes[46], 0xfe); // portrait Y=159 -> physical pixel 0
    assert_eq!(&bytes[47..66], &[255; 19]);
    assert_eq!(&bytes[66..], &[0, 0]);
}

#[test]
fn selected_row_count_controls_packet_length_and_masks_high_bits() {
    let mut fb = FrameBuffer::new();
    for (rows, addresses) in [
        (1u128 << 67, vec![68]),
        ((1 << 66) | (1 << 2) | (1 << 68), vec![3, 67]),
        (u128::MAX, (1..=68).collect()),
    ] {
        let bytes = fb.frame_rows(false, rows);
        assert_eq!(bytes.len(), 2 + 22 * addresses.len());
        assert_eq!(bytes[0], 1);
        assert_eq!(*bytes.last().unwrap(), 0);
        for (record, address) in bytes[1..bytes.len() - 1]
            .as_chunks::<22>()
            .0
            .iter()
            .zip(addresses)
        {
            assert_eq!(record[0], address);
            assert_eq!(&record[1..21], &[255; 20]);
            assert_eq!(record[21], 0);
        }
    }
}

#[test]
fn no_valid_rows_use_hold_with_requested_polarity() {
    let mut fb = FrameBuffer::new();
    let (mut p, trace) = panel();
    for rows in [0, 1 << 68, 1 << 127, u128::MAX << 68] {
        for (polarity, command) in [(false, 0), (true, 2)] {
            assert_eq!(fb.frame_rows(polarity, rows), &[command, 0]);
            futures::executor::block_on(p.flush_rows(polarity, rows)).unwrap();
            assert_eq!(trace.borrow().writes.last().unwrap(), &[command, 0]);
            assert!(!trace.borrow().cs);
        }
    }
    assert_eq!(trace.borrow().calls, 16); // every HOLD also flushes the SPI bus
}

#[test]
fn partial_then_full_serialization_restores_all_addresses_and_dummies() {
    let mut fb = FrameBuffer::new();
    fb.clear(BinaryColor::Off).unwrap();
    fb.draw_iter([Pixel(Point::new(67, 159), BinaryColor::On)])
        .unwrap();
    assert_eq!(fb.frame_rows(true, 1 << 67).len(), 24);
    assert_eq!(fb.frame_rows(false, 0), &[0, 0]);
    for polarity in [false, true] {
        let bytes = fb.frame(polarity);
        assert_eq!(bytes.len(), 1498);
        assert_eq!(bytes[0], if polarity { 3 } else { 1 });
        for row in 0..68 {
            let record = &bytes[1 + row * 22..1 + (row + 1) * 22];
            assert_eq!(record[0], row as u8 + 1);
            assert_eq!(record[1], if row == 67 { 1 } else { 0 });
            assert_eq!(&record[2..], &[0; 20]);
        }
        assert_eq!(bytes[1497], 0);
    }
}

#[test]
fn partial_flush_preserves_select_delays_supply_and_full_flush_api() {
    let (mut p, trace) = panel();
    futures::executor::block_on(async {
        p.flush_rows(true, (1 << 67) | 1).await.unwrap();
        p.flush(false).await.unwrap();
    });
    let t = trace.borrow();
    assert_eq!(t.writes[0].len(), 46);
    assert_eq!(t.writes[0][0], 3);
    assert_eq!(t.writes[0][1], 1);
    assert_eq!(t.writes[0][23], 68);
    assert_eq!(&t.writes[0][44..], &[0, 0]);
    assert_eq!(t.writes[1].len(), 1498);
    assert_eq!(t.writes[1][0], 1);
    assert_eq!(t.writes[1][1475], 68);
    assert_eq!(t.writes[1][1497], 0);
    assert_eq!(t.calls, 4);
    assert_eq!(t.edges, [false, true, false, true, false]);
    assert_eq!(t.delays, [6000, 2000, 6000, 6000, 2000, 6000]);
    assert_eq!(t.power_levels, [true]);
    assert!(t.power && !t.cs);
}

#[test]
fn cancelling_pending_partial_write_releases_select_and_allows_full_retry() {
    let (mut p, trace) = panel();
    trace.borrow_mut().pending = true;
    let mut future = Box::pin(p.flush_rows(false, (1 << 67) | 1));
    let mut cx = Context::from_waker(futures::task::noop_waker_ref());
    assert_eq!(future.as_mut().poll(&mut cx), Poll::Pending);
    assert!(trace.borrow().cs);
    assert_eq!(trace.borrow().writes[0].len(), 46);
    drop(future);
    assert!(!trace.borrow().cs);
    trace.borrow_mut().pending = false;
    futures::executor::block_on(p.flush(true)).unwrap();
    assert_eq!(trace.borrow().writes.last().unwrap().len(), 1498);
    assert!(!trace.borrow().cs);
}

#[test]
fn partial_write_and_flush_errors_release_select_and_preserve_pixels_for_retry() {
    for fail in [1, 2] {
        let (mut p, trace) = panel();
        p.framebuffer()
            .draw_iter([Pixel(Point::new(67, 159), BinaryColor::Off)])
            .unwrap();
        trace.borrow_mut().fail = Some(fail);
        assert_eq!(
            futures::executor::block_on(p.flush_rows(false, 1 << 67)),
            Err(Fault::Bus)
        );
        assert!(!trace.borrow().cs);
        trace.borrow_mut().fail = None;
        futures::executor::block_on(p.flush_rows(true, 1 << 67)).unwrap();
        {
            let t = trace.borrow();
            let bytes = t.writes.last().unwrap();
            assert_eq!(bytes.len(), 24);
            assert_eq!(&bytes[..3], &[3, 68, 0xfe]);
            assert_eq!(&bytes[3..22], &[255; 19]);
            assert_eq!(&bytes[22..], &[0, 0]);
            assert!(!t.cs);
        }
        futures::executor::block_on(p.flush(false)).unwrap();
        let t = trace.borrow();
        assert_eq!(t.writes.last().unwrap().len(), 1498);
        assert_eq!(t.writes.last().unwrap()[1476], 0xfe);
        // Assert transmitted bytes, not panel acknowledgement: SPI is write-only.
    }
}

#[test]
fn truncated_partial_transfers_can_be_retried_without_stale_packet_bytes() {
    for prefix in [0, 1, 22, 23, 45, 46] {
        let (mut p, trace) = panel();
        trace.borrow_mut().fail_after_bytes = Some(prefix);
        assert_eq!(
            futures::executor::block_on(p.flush_rows(true, (1 << 67) | 1)),
            Err(Fault::Bus)
        );
        assert!(!trace.borrow().cs);
        trace.borrow_mut().fail_after_bytes = None;
        futures::executor::block_on(p.flush_rows(false, 1 << 17)).unwrap();
        let t = trace.borrow();
        let bytes = t.writes.last().unwrap();
        assert_eq!(bytes.len(), 24);
        assert_eq!(&bytes[..2], &[1, 18]);
        assert_eq!(&bytes[2..22], &[255; 20]);
        assert_eq!(&bytes[22..], &[0, 0]);
        assert!(!t.cs);
    }
}
