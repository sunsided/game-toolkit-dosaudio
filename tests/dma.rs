//! Public memory and 8237 address, count, page, and transfer semantics.
#![cfg(feature = "sb")]
use game_toolkit_dosaudio::{
    ConventionalMemory, DmaChannel, DmaController, DmaDirection, DmaRequest, DmaUnit,
};

fn request(channel: u8) -> DmaRequest {
    DmaRequest {
        channel: DmaChannel::new(channel).unwrap(),
        direction: DmaDirection::MemoryToDevice,
    }
}

fn program(dma: &mut DmaController, channel: u8, page: u8, address: u16, count: u16, mode: u8) {
    let high = channel >= 4;
    let local = channel & 3;
    let register = if high {
        0xc0 + u16::from(local) * 4
    } else {
        u16::from(local) * 2
    };
    let step = if high { 2 } else { 1 };
    let clear = if high { 0xd8 } else { 0x0c };
    let page_port = match channel {
        0 => 0x87,
        1 => 0x83,
        2 => 0x81,
        3 => 0x82,
        5 => 0x8b,
        6 => 0x89,
        7 => 0x8a,
        _ => unreachable!(),
    };
    dma.outb(clear, 0);
    for value in address.to_le_bytes() {
        dma.outb(register, value);
    }
    for value in count.to_le_bytes() {
        dma.outb(register + step, value);
    }
    dma.outb(page_port, page);
    dma.outb(if high { 0xd6 } else { 0x0b }, mode | local);
    dma.outb(if high { 0xd4 } else { 0x0a }, local);
}

fn current(dma: &mut DmaController, channel: u8) -> (u16, u16) {
    let high = channel >= 4;
    let register = if high {
        0xc0 + u16::from(channel & 3) * 4
    } else {
        u16::from(channel) * 2
    };
    let step = if high { 2 } else { 1 };
    dma.outb(if high { 0xd8 } else { 0x0c }, 0);
    let address = u16::from_le_bytes([dma.inb(register), dma.inb(register)]);
    let count = u16::from_le_bytes([dma.inb(register + step), dma.inb(register + step)]);
    (address, count)
}

#[test]
fn shared_memory_checks_entire_ranges_before_mutating() {
    let memory = ConventionalMemory::new();
    let shared = memory.clone();
    memory.write(0xffffe, &[4, 5]).unwrap();
    let mut actual = [0; 2];
    shared.read(0xffffe, &mut actual).unwrap();
    assert_eq!(actual, [4, 5]);
    shared.write_byte(0xfffff, 6).unwrap();
    assert_eq!(memory.read_byte(0xfffff).unwrap(), 6);
    assert!(memory.write(0xfffff, &[8, 9]).is_err());
    assert_eq!(memory.read_byte(0xfffff).unwrap(), 6);
    let mut unchanged = [12; 2];
    assert!(memory.read(0xfffff, &mut unchanged).is_err());
    assert_eq!(unchanged, [12; 2]);
    assert!(memory.read_byte(0x100000).is_err());
    assert!(memory.write_byte(u32::MAX, 0).is_err());
    assert!(memory.read(u32::MAX, &mut []).is_err());
    memory.write(0x100000, &[]).unwrap();
}

#[test]
fn byte_channels_wrap_pages_and_stop_at_terminal_count_without_masking() {
    let memory = ConventionalMemory::new();
    memory.write_byte(0x2ffff, 0xa1).unwrap();
    memory.write_byte(0x20000, 0xb2).unwrap();
    let mut dma = DmaController::new();
    program(&mut dma, 1, 2, 0xffff, 1, 0x48);
    assert_eq!(
        dma.transfer(request(1), &memory).unwrap(),
        Some(DmaUnit::Byte(0xa1))
    );
    assert_eq!(current(&mut dma, 1), (0, 0));
    assert_eq!(
        dma.transfer(request(1), &memory).unwrap(),
        Some(DmaUnit::Byte(0xb2))
    );
    assert_eq!(current(&mut dma, 1), (1, 0xffff));
    assert_eq!(dma.transfer(request(1), &memory).unwrap(), None);
    assert_eq!(dma.inb(8), 2);
    assert_eq!(dma.inb(8), 0);
    // Reprogramming only the count reactivates the exhausted, still-unmasked channel.
    memory.write_byte(0x20001, 0xc3).unwrap();
    dma.outb(0x0c, 0);
    dma.outb(3, 0);
    dma.outb(3, 0);
    assert_eq!(
        dma.transfer(request(1), &memory).unwrap(),
        Some(DmaUnit::Byte(0xc3))
    );
}

