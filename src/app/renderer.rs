//! The rasterizer behind a frame: the GPU when one was found and keeps working, else the CPU.
//!
//! A frame is rasterized in one of two ways. `rasterize` returns when the pixels are there, which
//! is what a headless run (`--output`, `--benchmark`) and the tests use. `overlap` is for the
//! interactive loop: the GPU draws one frame while the caller records the next, so the frame it
//! hands back is the one before, and what the user sees is one frame behind what the scene has
//! just recorded.
//!
//! What a frame brings back is its pixels, its picks (`Frame::picks`, `identities`, `things`) and
//! the selected process's pixel position (`Frame::anchor`); those agree with each other. The text
//! the app writes over it (`App::overlay`) is placed from the app's state as it stands when the
//! frame is presented, which is one frame newer. That is exact only while nothing the text
//! anchors to has moved between the two, so `overlap` pipelines a frame only when its caller says
//! the anchors are at rest (the camera and the view are the ones that drew the frame before) and
//! finishes it at once otherwise; a body that moves under its own power, or the free turn of the
//! globe, still leads its pixels by one frame of its own motion (under a pixel for the globe, a
//! fraction of a terminal cell for a body). A hover is looked up in the window of picks read
//! around the pointer as it was when the frame was submitted, so a pointer that has moved more
//! than 80 px since finds nothing for one frame.

use crate::cli::{Power, RendererChoice};
use crate::gpu::{Flight, Gpu, Picks, Stats};
use crate::render::Frame;

/// A frame the GPU has been given and has not finished.
struct Flying {
    flight: Flight,
    frame: Frame,
}

/// Turns a recorded frame into pixels, and says which rasterizer did.
pub(crate) struct Renderer {
    gpu: Option<Gpu>,
    /// Which rasterizer draws the frames, for the status strip.
    kind: String,
    /// Whether the GPU brings back every pick of a frame, not only those around the pointer.
    every_pick: bool,
    /// Whether `overlap` lets the GPU run a frame behind.
    pipelined: bool,
    /// The frame `overlap` has submitted and not yet handed back.
    flying: Option<Flying>,
}

impl Renderer {
    /// The CPU rasterizer.
    pub(crate) fn cpu() -> Self {
        Self::with(None, "cpu".into())
    }

    fn with(gpu: Option<Gpu>, kind: String) -> Self {
        Self {
            gpu,
            kind,
            every_pick: false,
            pipelined: false,
            flying: None,
        }
    }

    /// The rasterizer `choice` asks for. `Auto` falls back to the CPU when no GPU can be opened,
    /// saying why in `kind`; `Gpu` fails instead. A `headless` run (`--output`, `--benchmark`)
    /// may take a software adapter when `ISOTOP_ALLOW_SOFTWARE_GPU=1` says so; a live session
    /// never does, since an emulated GPU is slower than the CPU rasterizer.
    pub(crate) fn choose(
        choice: RendererChoice,
        power: Power,
        headless: bool,
    ) -> Result<Self, String> {
        if choice == RendererChoice::Cpu {
            return Ok(Self::cpu());
        }
        let preference = match power {
            Power::Low => wgpu::PowerPreference::LowPower,
            Power::High => wgpu::PowerPreference::HighPerformance,
        };
        let opened = if headless {
            Gpu::new(preference)
        } else {
            Gpu::hardware(preference)
        };
        match opened {
            Ok(gpu) => {
                let kind = format!("gpu {}", gpu.name);
                Ok(Self::with(Some(gpu), kind))
            }
            Err(error) if choice == RendererChoice::Auto => {
                Ok(Self::with(None, format!("cpu ({error})")))
            }
            Err(error) => Err(error),
        }
    }

    /// Reads back every pick of a GPU frame instead of a window around the pointer: for a run
    /// that checks picks or writes them down, where there is no pointer.
    pub(crate) fn reading_every_pick(mut self, every_pick: bool) -> Self {
        self.every_pick = every_pick;
        self
    }

    /// Lets `overlap` run the GPU a frame behind (`true`) or finish each frame at once (`false`,
    /// which is how a renderer starts). A frame still in flight is finished first.
    pub(crate) fn set_pipelined(&mut self, pipelined: bool) -> Option<Frame> {
        let flushed = self.flush();
        self.pipelined = pipelined;
        flushed
    }

