use std::sync::Arc;
use std::time::Instant;

use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event_loop::EventLoopWindowTarget;
use winit::monitor::MonitorHandle;
use winit::platform::x11::WindowBuilderExtX11;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Window, WindowBuilder, WindowLevel};

use snowhush::geometry::V2;
use snowhush::simulation::{Pace, SnowSimulation, WindowId, WindowSurface};

use crate::log;
use crate::render::{Ambient, Canvas, Gpu};
use crate::tracker::{Bounds, Tracker};

/// How much the snow may spend on drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerBudget {
    Full,
    /// On battery: no frame rates above 60, which are hard to tell apart in snow anyway.
    Battery,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Monitor {
    pub name: String,
    /// The work area in physical pixels: the panel is the ground the snow settles on, so it is left out.
    pub rect: Bounds,
    /// The whole display in physical pixels.
    pub bounds: Bounds,
    pub scale: f64,
    pub refresh: u32,
}

pub fn monitors(handles: &[MonitorHandle], tracker: &Tracker) -> Vec<Monitor> {
    handles
        .iter()
        .map(|handle| {
            let position = handle.position();
            let size = handle.size();
            let bounds = Bounds {
                x: position.x,
                y: position.y,
                width: size.width.max(1).min(i32::MAX as u32) as i32,
                height: size.height.max(1).min(i32::MAX as u32) as i32,
            };
            let mut rect = tracker.work_area(bounds);
            rect.x = rect.x.max(bounds.x);
            rect.y = rect.y.max(bounds.y);
            rect.width = rect.width.min(bounds.right().saturating_sub(rect.x)).max(1);
            rect.height = rect
                .height
                .min(bounds.bottom().saturating_sub(rect.y))
                .max(1);
            Monitor {
                name: handle.name().unwrap_or_else(|| {
                    format!(
                        "{}x{}+{}+{}",
                        bounds.width, bounds.height, bounds.x, bounds.y
                    )
                }),
                rect,
                bounds,
                scale: handle.scale_factor(),
                refresh: handle
                    .refresh_rate_millihertz()
                    .map_or(60, |milli| (milli / 1000).clamp(60, 120)),
            }
        })
        .collect()
}

pub struct Clock {
    started: Instant,
}

impl Clock {
    pub fn new() -> Clock {
        Clock {
            started: Instant::now(),
        }
    }

    pub fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }
}

/// The snow on one display: its overlay window, drawing surface and simulation.
pub struct Screen {
    pub monitor: Monitor,
    pub simulation: SnowSimulation,
    pub next_due: f64,
    window: Arc<Window>,
    canvas: Canvas,
    size: (u32, u32),
    xwindow: u32,
    last_step: f64,
    idle_frames: u32,
    interactive_rate: u32,
    surfaces: Vec<WindowSurface>,
    windows_seen: u64,
    windows_changed_at: f64,
    covered: bool,
    paused: bool,
}