#[test]
fn every_data_channel_uses_its_at_page_register_and_bus_width() {
    let memory = ConventionalMemory::new();
    let mut dma = DmaController::new();
    for channel in [0, 1, 2, 3, 5, 6, 7] {
        let word = channel >= 4;
        let physical = if word { 0x20060 } else { 0x30030 };
        memory.write(physical, &[channel, 0xab]).unwrap();
        program(&mut dma, channel, 3, 0x30, 0, 0x48);
        let expected = if word {
            DmaUnit::Word(0xab00 | u16::from(channel))
        } else {
            DmaUnit::Byte(channel)
        };
        assert_eq!(
            dma.transfer(request(channel), &memory).unwrap(),
            Some(expected)
        );
    }
    assert!(DmaChannel::new(4).is_none());
    dma.outb(0xc0, 0x55);
    dma.outb(0xc0, 0x66);
    dma.outb(0xd8, 0);
    assert_eq!(dma.inb(0xc0), 0xff);
    assert_eq!(dma.inb(0xc0), 0xff);
}

#[test]
fn word_channels_ignore_page_bit_zero_and_wrap_at_128k() {
    let memory = ConventionalMemory::new();
    memory.write(0x3fffe, &[0x34, 0x12]).unwrap();
    memory.write(0x20000, &[0x78, 0x56]).unwrap();
    let mut dma = DmaController::new();
    program(&mut dma, 5, 3, 0xffff, 1, 0x48);
    assert_eq!(
        dma.transfer(request(5), &memory).unwrap(),
        Some(DmaUnit::Word(0x1234))
    );
    assert_eq!(current(&mut dma, 5), (0, 0));
    assert_eq!(
        dma.transfer(request(5), &memory).unwrap(),
        Some(DmaUnit::Word(0x5678))
    );
    assert_eq!(dma.inb(0xd0), 2);
    assert_eq!(dma.inb(8), 0);
    assert_eq!(dma.inb(0xd0), 0);
}

#[test]
fn decrement_and_auto_init_reload_the_programmed_block() {
    let memory = ConventionalMemory::new();
    memory.write_byte(0x50000, 0x11).unwrap();
    memory.write_byte(0x5ffff, 0x22).unwrap();
    let mut dma = DmaController::new();
    program(&mut dma, 3, 5, 0, 1, 0x78);
    for _ in 0..3 {
        assert_eq!(
            dma.transfer(request(3), &memory).unwrap(),
            Some(DmaUnit::Byte(0x11))
        );
        assert_eq!(
            dma.transfer(request(3), &memory).unwrap(),
            Some(DmaUnit::Byte(0x22))
        );
        assert_eq!(current(&mut dma, 3), (0, 1));
        assert_eq!(dma.inb(8) & 8, 8);
    }
}

#[test]
fn ffff_count_transfers_exactly_65536_units() {
    let memory = ConventionalMemory::new();
    let mut dma = DmaController::new();
    program(&mut dma, 2, 0, 0, 0xffff, 0x48);
    for _ in 0..65535 {
        assert_eq!(
            dma.transfer(request(2), &memory).unwrap(),
            Some(DmaUnit::Byte(0))
        );
    }
    assert_eq!(dma.inb(8), 0);
    assert_eq!(
        dma.transfer(request(2), &memory).unwrap(),
        Some(DmaUnit::Byte(0))
    );
    assert_eq!(dma.inb(8), 4);
    assert_eq!(dma.transfer(request(2), &memory).unwrap(), None);
}

#[test]
fn flip_flops_are_shared_within_chips_and_independent_between_chips() {
    let mut dma = DmaController::new();
    dma.outb(0, 0x34); // Low address byte on chip zero.
    dma.outb(0xc4, 0x78); // Independent low address byte on chip one.
    dma.outb(3, 0x12); // Shared chip-zero flip-flop: high count byte on channel one.
    dma.outb(0xc6, 0x56); // Shared chip-one flip-flop: high count byte on channel five.
    assert_eq!(current(&mut dma, 0), (0x34, 0));
    assert_eq!(current(&mut dma, 1), (0, 0x1200));
    assert_eq!(current(&mut dma, 5), (0x78, 0x5600));
    dma.outb(0, 0x9a); // Reading address/count also changes the shared flip-flop.
    assert_eq!(dma.inb(0), 0);
    dma.outb(0x0c, 0);
    assert_eq!(dma.inb(0), 0x9a);
}

