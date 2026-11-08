// Copyright (C) 2021-2026 Geoff Hill <geoff@geoffhill.org>
// Copyright (C) 2003-2026 Dean Beeler, Jerome Fisher, Sergey V. Mikayev
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, either version 2.1 of the License, or (at
// your option) any later version. Read COPYING.LESSER.txt for details.

use core::fmt::{Display, Formatter};
use core::{error, fmt};
use std::boxed::Box;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Msg {
    NoteOn { note: u8, velocity: u8, channel: u8 },
    NoteOff { note: u8, channel: u8 },
    ControlChange { cc: u8, value: u8, channel: u8 },
    ProgramChange { program: u8, channel: u8 },
    PitchBend { value: u16, channel: u8 },
    Sysex(SharedSysex),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MelodicInstrument {
    AcouPiano1 = 0,
    AcouPiano2 = 1,
    AcouPiano3 = 2,
    ElecPiano1 = 3,
    ElecPiano2 = 4,
    ElecPiano3 = 5,
    ElecPiano4 = 6,
    Honkytonk = 7,
    ElecOrg1 = 8,
    ElecOrg2 = 9,
    ElecOrg3 = 10,
    ElecOrg4 = 11,
    PipeOrg1 = 12,
    PipeOrg2 = 13,
    PipeOrg3 = 14,
    Accordion = 15,
    Harpsi1 = 16,
    Harpsi2 = 17,
    Harpsi3 = 18,
    Clavi1 = 19,
    Clavi2 = 20,
    Clavi3 = 21,
    Celesta1 = 22,
    Celesta2 = 23,
    SynBrass1 = 24,
    SynBrass2 = 25,
    SynBrass3 = 26,
    SynBrass4 = 27,
    SynBass1 = 28,
    SynBass2 = 29,
    SynBass3 = 30,
    SynBass4 = 31,
    Fantasy = 32,
    HarmoPan = 33,
    Chorale = 34,
    Glasses = 35,
    Soundtrack = 36,
    Atmosphere = 37,
    WarmBell = 38,
    FunnyVox = 39,
    EchoBell = 40,
    IceRain = 41,
    Oboe2001 = 42,
    EchoPan = 43,
    DoctorSolo = 44,
    Schooldaze = 45,
    Bellsinger = 46,
    SquareWave = 47,
    StrSect1 = 48,
    StrSect2 = 49,
    StrSect3 = 50,
    Pizzicato = 51,
    Violin1 = 52,
    Violin2 = 53,
    Cello1 = 54,
    Cello2 = 55,
    Contrabass = 56,
    Harp1 = 57,
    Harp2 = 58,
    Guitar1 = 59,
    Guitar2 = 60,
    ElecGtr1 = 61,
    ElecGtr2 = 62,
    Sitar = 63,
    AcouBass1 = 64,
    AcouBass2 = 65,
    ElecBass1 = 66,
    ElecBass2 = 67,
    SlapBass1 = 68,
    SlapBass2 = 69,
    Fretless1 = 70,
    Fretless2 = 71,
    Flute1 = 72,
    Flute2 = 73,
    Piccolo1 = 74,
    Piccolo2 = 75,
    Recorder = 76,
    PanPipes = 77,
    Sax1 = 78,
    Sax2 = 79,
    Sax3 = 80,
    Sax4 = 81,
    Clarinet1 = 82,
    Clarinet2 = 83,
    Oboe = 84,
    EnglHorn = 85,
    Bassoon = 86,
    Harmonica = 87,
    Trumpet1 = 88,
    Trumpet2 = 89,
    Trombone1 = 90,
    Trombone2 = 91,
    FrHorn1 = 92,
    FrHorn2 = 93,
    Tuba = 94,
    BrsSect1 = 95,
    BrsSect2 = 96,
    Vibe1 = 97,
    Vibe2 = 98,
    SynMallet = 99,
    Windbell = 100,
    Glock = 101,
    TubeBell = 102,
    Xylophone = 103,
    Marimba = 104,
    Koto = 105,
    Sho = 106,
    Shakuhachi = 107,
    Whistle1 = 108,
    Whistle2 = 109,
    Bottleblow = 110,
    Breathpipe = 111,
    Timpani = 112,
    MelodicTom = 113,
    DeepSnare = 114,
    ElecPerc1 = 115,
    ElecPerc2 = 116,
    Taiko = 117,
    TaikoRim = 118,
    Cymbal = 119,
    Castanets = 120,
    Triangle = 121,
    OrcheHit = 122,
    Telephone = 123,
    BirdTweet = 124,
    OneNoteJam = 125,
    WaterBells = 126,
    JungleTune = 127,
}

impl MelodicInstrument {
    pub fn from_program(program: u8) -> Option<Self> {
        match program {
            0 => Some(Self::AcouPiano1),
            1 => Some(Self::AcouPiano2),
            2 => Some(Self::AcouPiano3),
            3 => Some(Self::ElecPiano1),
            4 => Some(Self::ElecPiano2),
            5 => Some(Self::ElecPiano3),
            6 => Some(Self::ElecPiano4),
            7 => Some(Self::Honkytonk),
            8 => Some(Self::ElecOrg1),
            9 => Some(Self::ElecOrg2),
            10 => Some(Self::ElecOrg3),
            11 => Some(Self::ElecOrg4),
            12 => Some(Self::PipeOrg1),
            13 => Some(Self::PipeOrg2),
            14 => Some(Self::PipeOrg3),
            15 => Some(Self::Accordion),
            16 => Some(Self::Harpsi1),
            17 => Some(Self::Harpsi2),
            18 => Some(Self::Harpsi3),
            19 => Some(Self::Clavi1),
            20 => Some(Self::Clavi2),
            21 => Some(Self::Clavi3),
            22 => Some(Self::Celesta1),
            23 => Some(Self::Celesta2),
            24 => Some(Self::SynBrass1),
            25 => Some(Self::SynBrass2),
            26 => Some(Self::SynBrass3),
            27 => Some(Self::SynBrass4),
            28 => Some(Self::SynBass1),
            29 => Some(Self::SynBass2),
            30 => Some(Self::SynBass3),
            31 => Some(Self::SynBass4),
            32 => Some(Self::Fantasy),
            33 => Some(Self::HarmoPan),
            34 => Some(Self::Chorale),
            35 => Some(Self::Glasses),
            36 => Some(Self::Soundtrack),
            37 => Some(Self::Atmosphere),
            38 => Some(Self::WarmBell),
            39 => Some(Self::FunnyVox),
            40 => Some(Self::EchoBell),
            41 => Some(Self::IceRain),
            42 => Some(Self::Oboe2001),
            43 => Some(Self::EchoPan),
            44 => Some(Self::DoctorSolo),
            45 => Some(Self::Schooldaze),
            46 => Some(Self::Bellsinger),
            47 => Some(Self::SquareWave),
            48 => Some(Self::StrSect1),
            49 => Some(Self::StrSect2),
            50 => Some(Self::StrSect3),
            51 => Some(Self::Pizzicato),
            52 => Some(Self::Violin1),
            53 => Some(Self::Violin2),
            54 => Some(Self::Cello1),
            55 => Some(Self::Cello2),
            56 => Some(Self::Contrabass),
            57 => Some(Self::Harp1),
            58 => Some(Self::Harp2),
            59 => Some(Self::Guitar1),
            60 => Some(Self::Guitar2),
            61 => Some(Self::ElecGtr1),
            62 => Some(Self::ElecGtr2),
            63 => Some(Self::Sitar),
            64 => Some(Self::AcouBass1),
            65 => Some(Self::AcouBass2),
            66 => Some(Self::ElecBass1),
            67 => Some(Self::ElecBass2),
            68 => Some(Self::SlapBass1),
            69 => Some(Self::SlapBass2),
            70 => Some(Self::Fretless1),
            71 => Some(Self::Fretless2),
            72 => Some(Self::Flute1),
            73 => Some(Self::Flute2),
            74 => Some(Self::Piccolo1),
            75 => Some(Self::Piccolo2),
            76 => Some(Self::Recorder),
            77 => Some(Self::PanPipes),
            78 => Some(Self::Sax1),
            79 => Some(Self::Sax2),
            80 => Some(Self::Sax3),
            81 => Some(Self::Sax4),
            82 => Some(Self::Clarinet1),
            83 => Some(Self::Clarinet2),
            84 => Some(Self::Oboe),
            85 => Some(Self::EnglHorn),
            86 => Some(Self::Bassoon),
            87 => Some(Self::Harmonica),
            88 => Some(Self::Trumpet1),
            89 => Some(Self::Trumpet2),
            90 => Some(Self::Trombone1),
            91 => Some(Self::Trombone2),
            92 => Some(Self::FrHorn1),
            93 => Some(Self::FrHorn2),
            94 => Some(Self::Tuba),
            95 => Some(Self::BrsSect1),
            96 => Some(Self::BrsSect2),
            97 => Some(Self::Vibe1),
            98 => Some(Self::Vibe2),
            99 => Some(Self::SynMallet),
            100 => Some(Self::Windbell),
            101 => Some(Self::Glock),
            102 => Some(Self::TubeBell),
            103 => Some(Self::Xylophone),
            104 => Some(Self::Marimba),
            105 => Some(Self::Koto),
            106 => Some(Self::Sho),
            107 => Some(Self::Shakuhachi),
            108 => Some(Self::Whistle1),
            109 => Some(Self::Whistle2),
            110 => Some(Self::Bottleblow),
            111 => Some(Self::Breathpipe),
            112 => Some(Self::Timpani),
            113 => Some(Self::MelodicTom),
            114 => Some(Self::DeepSnare),
            115 => Some(Self::ElecPerc1),
            116 => Some(Self::ElecPerc2),
            117 => Some(Self::Taiko),
            118 => Some(Self::TaikoRim),
            119 => Some(Self::Cymbal),
            120 => Some(Self::Castanets),
            121 => Some(Self::Triangle),
            122 => Some(Self::OrcheHit),
            123 => Some(Self::Telephone),
            124 => Some(Self::BirdTweet),
            125 => Some(Self::OneNoteJam),
            126 => Some(Self::WaterBells),
            127 => Some(Self::JungleTune),
            _ => None,
        }
    }

    pub fn to_program(self) -> u8 {
        self as u8
    }
}

impl From<MelodicInstrument> for u8 {
    fn from(inst: MelodicInstrument) -> u8 {
        inst as u8
    }
}

impl Display for MelodicInstrument {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::AcouPiano1 => "Acoustic Piano 1",
            Self::AcouPiano2 => "Acoustic Piano 2",
            Self::AcouPiano3 => "Acoustic Piano 3",
            Self::ElecPiano1 => "Electronic Piano 1",
            Self::ElecPiano2 => "Electronic Piano 2",
            Self::ElecPiano3 => "Electronic Piano 3",
            Self::ElecPiano4 => "Electronic Piano 4",
            Self::Honkytonk => "Honkytonk",
            Self::ElecOrg1 => "Electronic Organ 1",
            Self::ElecOrg2 => "Electronic Organ 2",
            Self::ElecOrg3 => "Electronic Organ 3",
            Self::ElecOrg4 => "Electronic Organ 4",
            Self::PipeOrg1 => "Pipe Organ 1",
            Self::PipeOrg2 => "Pipe Organ 2",
            Self::PipeOrg3 => "Pipe Organ 3",
            Self::Accordion => "Accordion",
            Self::Harpsi1 => "Harpsichord 1",
            Self::Harpsi2 => "Harpsichord 2",
            Self::Harpsi3 => "Harpsichord 3",
            Self::Clavi1 => "Clavichord 1",
            Self::Clavi2 => "Clavichord 2",
            Self::Clavi3 => "Clavichord 3",
            Self::Celesta1 => "Celesta 1",
            Self::Celesta2 => "Celesta 2",
            Self::SynBrass1 => "Synthesized Brass 1",
            Self::SynBrass2 => "Synthesized Brass 2",
            Self::SynBrass3 => "Synthesized Brass 3",
            Self::SynBrass4 => "Synthesized Brass 4",
            Self::SynBass1 => "Synthesized Bass 1",
            Self::SynBass2 => "Synthesized Bass 2",
            Self::SynBass3 => "Synthesized Bass 3",
            Self::SynBass4 => "Synthesized Bass 4",
            Self::Fantasy => "Fantasy",
            Self::HarmoPan => "Harmony Pan",
            Self::Chorale => "Chorale",
            Self::Glasses => "Glasses",
            Self::Soundtrack => "Soundtrack",
            Self::Atmosphere => "Atmosphere",
            Self::WarmBell => "Warm Bell",
            Self::FunnyVox => "Funny Vox",
            Self::EchoBell => "Echo Bell",
            Self::IceRain => "Ice Rain",
            Self::Oboe2001 => "Oboe 2001",
            Self::EchoPan => "Echo Pan",
            Self::DoctorSolo => "Doctor Solo",
            Self::Schooldaze => "Schooldaze",
            Self::Bellsinger => "Bellsinger",
            Self::SquareWave => "Square Wave",
            Self::StrSect1 => "String Section 1",
            Self::StrSect2 => "String Section 2",
            Self::StrSect3 => "String Section 3",
            Self::Pizzicato => "Pizzicato",
            Self::Violin1 => "Violin 1",
            Self::Violin2 => "Violin 2",
            Self::Cello1 => "Cello 1",
            Self::Cello2 => "Cello 2",
            Self::Contrabass => "Contrabass",
            Self::Harp1 => "Harp 1",
            Self::Harp2 => "Harp 2",
            Self::Guitar1 => "Guitar 1",
            Self::Guitar2 => "Guitar 2",
            Self::ElecGtr1 => "Electric Guitar 1",
            Self::ElecGtr2 => "Electric Guitar 2",
            Self::Sitar => "Sitar",
            Self::AcouBass1 => "Acoustic Bass 1",
            Self::AcouBass2 => "Acoustic Bass 2",
            Self::ElecBass1 => "Electric Bass 1",
            Self::ElecBass2 => "Electric Bass 2",
            Self::SlapBass1 => "Slap Bass 1",
            Self::SlapBass2 => "Slap Bass 2",
            Self::Fretless1 => "Fretless Bass 1",
            Self::Fretless2 => "Fretless Bass 2",
            Self::Flute1 => "Flute 1",
            Self::Flute2 => "Flute 2",
            Self::Piccolo1 => "Piccolo 1",
            Self::Piccolo2 => "Piccolo 2",
            Self::Recorder => "Recorder",
            Self::PanPipes => "Pan Pipes",
            Self::Sax1 => "Sax 1",
            Self::Sax2 => "Sax 2",
            Self::Sax3 => "Sax 3",
            Self::Sax4 => "Sax 4",
            Self::Clarinet1 => "Clarinet 1",
            Self::Clarinet2 => "Clarinet 2",
            Self::Oboe => "Oboe",
            Self::EnglHorn => "English Horn",
            Self::Bassoon => "Bassoon",
            Self::Harmonica => "Harmonica",
            Self::Trumpet1 => "Trumpet 1",
            Self::Trumpet2 => "Trumpet 2",
            Self::Trombone1 => "Trombone 1",
            Self::Trombone2 => "Trombone 2",
            Self::FrHorn1 => "French Horn 1",
            Self::FrHorn2 => "French Horn 2",
            Self::Tuba => "Tuba",
            Self::BrsSect1 => "Brass Section 1",
            Self::BrsSect2 => "Brass Section 2",
            Self::Vibe1 => "Vibraphone 1",
            Self::Vibe2 => "Vibraphone 2",
            Self::SynMallet => "Synthetic Mallet",
            Self::Windbell => "Windbell",
            Self::Glock => "Glockenspiel",
            Self::TubeBell => "Tubular Bells",
            Self::Xylophone => "Xylophone",
            Self::Marimba => "Marimba",
            Self::Koto => "Koto",
            Self::Sho => "Sho",
            Self::Shakuhachi => "Shakuhachi",
            Self::Whistle1 => "Whistle 1",
            Self::Whistle2 => "Whistle 2",
            Self::Bottleblow => "Bottleblow",
            Self::Breathpipe => "Breathpipe",
            Self::Timpani => "Timpani",
            Self::MelodicTom => "Melodic Tom",
            Self::DeepSnare => "Deep Snare",
            Self::ElecPerc1 => "Electronic Percussion 1",
            Self::ElecPerc2 => "Electronic Percussion 2",
            Self::Taiko => "Taiko",
            Self::TaikoRim => "Taiko Rim",
            Self::Cymbal => "Cymbal",
            Self::Castanets => "Castanets",
            Self::Triangle => "Triangle",
            Self::OrcheHit => "Orchestra Hit",
            Self::Telephone => "Telephone",
            Self::BirdTweet => "Bird Tweet",
            Self::OneNoteJam => "One Note Jam",
            Self::WaterBells => "Water Bells",
            Self::JungleTune => "Jungle Tune",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum RhythmInstrument {
    AcousticBassB = 35,
    AcousticBassC = 36,
    RimShot = 37,
    AcousticSnare = 38,
    HandClap = 39,
    ElectronicSnare = 40,
    AcousticLowTomF = 41,
    ClosedHighHat = 42,
    AcousticLowTomG = 43,
    OpenHighHatAb = 44,
    AcousticMiddleTomA = 45,
    OpenHighHatBb = 46,
    AcousticMiddleTomB = 47,
    AcousticHighTomC = 48,
    CrashCymbal = 49,
    AcousticHighTomD = 50,
    RideCymbal = 51,
    Tambourine = 54,
    Cowbell = 56,
    HighBongo = 60,
    LowBongo = 61,
    MuteHighConga = 62,
    HighConga = 63,
    LowConga = 64,
    HighTimbale = 65,
    LowTimbale = 66,
    HighAgogo = 67,
    LowAgogo = 68,
    Cabasa = 69,
    Maracas = 70,
    ShortWhistle = 71,
    LongWhistle = 72,
    Quijada = 73,
    Claves = 75,
    Laughing = 76,
    Screaming = 77,
    Punch = 78,
    Heartbeat = 79,
    FootstepsAb = 80,
    FootstepsA = 81,
    Applause = 82,
    Creaking = 83,
    Door = 84,
    Scratch = 85,
    Windchime = 86,
    Engine = 87,
    CarStop = 88,
    CarPass = 89,
    Crash = 90,
    Siren = 91,
    Train = 92,
    Jet = 93,
    Helicopter = 94,
    Starship = 95,
    Pistol = 96,
    Machinegun = 97,
    Lasergun = 98,
    Explosion = 99,
    Dog = 100,
    Horse = 101,
    Birds = 102,
    Rain = 103,
    Thunder = 104,
    Wind = 105,
    Waves = 106,
    Stream = 107,
    Babble = 108,
}

impl RhythmInstrument {
    pub fn from_note(note: u8) -> Option<Self> {
        match note {
            35 => Some(Self::AcousticBassB),
            36 => Some(Self::AcousticBassC),
            37 => Some(Self::RimShot),
            38 => Some(Self::AcousticSnare),
            39 => Some(Self::HandClap),
            40 => Some(Self::ElectronicSnare),
            41 => Some(Self::AcousticLowTomF),
            42 => Some(Self::ClosedHighHat),
            43 => Some(Self::AcousticLowTomG),
            44 => Some(Self::OpenHighHatAb),
            45 => Some(Self::AcousticMiddleTomA),
            46 => Some(Self::OpenHighHatBb),
            47 => Some(Self::AcousticMiddleTomB),
            48 => Some(Self::AcousticHighTomC),
            49 => Some(Self::CrashCymbal),
            50 => Some(Self::AcousticHighTomD),
            51 => Some(Self::RideCymbal),
            52 | 53 => None,
            54 => Some(Self::Tambourine),
            55 => None,
            56 => Some(Self::Cowbell),
            57 | 58 | 59 => None,
            60 => Some(Self::HighBongo),
            61 => Some(Self::LowBongo),
            62 => Some(Self::MuteHighConga),
            63 => Some(Self::HighConga),
            64 => Some(Self::LowConga),
            65 => Some(Self::HighTimbale),
            66 => Some(Self::LowTimbale),
            67 => Some(Self::HighAgogo),
            68 => Some(Self::LowAgogo),
            69 => Some(Self::Cabasa),
            70 => Some(Self::Maracas),
            71 => Some(Self::ShortWhistle),
            72 => Some(Self::LongWhistle),
            73 => Some(Self::Quijada),
            74 => None,
            75 => Some(Self::Claves),
            76 => Some(Self::Laughing),
            77 => Some(Self::Screaming),
            78 => Some(Self::Punch),
            79 => Some(Self::Heartbeat),
            80 => Some(Self::FootstepsAb),
            81 => Some(Self::FootstepsA),
            82 => Some(Self::Applause),
            83 => Some(Self::Creaking),
            84 => Some(Self::Door),
            85 => Some(Self::Scratch),
            86 => Some(Self::Windchime),
            87 => Some(Self::Engine),
            88 => Some(Self::CarStop),
            89 => Some(Self::CarPass),
            90 => Some(Self::Crash),
            91 => Some(Self::Siren),
            92 => Some(Self::Train),
            93 => Some(Self::Jet),
            94 => Some(Self::Helicopter),
            95 => Some(Self::Starship),
            96 => Some(Self::Pistol),
            97 => Some(Self::Machinegun),
            98 => Some(Self::Lasergun),
            99 => Some(Self::Explosion),
            100 => Some(Self::Dog),
            101 => Some(Self::Horse),
            102 => Some(Self::Birds),
            103 => Some(Self::Rain),
            104 => Some(Self::Thunder),
            105 => Some(Self::Wind),
            106 => Some(Self::Waves),
            107 => Some(Self::Stream),
            108 => Some(Self::Babble),
            _ => None,
        }
    }

    pub fn to_note(self) -> u8 {
        self as u8
    }
}

impl From<RhythmInstrument> for u8 {
    fn from(inst: RhythmInstrument) -> u8 {
        inst as u8
    }
}

impl Display for RhythmInstrument {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::AcousticBassB => "Acoustic Bass Drum (B)",
            Self::AcousticBassC => "Acoustic Bass Drum (C)",
            Self::RimShot => "Rim Shot",
            Self::AcousticSnare => "Acoustic Snare Drum",
            Self::HandClap => "Hand Clap",
            Self::ElectronicSnare => "Electronic Snare Drum",
            Self::AcousticLowTomF => "Acoustic Low Tom (F)",
            Self::ClosedHighHat => "Closed High Hat",
            Self::AcousticLowTomG => "Acoustic Low Tom (G)",
            Self::OpenHighHatAb => "Open High Hat (A♭)",
            Self::AcousticMiddleTomA => "Acoustic Middle Tom (A)",
            Self::OpenHighHatBb => "Open High Hat (B♭)",
            Self::AcousticMiddleTomB => "Acoustic Middle Tom (B)",
            Self::AcousticHighTomC => "Acoustic High Tom (C)",
            Self::CrashCymbal => "Crash Cymbal",
            Self::AcousticHighTomD => "Acoustic High Tom (D)",
            Self::RideCymbal => "Ride Cymbal",
            Self::Tambourine => "Tambourine",
            Self::Cowbell => "Cowbell",
            Self::HighBongo => "High Bongo",
            Self::LowBongo => "Low Bongo",
            Self::MuteHighConga => "Mute High Conga",
            Self::HighConga => "High Conga",
            Self::LowConga => "Low Conga",
            Self::HighTimbale => "High Timbale",
            Self::LowTimbale => "Low Timbale",
            Self::HighAgogo => "High Agogo",
            Self::LowAgogo => "Low Agogo",
            Self::Cabasa => "Cabasa",
            Self::Maracas => "Maracas",
            Self::ShortWhistle => "Short Whistle",
            Self::LongWhistle => "Long Whistle",
            Self::Quijada => "Quijada",
            Self::Claves => "Claves",
            Self::Laughing => "Laughing",
            Self::Screaming => "Screaming",
            Self::Punch => "Punch",
            Self::Heartbeat => "Heartbeat",
            Self::FootstepsAb => "Footsteps (A♭)",
            Self::FootstepsA => "Footsteps (A)",
            Self::Applause => "Applause",
            Self::Creaking => "Creaking",
            Self::Door => "Door",
            Self::Scratch => "Scratch",
            Self::Windchime => "Windchime",
            Self::Engine => "Engine",
            Self::CarStop => "Car Stop",
            Self::CarPass => "Car Pass",
            Self::Crash => "Crash",
            Self::Siren => "Siren",
            Self::Train => "Train",
            Self::Jet => "Jet",
            Self::Helicopter => "Helicopter",
            Self::Starship => "Starship",
            Self::Pistol => "Pistol",
            Self::Machinegun => "Machinegun",
            Self::Lasergun => "Lasergun",
            Self::Explosion => "Explosion",
            Self::Dog => "Dog",
            Self::Horse => "Horse",
            Self::Birds => "Birds",
            Self::Rain => "Rain",
            Self::Thunder => "Thunder",
            Self::Wind => "Wind",
            Self::Waves => "Waves",
            Self::Stream => "Stream",
            Self::Babble => "Babble",
        };
        f.write_str(s)
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message(pub(crate) Msg);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Event {
    pub msg: Message,
    pub time: u32,
}

#[non_exhaustive]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Error {
    UnfinishedData,
    CannotRepresentAsU32,
    UnrecognizedCommand(u8),
    ControllerTooHigh(u8),
    NoteTooHigh(u8),
    ProgramTooHigh(u8),
    PitchBendTooHigh(u16),
    VelocityTooHigh(u8),
    ChannelTooHigh(u8),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnfinishedData => {
                write!(f, "MIDI message is incomplete")
            }
            Error::CannotRepresentAsU32 => {
                write!(f, "MIDI not representable as u32")
            }
            Error::UnrecognizedCommand(x) => {
                write!(f, "Unrecognized MIDI command: {x}")
            }
            Error::ControllerTooHigh(x) => {
                write!(f, "MIDI controller value too high: {x}")
            }
            Error::NoteTooHigh(x) => {
                write!(f, "MIDI note value too high: {x}")
            }
            Error::ProgramTooHigh(x) => {
                write!(f, "MIDI program value too high: {x}")
            }
            Error::PitchBendTooHigh(x) => {
                write!(f, "MIDI pitch bend value too high: {x}")
            }
            Error::VelocityTooHigh(x) => {
                write!(f, "MIDI velocity value too high: {x}")
            }
            Error::ChannelTooHigh(x) => {
                write!(f, "MIDI channel value too high: {x}")
            }
        }
    }
}

