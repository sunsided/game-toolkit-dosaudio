use std::{collections::VecDeque, sync::Arc};

use crate::device::sb::{DacState, SbControl};
use crate::{
    ConventionalMemory, DmaChannel, DmaController, DmaUnit, Error, IrqRequest, Result, Tick,
};

/// A DSP block completion observed while servicing attached conventional memory.
///
/// Polling this event does not acknowledge the hardware interrupt: read the DSP's
/// byte or word interrupt-acknowledge port separately. Events remain distinct
/// when several auto-init blocks complete with the IRQ line still asserted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DmaCompletion {
    /// Machine timestamp of the DMA unit that completed the DSP block.
    pub at: Tick,
    /// Configured interrupt line and its assertion state at block completion.
    pub irq: IrqRequest,
    /// Physical byte or word channel that supplied the final DMA unit.
    pub channel: DmaChannel,
    /// DSP block-completion counter, as reported at this completion.
    pub block: u64,
}

pub(crate) struct MemoryPlayback {
    pub controller: DmaController,
    pub memory: Arc<ConventionalMemory>,
    completions: VecDeque<DmaCompletion>,
    capacity: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct MemoryPlan {
    pub sb: SbControl,
    pub controller: DmaController,
    pub commands: usize,
}

impl MemoryPlayback {
    pub(crate) fn new(memory: Arc<ConventionalMemory>, capacity: usize) -> Self {
        Self {
            controller: DmaController::new(),
            memory,
            completions: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    fn walk(
        memory: &ConventionalMemory,
        mut sb: SbControl,
        mut controller: DmaController,
        span: (Tick, Tick),
        mut emit: impl FnMut(Tick, DacState),
        mut complete: impl FnMut(DmaCompletion) -> Result<()>,
    ) -> Result<MemoryPlan> {
        let (from, to) = span;
        let mut commands = 0_usize;
        while let Some(next) = sb.next_dma_tick() {
            // A masked/exhausted controller may leave an overdue DSP request.
            // Its first successful transfer resumes at current machine time.
            let due = next.max(from);
            if due > to {
                break;
            }
            let Some(request) = sb.dma_request(due) else {
                break;
            };
            let Some(unit) = controller.transfer(request, memory)? else {
                break;
            };
            let before = sb.completed_blocks();
            let state = match unit {
                DmaUnit::Byte(value) => sb.dma_write(due, request.channel, value)?,
                DmaUnit::Word(value) => sb.dma_write16(due, request.channel, value)?,
            };
            commands = commands.checked_add(1).ok_or(Error::TooLarge)?;
            emit(due, state);
            if sb.completed_blocks() != before {
                complete(DmaCompletion {
                    at: due,
                    irq: sb.irq_request(),
                    channel: request.channel,
                    block: sb.completed_blocks(),
                })?;
            }
        }
        Ok(MemoryPlan {
            sb,
            controller,
            commands,
        })
    }

    pub(crate) fn plan(&self, sb: SbControl, from: Tick, to: Tick) -> Result<MemoryPlan> {
        let mut pending = self.completions.len();
        Self::walk(
            &self.memory,
            sb,
            self.controller,
            (from, to),
            |_, _| {},
            |_| {
                pending = pending.checked_add(1).ok_or(Error::TooLarge)?;
                if pending > self.capacity {
                    Err(Error::IrqQueueFull)
                } else {
                    Ok(())
                }
            },
        )
    }

    pub(crate) fn commit(
        &mut self,
        sb: &mut SbControl,
        from: Tick,
        to: Tick,
        emit: impl FnMut(Tick, DacState),
    ) -> Result<()> {
        // Preflight here also makes this operation transactional independently of
        // the bus's command-queue reservation. Memory size and DMA addresses are
        // stable between these passes; only atomic sample values may change.
        self.plan(*sb, from, to)?;
        let completions = &mut self.completions;
        let plan = Self::walk(
            &self.memory,
            *sb,
            self.controller,
            (from, to),
            emit,
            |event| {
                completions.push_back(event);
                Ok(())
            },
        )?;
        *sb = plan.sb;
        self.controller = plan.controller;
        Ok(())
    }

    pub(crate) fn next(&self, sb: &SbControl, now: Tick) -> Option<Tick> {
        let due = sb.next_dma_tick()?.max(now);
        let request = sb.dma_request(due)?;
        let mut controller = self.controller;
        match controller.transfer(request, &self.memory) {
            Ok(None) => None,
            // Surface memory errors through the next actual advance, so a bad
            // address cannot silently stall an offline render forever.
            Ok(Some(_)) | Err(_) => Some(due),
        }
    }

    pub(crate) fn poll(&mut self) -> Option<DmaCompletion> {
        self.completions.pop_front()
    }
}