impl Screen {
    pub fn new<T: 'static>(
        gpu: &mut Gpu,
        target: &EventLoopWindowTarget<T>,
        tracker: &Tracker,
        monitor: Monitor,
        simulation: SnowSimulation,
        format_hint: Option<wgpu::TextureFormat>,
        paused: bool,
    ) -> Result<Screen, String> {
        let size = dip_size(&monitor);
        let window = Arc::new(
            WindowBuilder::new()
                .with_title("Snowhush")
                .with_decorations(false)
                .with_transparent(true)
                .with_resizable(false)
                .with_active(false)
                .with_window_level(WindowLevel::AlwaysOnTop)
                .with_override_redirect(true)
                .with_visible(false)
                .with_position(PhysicalPosition::new(monitor.rect.x, monitor.rect.y))
                .with_inner_size(PhysicalSize::new(
                    monitor.rect.width.max(1) as u32,
                    monitor.rect.height.max(1) as u32,
                ))
                .build(target)
                .map_err(|error| format!("opening an overlay: {error}"))?,
        );
        // Clicks fall through to the windows below, as snow should never take the pointer.
        if let Err(error) = window.set_cursor_hittest(false) {
            log!("input passthrough failed: {error}");
        }
        let mut xwindow = 0;
        if let Ok(handle) = window.window_handle() {
            if let RawWindowHandle::Xlib(xlib) = handle.as_raw() {
                xwindow = xlib.window as u32;
                tracker.set_input_transparent(xwindow);
                tracker.decorate_overlay(xwindow);
            }
        }

        let surface = gpu.create_surface(window.clone())?;
        let caps = surface.get_capabilities(&gpu.adapter);
        let format = match format_hint {
            Some(format) if caps.formats.contains(&format) => format,
            Some(format) => {
                return Err(format!(
                    "this display does not support the shared {format:?} surface format"
                ));
            }
            None => gpu
                .preferred_format(&surface)
                .ok_or_else(|| "this display offers no usable surface format".to_string())?,
        };
        let alpha = gpu
            .preferred_alpha(&surface)
            .ok_or_else(|| "this display offers no composite alpha mode".to_string())?;
        if alpha != wgpu::CompositeAlphaMode::PreMultiplied {
            log!("surface alpha mode is {alpha:?}; transparency may look wrong");
        }
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: monitor.rect.width.max(1) as u32,
            height: monitor.rect.height.max(1) as u32,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: alpha,
            view_formats: vec![],
        };
        surface.configure(&gpu.device, &config);
        gpu.ensure_pipelines(format);

        let scale = monitor.rect.width as f32 / size.0 as f32;
        let canvas = Canvas::new(gpu, surface, config, simulation.column_count(), scale);
        log!(
            "display {}: work area {}x{}+{},{} of {}x{}+{},{} at {:.2}x, {} Hz, {:?}, {:?}",
            monitor.name,
            monitor.rect.width,
            monitor.rect.height,
            monitor.rect.x,
            monitor.rect.y,
            monitor.bounds.width,
            monitor.bounds.height,
            monitor.bounds.x,
            monitor.bounds.y,
            monitor.scale,
            monitor.refresh,
            format,
            alpha
        );
        if !paused {
            window.set_visible(true);
        }
        // Reapply the shape after mapping as well: some servers only consult it once the window is
        // viewable, and an overlay must never take a click.
        if xwindow != 0 {
            tracker.set_input_transparent(xwindow);
        }
        Ok(Screen {
            interactive_rate: monitor.refresh,
            monitor,
            simulation,
            next_due: 0.0,
            window,
            canvas,
            size,
            xwindow,
            last_step: 0.0,
            idle_frames: 0,
            surfaces: Vec::new(),
            windows_seen: 0,
            windows_changed_at: 0.0,
            covered: false,
            paused,
        })
    }

    /// Steps the snow and draws it if anything visible changed; returns whether it drew.
    #[allow(clippy::too_many_arguments)]
    pub fn frame(
        &mut self,
        gpu: &Gpu,
        now: f64,
        budget: PowerBudget,
        light: [f32; 3],
        time: f32,
        windows: Option<&Tracker>,
        cursor: (i32, i32),
    ) -> Result<bool, wgpu::SurfaceError> {
        let dt = if self.last_step == 0.0 {
            1.0 / 60.0
        } else {
            (now - self.last_step).min(0.05) as f32
        };
        self.last_step = now;
        if let Some(tracker) = windows {
            self.follow(tracker, now);
        }
        let changed = self.simulation.step(dt, self.local(cursor));

        let fastest = if budget == PowerBudget::Full {
            self.interactive_rate
        } else {
            self.interactive_rate.min(60)
        };
        let rate = match self.simulation.pace() {
            Pace::Calm => 30,
            Pace::Lively => fastest.min(60),
            Pace::Interactive => fastest,
        };
        self.next_due = now + 1.0 / rate as f64;
        self.idle_frames = if changed { 0 } else { self.idle_frames + 1 };
        // A still pile only needs a slow refresh for its glints.
        if !(changed || (self.simulation.has_snow() && self.idle_frames % 2 == 0)) {
            return Ok(false);
        }
        self.canvas
            .draw(gpu, &self.simulation, Ambient { light }, time)?;
        Ok(true)
    }

    /// Hands the simulation the windows on this display when they changed.
    fn follow(&mut self, tracker: &Tracker, now: f64) {
        if tracker.generation == self.windows_seen {
            return;
        }
        self.windows_seen = tracker.generation;
        let surfaces = tracker.surfaces(&self.monitor.rect, self.size);
        if surfaces == self.surfaces {
            return;
        }
        let since = (now - self.windows_changed_at) as f32;
        // A window that moves outside a drag, as when it is maximized or snapped, jumps there in one go:
        // its snow is left behind instead of being carried along.
        let interval = if tracker.moving() || since < 0.1 {
            since.min(0.25)
        } else {
            1.0 / 60.0
        };
        let elsewhere: std::collections::HashSet<WindowId> = self
            .surfaces
            .iter()
            .map(|surface| surface.id)
            .filter(|&id| !surfaces.iter().any(|surface| surface.id == id) && tracker.elsewhere(id))
            .collect();
        self.simulation.update_windows(
            &surfaces,
            interval,
            &std::collections::HashSet::new(),
            &elsewhere,
            false,
        );
        self.surfaces = surfaces;
        self.windows_changed_at = now;
    }

    pub fn drop_window_snow(&mut self) {
        self.simulation.update_windows(
            &[],
            0.1,
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            false,
        );
        self.surfaces.clear();
        self.windows_seen = 0;
    }

    /// Hides the snow while a full-screen app covers the display, and stops it, as it stops over a full-screen app on a Mac.
    pub fn set_covered(&mut self, covered: bool) {
        if covered == self.covered {
            return;
        }
        self.covered = covered;
        self.apply_visibility();
        if !self.hidden() {
            self.resume();
        }
    }

    /// Hides the overlay without forgetting the snow, as when the session is locked. A paused
    /// screen is invisible and is not stepped; unlocking resumes it exactly where it stopped.
    pub fn set_paused(&mut self, paused: bool) {
        if paused == self.paused {
            return;
        }
        self.paused = paused;
        self.apply_visibility();
        if !self.hidden() {
            self.resume();
        }
    }

    fn hidden(&self) -> bool {
        self.covered || self.paused
    }

    fn apply_visibility(&self) {
        let visible = !self.hidden();
        log!("overlay {} visible: {visible}", self.monitor.name);
        self.window.set_visible(visible);
    }

    pub fn covered(&self) -> bool {
        self.covered
    }

    /// Override-redirect overlays sit outside the window manager's stacking rules, so they are
    /// raised whenever the window list changes and after the snow comes back.
    pub fn raise(&self, tracker: &Tracker) {
        if self.xwindow != 0 {
            tracker.raise_overlay(self.xwindow);
        }
    }

    /// Gives the snow a chance to settle again after the screen was locked or the display slept.
    pub fn resume(&mut self) {
        self.last_step = 0.0;
        self.next_due = 0.0;
        self.windows_seen = 0;
    }

    pub fn wake(&mut self, now: f64) {
        self.next_due = self.next_due.min(now);
    }

    pub fn reconfigure(&self, gpu: &Gpu) {
        self.canvas.configure(gpu);
    }

    /// The pointer in overlay coordinates.
    fn local(&self, point: (i32, i32)) -> V2 {
        let rect = &self.monitor.rect;
        let (width, height) = self.size;
        V2::new(
            (point.0 - rect.x) as f32 * width as f32 / rect.width.max(1) as f32,
            (rect.bottom() - point.1) as f32 * height as f32 / rect.height.max(1) as f32,
        )
    }
}

pub fn dip_size(monitor: &Monitor) -> (u32, u32) {
    let scale = monitor.scale.max(0.5);
    (
        (monitor.rect.width as f64 / scale).ceil() as u32,
        (monitor.rect.height as f64 / scale).ceil() as u32,
    )
}