impl error::Error for Error {}

impl Message {
    pub fn note_on(note: u8, velocity: u8, channel: u8) -> Result<Message, Error> {
        if note > 0x7f {
            Err(Error::NoteTooHigh(note))
        } else if velocity > 0x7f {
            Err(Error::VelocityTooHigh(velocity))
        } else if channel > 0xf {
            Err(Error::ChannelTooHigh(channel))
        } else {
            Ok(Message(Msg::NoteOn {
                note,
                velocity,
                channel,
            }))
        }
    }

    pub fn note_off(note: u8, channel: u8) -> Result<Message, Error> {
        if note > 0x7f {
            Err(Error::NoteTooHigh(note))
        } else if channel > 0xf {
            Err(Error::ChannelTooHigh(channel))
        } else {
            Ok(Message(Msg::NoteOff { note, channel }))
        }
    }

    pub fn control_change(cc: u8, value: u8, channel: u8) -> Result<Message, Error> {
        if cc > 0x7f {
            Err(Error::ControllerTooHigh(cc))
        } else if value > 0x7f {
            Err(Error::VelocityTooHigh(value))
        } else if channel > 0xf {
            Err(Error::ChannelTooHigh(channel))
        } else {
            Ok(Message(Msg::ControlChange { cc, value, channel }))
        }
    }

