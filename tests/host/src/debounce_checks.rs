#[cfg(test)]
mod tests {
    use embassy_time::{Duration, MockDriver};
    use rmk::{
        debounce::{default_debouncer::DefaultDebouncer, DebounceState, DebouncerTrait},
        matrix::KeyState,
    };

    // Hardware expectation comes from the actual ZMK v0.3 matrix binding:
    // debounce-press-ms and debounce-release-ms both default to 5ms; the
    // user's urchin.conf has no enabled override. Exercise the real RMK
    // debouncer, whose threshold is compiled from the project's TOML.
    #[test]
    fn stable_press_and_release_are_recognized_after_five_ms() {
        let _lock = crate::tests::EVENT_LOCK.lock().unwrap();
        let clock = MockDriver::get();
        clock.reset();
        clock.advance(Duration::from_millis(1));
        let mut debounce = DefaultDebouncer::<1, 1>::new();
        let mut key = KeyState::new();
        for pressed in [true, false] {
            assert!(matches!(
                debounce.detect_change_with_debounce(0, 0, pressed, &key),
                DebounceState::InProgress
            ));
            clock.advance(Duration::from_millis(4));
            assert!(matches!(
                debounce.detect_change_with_debounce(0, 0, pressed, &key),
                DebounceState::InProgress
            ));
            clock.advance(Duration::from_millis(1));
            assert!(
                matches!(
                    debounce.detect_change_with_debounce(0, 0, pressed, &key),
                    DebounceState::Debounced
                ),
                "stable input should reach hold-tap processing after 5ms, as with ZMK"
            );
            key.toggle_pressed();
            assert!(matches!(
                debounce.detect_change_with_debounce(0, 0, pressed, &key),
                DebounceState::Ignored
            ));
            clock.advance(Duration::from_millis(10));
        }
    }
}