    /// What drew the last frame: `cpu`, `gpu <adapter>`, or a CPU fallback with its reason.
    pub(crate) fn kind(&self) -> &str {
        &self.kind
    }

    /// The draw calls of the last frame the GPU drew, None on the CPU.
    pub(crate) fn draw_stats(&self) -> Option<Stats> {
        self.gpu.as_ref().map(Gpu::stats)
    }

    /// Which of the pick buffer the GPU reads back for a pointer at `pointer`.
    fn picks(&self, pointer: Option<[f32; 2]>) -> Picks {
        match pointer {
            _ if self.every_pick => Picks::Everywhere,
            Some(pointer) => Picks::Around(pointer),
            None => Picks::Nowhere,
        }
    }

    /// Fills `frame`'s colour, depth and pick buffers. A GPU that fails is given up for the CPU,
    /// which then draws this frame; `pointer` is the frame position the GPU reads the pick under.
    pub(crate) fn rasterize(&mut self, frame: &mut Frame, pointer: Option<[f32; 2]>) {
        let picks = self.picks(pointer);
        let failure = match self.gpu.as_mut() {
            Some(gpu) => gpu.render(frame, picks).err(),
            None => {
                frame.rasterize();
                None
            }
        };
        if let Some(error) = failure {
            self.fail(&error);
            frame.rasterize();
        }
    }

    /// Gives `frame` to the GPU and returns the frame it was given before, finished: with the CPU,
    /// with the GPU not allowed to run behind, or with `anchors_at_rest` false, this frame itself,
    /// at once (a frame still in flight is dropped for it, unshown). None for the first frame the
    /// GPU is given, which has nothing before it. The frame handed back has its pixels, picks and
    /// `anchor` from one recording; the next call hands back this one.
    ///
    /// `anchors_at_rest` says that what the caller places text by (the camera, the view) is what
    /// drew the frame given before this one, so text placed from the caller's state now lands on
    /// the pixels of that frame. The caller finishes frames at once while it is not.
    pub(crate) fn overlap(
        &mut self,
        mut frame: Frame,
        pointer: Option<[f32; 2]>,
        anchors_at_rest: bool,
    ) -> Option<Frame> {
        if self.gpu.is_none() || !self.pipelined || !anchors_at_rest {
            let earlier = self.flush();
            self.rasterize(&mut frame, pointer);
            // A frame flushed here is older than this one, and is dropped for it.
            drop(earlier);
            return Some(frame);
        }
        let picks = self.picks(pointer);
        let gpu = self.gpu.as_mut()?;
        let submitted = gpu.submit(&frame, picks);
        let before = self.flying.take();
        let mut failure = None;
        let ready = match submitted {
            Ok(flight) => {
                let ready = before.map(
                    |Flying {
                         flight: old,
                         frame: mut ready,
                     }| {
                        if let Err(error) = gpu.collect(old, &mut ready) {
                            failure = Some(error);
                            ready.rasterize();
                        }
                        ready
                    },
                );
                self.flying = Some(Flying { flight, frame });
                ready
            }
            Err(error) => {
                failure = Some(error);
                before.map(
                    |Flying {
                         flight,
                         frame: mut ready,
                     }| {
                        drop(flight);
                        ready.rasterize();
                        ready
                    },
                )
            }
        };
        if let Some(error) = failure {
            self.fail(&error);
        }
        ready
    }

    /// Finishes the frame `overlap` has in flight, if any, and hands it back.
    pub(crate) fn flush(&mut self) -> Option<Frame> {
        let Flying { flight, mut frame } = self.flying.take()?;
        match self.gpu.as_mut().map(|gpu| gpu.collect(flight, &mut frame)) {
            Some(Ok(())) => {}
            Some(Err(error)) => {
                self.fail(&error);
                frame.rasterize();
            }
            None => frame.rasterize(),
        }
        Some(frame)
    }

    /// Gives the GPU up for the CPU, and lets go of a frame it still holds.
    fn fail(&mut self, error: &str) {
        self.gpu = None;
        self.flying = None;
        self.kind = format!("cpu (GPU failed: {error})");
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::cpu()
    }
}
