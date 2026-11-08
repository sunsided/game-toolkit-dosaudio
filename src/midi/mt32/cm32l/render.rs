// Copyright (C) 2021-2026 Geoff Hill <geoff@geoffhill.org>
// Copyright (C) 2003-2026 Dean Beeler, Jerome Fisher, Sergey V. Mikayev
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, either version 2.1 of the License, or (at
// your option) any later version. Read COPYING.LESSER.txt for details.

use crate::midi::mt32::cm32l::element::{
    AmpContext, PairRole, PartialArena, Wavegen, lookup_timbre,
};
use crate::midi::mt32::cm32l::midi::{Event, Message};
use crate::midi::mt32::cm32l::param::{PartialParam, TimbreParam};
use crate::midi::mt32::cm32l::reverb::{self, Reverb};
use crate::midi::mt32::cm32l::{CM32L, ControlCommand, ControlError, Frame, Synth};
use std::boxed::Box;

fn distort_sample(sample: i16) -> i32 {
    if (sample & 0x2000) == 0 {
        (sample & 0x1fff) as i32
    } else {
        (sample | !0x1fff) as i32
    }
}

fn get_pan_factor(pan: i32) -> i32 {
    if pan == 0 {
        0
    } else {
        ((pan as i64 * 8192 + 7) / 14) as i32
    }
}

impl Synth for CM32L {
    fn current_time(&self) -> u32 {
        self.time
    }

    fn play_msg_at(&mut self, msg: u32, time: u32) -> bool {
        match Message::try_from(msg) {
            Ok(parsed) => {
                if self.midi_queue.push(Event { msg: parsed, time }) {
                    true
                } else {
                    warn!("Unable to enqueue message");
                    false
                }
            }
            Err(_e) => {
                warn!("Ignoring message: {}", _e);
                true
            }
        }
    }

    fn play_sysex_at(&mut self, sysex: &[u8], time: u32) -> bool {
        if sysex.len() > 65_536 {
            return false;
        }
        let Some(slot) = self
            .sysex_pool
            .iter_mut()
            .find(|b| std::sync::Arc::strong_count(b) == 1)
        else {
            return false;
        };
        std::sync::Arc::get_mut(slot).unwrap()[..sysex.len()].copy_from_slice(sysex);
        let data = super::midi::SharedSysex {
            bytes: std::sync::Arc::clone(slot),
            length: sysex.len(),
        };
        self.midi_queue.push(Event {
            msg: Message::sysex(data),
            time,
        })
    }

    fn render(&mut self, out: &mut [Frame]) {
        let total = out.len() as u32;
        let mut produced = 0u32;

        while produced < total {
            // When a poly is aborting, render 1 sample at a time to let the
            // abort complete, then retry the pending MIDI event.
            if self.free_partials.is_aborting_poly() {
                trace!("render waiting for abort");
                let start = produced as usize;
                let slice = &mut out[start..start + 1];
                self.fill_frames(slice);
                self.time = self.time.wrapping_add(1);
                produced = produced.wrapping_add(1);
                continue;
            }

            if let Some(ev) = self.midi_queue.peek() {
                if self.time.wrapping_sub(ev.time) as i32 >= 0 {
                    let msg = ev.msg.clone();
                    self.process_midi(msg);

                    // If processing triggered an abort, don't pop the event yet.
                    // We'll retry on the next iteration after the abort completes.
                    if !self.free_partials.is_aborting_poly() {
                        self.midi_queue.pop();
                    }

                    // Render 1 sample after each MIDI event.
                    let start = produced as usize;
                    let slice = &mut out[start..start + 1];
                    self.fill_frames(slice);
                    self.time = self.time.wrapping_add(1);
                    produced = produced.wrapping_add(1);
                    continue;
                }
            }

            let chunk = match self.midi_queue.peek() {
                Some(ev) => {
                    let until_next = ev.time.wrapping_sub(self.time);
                    until_next.min(total - produced)
                }
                None => total - produced,
            };

            if chunk > 0 {
                let start = produced as usize;
                let end = (produced + chunk) as usize;
                let slice = &mut out[start..end];

                self.fill_frames(slice);

                self.time = self.time.wrapping_add(chunk);
                produced = produced.wrapping_add(chunk);
                continue;
            }
        }
    }

    fn apply_command(&mut self, command: ControlCommand) -> Result<(), ControlError> {
        self.apply_command_inner(command)
    }
}

impl CM32L {
    fn fill_frames(&mut self, out: &mut [Frame]) {
        let out_len = out.len();
        let _span = debug_span!("fill_frames", frame_count = out_len);

        let mut pos = 0;
        while pos < out_len {
            let chunk_len = (out_len - pos).min(reverb::MAX_BUFFER_LEN);
            let chunk = &mut out[pos..pos + chunk_len];
            self.reverb.prepare(chunk_len);
            for i in 0..32 {
                let partial = match &self.free_partials.partials[i] {
                    Some(p) => p,
                    None => continue,
                };
                let part = &self.parts.parts[partial.config.part_idx];
                let (amp_ctx, modulation, pitch_bend) =
                    (&part.amp_ctx, part.modulation, part.pitch_bend);
                let timbre = lookup_timbre(&partial.config, &self.mem);
                Self::render_partial(
                    &mut self.free_partials,
                    i,
                    amp_ctx,
                    &mut self.reverb,
                    self.rom.pcm(),
                    modulation,
                    pitch_bend,
                    self.master_tune_pitch_delta,
                    timbre,
                );
            }

            self.reverb.apply_la32_output();
            self.reverb.process();
            self.reverb.mix_output(chunk);

            for frame in chunk.iter_mut() {
                frame.0 = self.lpf_left.process(frame.0);
                frame.1 = self.lpf_right.process(frame.1);
            }

            pos += chunk_len;
        }

        self.free_partials
            .cleanup_finished_polys(&mut self.free_polys, &mut self.parts);
        self.free_partials
            .check_aborting_poly_done(&mut self.free_polys, &mut self.parts);
    }

    fn process_partial_sample(
        partials: &mut PartialArena,
        idx: usize,
        amp_ctx: &AmpContext,
        pcm_rom: &[i16],
        modulation: u8,
        pitch_bend: i32,
        master_tune: i32,
        use_interpolated: bool,
        pp: &PartialParam,
    ) -> Option<i16> {
        {
            let partial = partials.partials[idx].as_ref()?;
            let dead = partial.tva.is_dead();
            let pcm_ex = matches!(
                &partial.wavegen,
                Wavegen::Pcm(pcm) if pcm.is_exhausted()
            );
            if dead || pcm_ex {
                let _part = partial.config.part_idx;
                debug!(idx, _part, dead, pcm_ex, "partial deactivated");
                partials.deactivate(idx);
                return None;
            }
        }

        let Some(partial) = partials.partials[idx].as_mut() else {
            return None;
        };

        let (pitch, tvp_timer_fired) = partial.tvp.next_pitch(modulation, pitch_bend, master_tune);
        if tvp_timer_fired {
            partial.tva.recalc_sustain(amp_ctx, &pp.tva);
        }

        let amp = partial.tva.next_amplitude(amp_ctx, &pp.tva);
        let cutoff = match &partial.wavegen {
            Wavegen::Pcm(_) => 0,
            Wavegen::Oscillator(_) => partial.tvf.next_cutoff(&pp.tvf),
        };

        let amp_val = 67117056u32.saturating_sub(amp);

        let sample = match &mut partial.wavegen {
            Wavegen::Pcm(pcm) => {
                let sample = if use_interpolated {
                    pcm.get_interpolated_sample(pcm_rom, amp_val)
                } else {
                    pcm.get_first_sample(pcm_rom, amp_val)
                };
                pcm.advance(pitch);
                if pcm.is_exhausted() { 0 } else { sample }
            }
            Wavegen::Oscillator(synth) => {
                synth.generate_samples(amp_val, pitch, cutoff);
                synth.get_output_sample()
            }
        };

        trace!(sample, amp_val, amp, pitch, "partial sample");

        Some(sample)
    }

    fn render_partial(
        partials: &mut PartialArena,
        partial_idx: usize,
        amp_ctx: &AmpContext,
        reverb: &mut Reverb,
        pcm_rom: &[i16],
        modulation: u8,
        pitch_bend: i32,
        master_tune: i32,
        timbre: &TimbreParam,
    ) {
        let partial = match &partials.partials[partial_idx] {
            Some(p) => p,
            None => return,
        };

        let pair_role = partial.config.pair_role;

        let _span = debug_span!("partial", partial_idx, ?pair_role);

        if matches!(pair_role, PairRole::RingSlave { .. }) {
            return;
        }

        let use_reverb = partial.config.reverb;
        let panpot = partial.config.panpot;
        let owner_poly_idx = partial.config.owner_poly_idx;
        let master_slot = partial.config.timbre_slot;

        let ring_mod_slave = match pair_role {
            PairRole::RingMaster {
                slave_idx,
                ring_only,
            } if partials.partials[slave_idx]
                .as_ref()
                .is_some_and(|s| s.config.owner_poly_idx == owner_poly_idx) =>
            {
                partials.partials[slave_idx]
                    .as_ref()
                    .map(|slave| (slave_idx, ring_only, slave.config.timbre_slot))
            }
            _ => None,
        };

        let (buf_left, buf_right) = if use_reverb {
            reverb.dry_bufs()
        } else {
            reverb.clean_bufs()
        };

        let left_pan_factor = get_pan_factor(panpot);
        let right_pan_factor = get_pan_factor(14 - panpot);

        let master_pp = &timbre.partials[master_slot];

        for i in 0..buf_left.len() {
            let master_sample = match Self::process_partial_sample(
                partials,
                partial_idx,
                amp_ctx,
                pcm_rom,
                modulation,
                pitch_bend,
                master_tune,
                true,
                master_pp,
            ) {
                Some(s) => s,
                None => {
                    if let Some((slave_idx, _, _)) = ring_mod_slave {
                        debug!(slave_idx, "deactivating ring mod slave with master");
                        partials.deactivate(slave_idx);
                    }
                    break;
                }
            };

            let slave_sample = if let Some((slave_idx, ring_only, slave_slot)) = ring_mod_slave {
                let slave_pp = &timbre.partials[slave_slot];
                match Self::process_partial_sample(
                    partials,
                    slave_idx,
                    amp_ctx,
                    pcm_rom,
                    modulation,
                    pitch_bend,
                    master_tune,
                    false,
                    slave_pp,
                ) {
                    Some(s) => {
                        // If slave TVA died, deactivate before ring mod use.
                        let slave_died = partials.partials[slave_idx]
                            .as_ref()
                            .map_or(false, |p| p.tva.is_dead());
                        if slave_died {
                            debug!(slave_idx, ring_only, "ring mod slave TVA died mid-sample");
                            partials.deactivate(slave_idx);
                            if ring_only {
                                partials.deactivate(partial_idx);
                                break;
                            }
                            None
                        } else {
                            Some((s, ring_only))
                        }
                    }
                    None => {
                        if ring_only {
                            partials.deactivate(partial_idx);
                            break;
                        }
                        None
                    }
                }
            } else {
                None
            };

            let sample = if let Some((slave_sample, ring_only)) = slave_sample {
                let mod_master = distort_sample(master_sample);
                let mod_slave = distort_sample(slave_sample);
                let ring_mod_sample = ((mod_master * mod_slave) >> 13) as i16;

                if ring_only {
                    ring_mod_sample
                } else {
                    master_sample.saturating_add(ring_mod_sample)
                }
            } else {
                master_sample
            };

            let scaled = sample as i32;
            let left_sample = (scaled * left_pan_factor) >> 13;
            let right_sample = (scaled * right_pan_factor) >> 13;

            buf_left[i] = buf_left[i].saturating_add(left_sample as i16);
            buf_right[i] = buf_right[i].saturating_add(right_sample as i16);
        }
    }
}
