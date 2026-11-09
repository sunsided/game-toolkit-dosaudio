#![no_main]
use game_toolkit_dosaudio_fuzz::midi_uart;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    if data.len() > 512 {
        return;
    }
    let Some((&chunk, bytes)) = data.split_first() else {
        return;
    };
    assert_eq!(
        midi_uart(bytes, 2304),
        midi_uart(bytes, usize::from(chunk % 64) + 1)
    );
});
