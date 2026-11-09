"""Generate original, reproducible corpus inputs; no firmware or external music."""
from pathlib import Path
import struct

root = Path(__file__).resolve().parent / "corpus"


def seed(target, name, data):
    directory = root / target
    directory.mkdir(parents=True, exist_ok=True)
    (directory / name).write_bytes(data)


# First byte selects a parser. Every fixture below is authored here.
imf = bytes([0x20, 1, 1, 0, 0xB0, 0x31, 10, 0, 0xB0, 0x11, 0, 0])
seed("music_formats", "imf0", bytes([0]) + imf)
seed("music_formats", "imf1", bytes([1]) + struct.pack("<H", len(imf)) + imf)
dro = bytes([0x20, 1, 0, 9, 0xB0, 0x31, 0, 9, 0xB0, 0x11])
seed("music_formats", "dro", bytes([2]) + b"DBRAWOPL" + struct.pack("<HHIII", 0, 1, 20, len(dro), 0) + dro)
vgm = bytearray(0x100)
vgm[:4] = b"Vgm "
struct.pack_into("<I", vgm, 8, 0x151)
struct.pack_into("<I", vgm, 0x34, 0x100 - 0x34)
struct.pack_into("<I", vgm, 0x50, 3_579_545)
vgm += bytes([0x5A, 0x20, 1, 0x62, 0x5A, 0xB0, 0x31, 0x62, 0x66])
struct.pack_into("<I", vgm, 4, len(vgm) - 4)
seed("music_formats", "vgm", bytes([3]) + vgm)
track = bytes([0, 0xC0, 40, 0, 0x90, 60, 100, 96, 0x80, 60, 0, 0, 0xFF, 0x2F, 0])
smf = b"MThd" + struct.pack(">IHHH", 6, 0, 1, 96) + b"MTrk" + struct.pack(">I", len(track)) + track
seed("music_formats", "smf", bytes([4]) + smf)
seed("music_formats", "play", bytes([5]) + b"T137 O4 L8 C D E. P8 <G >C")
seed("music_formats", "blaster", bytes([6]) + b"A220 I5 D1 H5 T6")
seed("music_formats", "midi", bytes([6, 0x90, 60, 100]))
seed("music_formats", "sysex", bytes([7]) + struct.pack("<IIQQ", 0, 4, 0, 1) + bytes([0xF0, 0x7D, 1, 0xF7]))
seed("music_formats", "sysex-overflow", bytes([7]) + struct.pack("<IIQQ", 0xFFFFFFFF, 0xFFFFFFFF, 0, 1))
sbi = bytearray(52)
sbi[:4] = b"SBI\x1a"
sbi[4:9] = b"Tone\0"
seed("opl_banks", "sbi", bytes([0]) + sbi)
op2 = bytearray(b"#OPL_II#") + bytearray(175 * 36)
seed("opl_banks", "op2", bytes([1]) + op2)
# v1, one melodic bank, two-op zero patches.
wopl = b"WOPL3-BANK\0" + struct.pack("<H", 1) + struct.pack(">HH", 1, 0) + bytes(2) + bytes(128 * 62)
seed("opl_banks", "wopl", bytes([2]) + wopl)
bnk = bytearray(28 + 12 + 30)
bnk[:8] = b"\x01\x00ADLIB-"
struct.pack_into("<HHII", bnk, 8, 1, 1, 28, 40)
bnk[30] = 1
bnk[31:36] = b"Tone\0"
seed("opl_banks", "bnk", bytes([3]) + bnk)
seed("playback", "switch-pause-fade-stop", bytes([12, 1, 0, 64, 2, 0, 32, 0, 0, 8, 3, 0, 64, 5, 128, 64, 8, 100, 64, 7, 0, 64]))
seed("playback", "loop", bytes([0]) + bytes([0, 0, 64, 1, 0, 64]) * 8)
seed("midi_uart", "running-status", bytes([13, 0x90, 60, 100, 64, 0xF8, 100, 60, 0]))
seed("midi_uart", "sysex-reset", bytes([3, 0xF0, 0x7D, 1, 0xF8, 2, 0xF7, 0xFF, 0x90, 64, 100]))
seed("midi_uart", "aborted-sysex", bytes([63, 0xF0, 0x7D, 1, 0x90, 60, 100]))
print("Generated original seeds for music_formats, opl_banks, playback and midi_uart.")
