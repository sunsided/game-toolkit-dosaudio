#![no_main]
use game_toolkit_dosaudio::OplBank;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 65_536 {
        return;
    }
    let Some((&format, bytes)) = data.split_first() else {
        return;
    };
    let bank = match format % 4 {
        0 => OplBank::from_sbi(bytes),
        1 => OplBank::from_op2(bytes),
        2 => OplBank::from_wopl(bytes),
        _ => OplBank::from_bnk(bytes),
    };
    if let Ok(bank) = bank {
        assert!(
            bank.entries()
                .windows(2)
                .all(|pair| pair[0].key < pair[1].key)
        );
        let mut validated = OplBank::new();
        for entry in bank.entries() {
            assert_eq!(bank.get(entry.key), Some(entry));
            validated
                .insert(entry.clone())
                .expect("a parsed entry must be valid");
        }
        assert_eq!(bank.entries(), validated.entries());
    }
});
