#![no_main]
use game_toolkit_dosaudio_fuzz::playback;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    if data.len() > 256 {
        return;
    }
    let Some((&chunk, program)) = data.split_first() else {
        return;
    };
    assert_eq!(
        playback(program, 64),
        playback(program, usize::from(chunk % 64) + 1)
    );
});