    pub fn program_change(program: u8, channel: u8) -> Result<Message, Error> {
        if program > 0x7f {
            Err(Error::ProgramTooHigh(program))
        } else if channel > 0xf {
            Err(Error::ChannelTooHigh(channel))
        } else {
            Ok(Message(Msg::ProgramChange { program, channel }))
        }
    }

    pub fn pitch_bend(value: u16, channel: u8) -> Result<Message, Error> {
        if value > 0x3fff {
            Err(Error::PitchBendTooHigh(value))
        } else if channel > 0xf {
            Err(Error::ChannelTooHigh(channel))
        } else {
            Ok(Message(Msg::PitchBend { value, channel }))
        }
    }

    pub fn sysex(data: SharedSysex) -> Message {
        Message(Msg::Sysex(data))
    }
}

impl From<Message> for Box<[u8]> {
    fn from(msg: Message) -> Box<[u8]> {
        match msg.0 {
            Msg::NoteOn {
                note,
                velocity,
                channel,
            } => {
                let lo = channel | 0x90;
                let md = note;
                let hi = velocity;
                Box::new([lo, md, hi])
            }
            Msg::NoteOff { note, channel } => {
                let lo = channel | 0x80;
                let md = note;
                Box::new([lo, md, 0])
            }
            Msg::ControlChange { cc, value, channel } => {
                let lo = channel | 0xb0;
                let md = cc;
                let hi = value;
                Box::new([lo, md, hi])
            }
            Msg::ProgramChange { program, channel } => {
                let lo = channel | 0xc0;
                let md = program;
                Box::new([lo, md])
            }
            Msg::PitchBend { value, channel } => {
                let lo = channel | 0xe0;
                let md = (value & 0x7f) as u8;
                let hi = (value >> 7) as u8;
                Box::new([lo, md, hi])
            }
            Msg::Sysex(data) => Box::from(&data[..]),
        }
    }
}