#[test]
fn masks_disable_and_incompatible_modes_do_not_advance() {
    let memory = ConventionalMemory::new();
    let mut dma = DmaController::new();
    assert_eq!(dma.transfer(request(1), &memory).unwrap(), None);
    program(&mut dma, 1, 0, 0x1234, 3, 0x48);
    dma.outb(0x0a, 5);
    assert_eq!(dma.transfer(request(1), &memory).unwrap(), None);
    dma.outb(0x0e, 0);
    dma.outb(8, 4);
    assert_eq!(dma.transfer(request(1), &memory).unwrap(), None);
    dma.outb(8, 0);
    for mode in [0x41, 0x45, 0xc9] {
        dma.outb(0x0b, mode);
        assert_eq!(dma.transfer(request(1), &memory).unwrap(), None);
    }
    dma.outb(0x0b, 0x49);
    let input = DmaRequest {
        direction: DmaDirection::DeviceToMemory,
        ..request(1)
    };
    assert_eq!(dma.transfer(input, &memory).unwrap(), None);
    assert_eq!(current(&mut dma, 1), (0x1234, 3));
    dma.outb(0x0f, 0b0010);
    assert_eq!(dma.transfer(request(1), &memory).unwrap(), None);
    dma.outb(0x0f, 0);
    assert_eq!(
        dma.transfer(request(1), &memory).unwrap(),
        Some(DmaUnit::Byte(0))
    );
}

#[test]
fn master_clear_masks_only_its_chip_and_clears_status_and_flip_flop() {
    let memory = ConventionalMemory::new();
    let mut dma = DmaController::new();
    program(&mut dma, 1, 0, 0, 0, 0x58);
    program(&mut dma, 5, 0, 0, 0, 0x58);
    dma.transfer(request(1), &memory).unwrap();
    dma.outb(9, 5); // Software request for channel one.
    dma.outb(0, 0x55); // Flip-flop now selects the high byte.
    dma.outb(0x0d, 0);
    assert_eq!(dma.inb(8), 0);
    assert_eq!(dma.inb(0), 0x55);
    assert_eq!(dma.transfer(request(1), &memory).unwrap(), None);
    assert_eq!(
        dma.transfer(request(5), &memory).unwrap(),
        Some(DmaUnit::Word(0))
    );
    dma.outb(0x0e, 0);
    assert_eq!(
        dma.transfer(request(1), &memory).unwrap(),
        Some(DmaUnit::Byte(0))
    );
}

#[test]
fn memory_errors_preserve_dma_state_and_can_be_retried() {
    let memory = ConventionalMemory::new();
    let mut dma = DmaController::new();
    for channel in [1, 5] {
        program(&mut dma, channel, 0x10, 0x1234, 7, 0x48);
        assert!(dma.transfer(request(channel), &memory).is_err());
        assert_eq!(current(&mut dma, channel), (0x1234, 7));
        assert_eq!(dma.inb(if channel >= 4 { 0xd0 } else { 8 }), 0);
        dma.outb(if channel >= 4 { 0x8b } else { 0x83 }, 0);
        assert!(dma.transfer(request(channel), &memory).unwrap().is_some());
        assert_eq!(current(&mut dma, channel), (0x1235, 6));
    }
}

#[test]
fn advertised_ports_are_precise_and_unknown_ports_are_open_bus() {
    let mut dma = DmaController::new();
    for port in [0, 0x0f, 0x81, 0x87, 0x8b, 0xc0, 0xde] {
        assert!(dma.ports().iter().any(|range| range.contains(port)));
    }
    for port in [0x61, 0x80, 0x84, 0x88, 0xc1, 0xdd, 0xdf] {
        assert!(!dma.ports().iter().any(|range| range.contains(port)));
        dma.outb(port, 0);
        assert_eq!(dma.inb(port), 0xff);
    }
}