impl TryFrom<Message> for u32 {
    type Error = Error;
    fn try_from(value: Message) -> Result<u32, Error> {
        match value.0 {
            Msg::NoteOn {
                note,
                velocity,
                channel,
            } => {
                let lo = (channel as u32) | 0x90;
                let md = (note as u32) << 8;
                let hi = (velocity as u32) << 16;
                Ok(lo | md | hi)
            }
            Msg::NoteOff { note, channel } => {
                let lo = (channel as u32) | 0x80;
                let md = (note as u32) << 8;
                Ok(lo | md)
            }
            Msg::ControlChange { cc, value, channel } => {
                let lo = (channel as u32) | 0xb0;
                let md = (cc as u32) << 8;
                let hi = (value as u32) << 16;
                Ok(lo | md | hi)
            }
            Msg::ProgramChange { program, channel } => {
                let lo = (channel as u32) | 0xc0;
                let md = (program as u32) << 8;
                Ok(lo | md)
            }
            Msg::PitchBend { value, channel } => {
                let lo = (channel as u32) | 0xe0;
                let md = ((value & 0x7f) as u32) << 8;
                let hi = ((value >> 7) as u32) << 16;
                Ok(lo | md | hi)
            }
            Msg::Sysex(_) => Err(Error::CannotRepresentAsU32),
        }
    }
}

impl TryFrom<u32> for Message {
    type Error = Error;
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        if value > 0x7fffff {
            return Err(Error::UnfinishedData);
        }

        let channel = (value & 0xf) as u8;
        let code = ((value & 0xf0) >> 4) as u8;
        let note = ((value & 0xff00) >> 8) as u8;
        let velocity = ((value & 0xff0000) >> 16) as u8;

        match code {
            0x9 if velocity == 0 => Message::note_off(note, channel),
            0x9 => Message::note_on(note, velocity, channel),
            0x8 => Message::note_off(note, channel),
            0xb => Ok(Message(Msg::ControlChange {
                cc: note,
                value: velocity,
                channel,
            })),
            0xc => {
                if velocity != 0 {
                    return Err(Error::UnfinishedData);
                }
                if note > 0x7f {
                    return Err(Error::NoteTooHigh(note));
                }
                Ok(Message(Msg::ProgramChange {
                    program: note,
                    channel,
                }))
            }
            0xe => {
                let value = ((velocity as u16) << 7) | (note as u16);
                Ok(Message(Msg::PitchBend { value, channel }))
            }
            _ => Err(Error::UnrecognizedCommand(code)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedSysex {
    pub bytes: std::sync::Arc<[u8]>,
    pub length: usize,
}
impl std::ops::Deref for SharedSysex {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.bytes[..self.length]
    }
}
