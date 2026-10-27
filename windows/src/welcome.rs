use std::cell::RefCell;
use std::mem::size_of;
use std::rc::Rc;
use std::sync::Once;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1Factory, ID2D1RenderTarget,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH, DWRITE_FONT_STRETCH_CONDENSED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_LIGHT, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_LINE_SPACING_METHOD_UNIFORM,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_METRICS, DWRITE_TEXT_RANGE, DWriteCreateFactory, IDWriteFactory,
    IDWriteTextLayout, IDWriteTextLayout1,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDC, GetMonitorInfoW, HGDIOBJ, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, ReleaseDC, SelectObject,
};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmapSource, IWICImagingFactory, WICBitmapCacheOnLoad, WICBitmapDitherTypeNone,
    WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, VK_ESCAPE, VK_RETURN};
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Error, Interface, PCWSTR, Result, w};
use windows_numerics::Vector2;

use snowhush::geometry::V2;

use crate::log;
use crate::snow::Piece;

const CLASS: PCWSTR = w!("SnowhushWelcome");
const ICON: &[u8] = include_bytes!("../assets/icon-256.png");
const WIDTH: f32 = 440.0;
const HEIGHT: f32 = 430.0;
/// Room around the card for its shadow.
const MARGIN: f32 = 36.0;
const BACKGROUND: [f32; 3] = [0.105, 0.088, 0.072];
const INK: [f32; 3] = [0.93, 0.9, 0.85];
const MUTED: [f32; 3] = [0.78, 0.75, 0.7];
const LABEL: [f32; 3] = [0.09, 0.075, 0.06];
const TITLE: &str = "IT'S SNOWING\nON YOUR DESKTOP.";
const BODY: &str = "Snowhush lives in the system tray. Click its icon to change the snowfall, let the drift melt, or keep the snow off your windows. Sweep it aside with your cursor.";
const BUTTON: &str = "GET STARTED";
/// Spacing of the pieces that fly off.
const CELL: f32 = 6.0;
/// Spacing of the crumbling front: fine enough that its edge reads as a line, not as squares.
const GRAIN: f32 = 2.0;
/// Seconds over which a spot fades out once the front reaches it.
const FEATHER: f64 = 0.05;
const TIMER: usize = 1;

pub struct Handlers {
    /// Receives the pieces of the window as it crumbles, in physical screen pixels.
    pub scatter: Box<dyn Fn(Vec<Piece>)>,
    pub finished: Box<dyn Fn()>,
    /// Called instead of `scatter` when the window can't be drawn and just closes.
    pub skipped: Box<dyn Fn()>,
}

thread_local! {
    static WELCOME: RefCell<Option<Welcome>> = const { RefCell::new(None) };
    static HANDLERS: RefCell<Option<Rc<Handlers>>> = const { RefCell::new(None) };
}

struct Welcome {
    window: HWND,
    scale: f32,
    /// The bitmap's size in physical pixels, shadow included.
    size: (i32, i32),
    painter: Painter,
    pressed: bool,
    /// The mouse went down on the button and hasn't come up yet.
    holding: bool,
    crumble: Option<Crumble>,
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Down,
    Move,
    Up,
}

struct Cell {
    /// DIPs from the card's top left.
    x: f32,
    y: f32,
    color: [f32; 3],
    delay: f64,
}

struct Crumble {
    started: Instant,
    base: Vec<u8>,
    /// When the front reaches each pixel of the bitmap.
    front: Vec<f32>,
    cells: Vec<Cell>,
    next: usize,
    duration: f64,
    origin: POINT,
    seed: u64,
}

/// Shown on first launch. Getting started crumbles the window into snow that falls onto the desktop.
/// If the window can't be drawn, the snowfall just starts.
pub fn show(handlers: Handlers) {
    let handlers = Rc::new(handlers);
    HANDLERS.set(Some(handlers.clone()));
    if let Err(error) = open() {
        log!("welcome failed: {error}");
        HANDLERS.set(None);
        (handlers.skipped)();
        (handlers.finished)();
    }
}

fn open() -> Result<()> {
    static REGISTER: Once = Once::new();
    unsafe {
        let instance = GetModuleHandleW(None)?;
        REGISTER.call_once(|| {
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(procedure),
                hInstance: instance.into(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                lpszClassName: CLASS,
                ..Default::default()
            };
            RegisterClassExW(&class);
        });

        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(monitor, &mut info);
        let (mut dpi, mut _y) = (96, 96);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut _y);
        let scale = dpi.max(96) as f32 / 96.0;
        let size = (((WIDTH + 2.0 * MARGIN) * scale).ceil() as i32, ((HEIGHT + 2.0 * MARGIN) * scale).ceil() as i32);
        let work = info.rcWork;
        let left = work.left + (work.right - work.left - size.0) / 2;
        let top = work.top + (work.bottom - work.top - size.1) / 2;

        let mut painter = Painter::new(scale)?;
        let pixels = painter.paint(size, false)?;
        let window = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            CLASS,
            w!("Snowhush"),
            WS_POPUP,
            left,
            top,
            size.0,
            size.1,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        present(window, size, &pixels, Some(POINT { x: left, y: top }));
        WELCOME.set(Some(Welcome { window, scale, size, painter, pressed: false, holding: false, crumble: None }));
        let _ = ShowWindow(window, SW_SHOW);
        let _ = SetForegroundWindow(window);
    }
    Ok(())
}

unsafe extern "system" fn procedure(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_NCHITTEST => {
            let point = POINT { x: (lparam.0 & 0xffff) as i16 as i32, y: ((lparam.0 >> 16) & 0xffff) as i16 as i32 };
            let over = WELCOME.with_borrow(|welcome| welcome.as_ref().map(|welcome| welcome.over_button(point, true)));
            // The card drags the window anywhere but on the button.
            LRESULT(if over == Some(false) { HTCAPTION as isize } else { HTCLIENT as isize })
        }
        WM_LBUTTONDOWN | WM_MOUSEMOVE | WM_LBUTTONUP => {
            let phase = match message {
                WM_LBUTTONDOWN => Phase::Down,
                WM_MOUSEMOVE => Phase::Move,
                _ => Phase::Up,
            };
            match phase {
                Phase::Down => unsafe {
                    SetCapture(window);
                },
                Phase::Up => unsafe {
                    let _ = ReleaseCapture();
                },
                Phase::Move => {}
            }
            let outcome =
                WELCOME.with_borrow_mut(|welcome| welcome.as_mut().map(|welcome| (welcome.press(lparam, phase), welcome.window, welcome.size)));
            if let Some(((clicked, repaint), window, size)) = outcome {
                if let Some(pixels) = repaint {
                    present(window, size, &pixels, None);
                }
                if clicked {
                    get_started();
                }
            }
            LRESULT(0)
        }
        WM_KEYDOWN if wparam.0 == VK_RETURN.0 as usize || wparam.0 == VK_ESCAPE.0 as usize => {
            get_started();
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER => {
            tick();
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

fn get_started() {
    let started = WELCOME.with_borrow_mut(|welcome| {
        let welcome = welcome.as_mut()?;
        if welcome.crumble.is_some() {
            return None;
        }
        welcome.pressed = false;
        let base = welcome.painter.paint(welcome.size, false).ok()?;
        let mut rect = RECT::default();
        unsafe { GetWindowRect(welcome.window, &mut rect).ok()? };
        welcome.crumble = Some(Crumble::new(base, welcome.size, welcome.scale, POINT { x: rect.left, y: rect.top }));
        unsafe { SetTimer(Some(welcome.window), TIMER, 15, None) };
        Some(())
    });
    if started.is_none() {
        finish(true);
    }
}

fn tick() {
    let step = WELCOME.with_borrow_mut(|welcome| {
        let welcome = welcome.as_mut()?;
        let scale = welcome.scale;
        let crumble = welcome.crumble.as_mut()?;
        let elapsed = crumble.started.elapsed().as_secs_f64();
        let pieces = crumble.pieces(elapsed, scale);
        let frame = crumble.frame(elapsed);
        let done = elapsed >= crumble.duration;
        Some((pieces, done, frame, welcome.window, welcome.size))
    });
    let Some((pieces, done, frame, window, size)) = step else { return };
    present(window, size, &frame, None);
    if !pieces.is_empty()
        && let Some(handlers) = HANDLERS.with_borrow(Clone::clone)
    {
        (handlers.scatter)(pieces);
    }
    if done {
        finish(false);
    }
}

fn finish(skipped: bool) {
    if let Some(welcome) = WELCOME.take() {
        unsafe {
            let _ = KillTimer(Some(welcome.window), TIMER);
            let _ = DestroyWindow(welcome.window);
        }
    }
    if let Some(handlers) = HANDLERS.take() {
        if skipped {
            (handlers.skipped)();
        }
        (handlers.finished)();
    }
}

impl Welcome {
    /// Whether a point, in screen pixels or the window's own, is on the button; `false` elsewhere on the card.
    fn over_button(&self, point: POINT, screen: bool) -> bool {
        let mut origin = RECT::default();
        if screen {
            unsafe {
                let _ = GetWindowRect(self.window, &mut origin);
            }
        }
        let x = (point.x - origin.left) as f32 / self.scale;
        let y = (point.y - origin.top) as f32 / self.scale;
        let button = self.painter.button;
        x >= button.left && x < button.right && y >= button.top && y < button.bottom
    }

    /// Tracks the button being held: returns whether it was released over the button, a click,
    /// and the window redrawn if the button's look changed.
    fn press(&mut self, lparam: LPARAM, phase: Phase) -> (bool, Option<Vec<u8>>) {
        let point = POINT { x: (lparam.0 & 0xffff) as i16 as i32, y: ((lparam.0 >> 16) & 0xffff) as i16 as i32 };
        let over = self.over_button(point, false);
        match phase {
            Phase::Down => self.holding = over,
            Phase::Move if !self.holding => return (false, None),
            _ => {}
        }
        let clicked = phase == Phase::Up && self.holding && over;
        if phase == Phase::Up {
            self.holding = false;
        }
        let pressed = self.holding && over;
        if pressed == self.pressed || self.crumble.is_some() {
            return (clicked, None);
        }
        self.pressed = pressed;
        (clicked, self.painter.paint(self.size, pressed).ok())
    }
}

/// Shows premultiplied pixels in the layered window, moving it to `position` if given.
fn present(window: HWND, size: (i32, i32), pixels: &[u8], position: Option<POINT>) {
    unsafe {
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size.0,
                biHeight: -size.1,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        if let Ok(bitmap) = CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len());
            let previous = SelectObject(memory, HGDIOBJ(bitmap.0));
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            let size = SIZE { cx: size.0, cy: size.1 };
            let _ = UpdateLayeredWindow(
                window,
                Some(screen),
                position.as_ref().map(|p| p as *const POINT),
                Some(&size),
                Some(memory),
                Some(&POINT::default()),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
            SelectObject(memory, previous);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
        }
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
    }
}

impl Crumble {
    fn new(base: Vec<u8>, size: (i32, i32), scale: f32, origin: POINT) -> Crumble {
        let (width, height) = (size.0 as usize, size.1 as usize);
        let columns = (WIDTH / GRAIN).ceil() as usize;
        let rows = (HEIGHT / GRAIN).ceil() as usize;
        let mut grid = vec![0.0f32; columns * rows];
        for row in 0..rows {
            for column in 0..columns {
                let x = (column as f32 + 0.5) * GRAIN;
                let y = (row as f32 + 0.5) * GRAIN;
                grid[row * columns + column] = arrival(x, y) as f32;
            }
        }
        // Each pixel takes the front of the nearest point on the card, so the shadow goes with it.
        let mut front = vec![0.0f32; width * height];
        for py in 0..height {
            let y = ((py as f32 / scale - MARGIN) / GRAIN - 0.5).clamp(0.0, (rows - 1) as f32);
            let (row, fy) = (y.floor() as usize, y.fract());
            let below = (row + 1).min(rows - 1);
            for px in 0..width {
                let x = ((px as f32 / scale - MARGIN) / GRAIN - 0.5).clamp(0.0, (columns - 1) as f32);
                let (column, fx) = (x.floor() as usize, x.fract());
                let right = (column + 1).min(columns - 1);
                let top = grid[row * columns + column] * (1.0 - fx) + grid[row * columns + right] * fx;
                let bottom = grid[below * columns + column] * (1.0 - fx) + grid[below * columns + right] * fx;
                front[py * width + px] = top * (1.0 - fy) + bottom * fy;
            }
        }

        let mut cells = Vec::new();
        let mut y = CELL / 2.0;
        while y < HEIGHT {
            let mut x = CELL / 2.0;
            while x < WIDTH {
                let px = (((MARGIN + x) * scale) as usize).min(width - 1);
                let py = (((MARGIN + y) * scale) as usize).min(height - 1);
                let i = (py * width + px) * 4;
                let alpha = base[i + 3] as f32 / 255.0;
                if alpha > 0.1 {
                    let channel = |c: u8| (c as f32 / 255.0 / alpha).min(1.0);
                    cells.push(Cell { x, y, color: [channel(base[i + 2]), channel(base[i + 1]), channel(base[i])], delay: arrival(x, y) });
                }
                x += CELL;
            }
            y += CELL;
        }
        cells.sort_by(|a, b| a.delay.total_cmp(&b.delay));
        let duration = grid.iter().copied().fold(0.0, f32::max) as f64 + FEATHER;
        let seed = SystemTime::now().duration_since(UNIX_EPOCH).map_or(1, |time| time.as_nanos() as u64) | 1;
        Crumble { started: Instant::now(), base, front, cells, next: 0, duration, origin, seed }
    }

    /// The pieces the front passed since the last call.
    fn pieces(&mut self, elapsed: f64, scale: f32) -> Vec<Piece> {
        let mut pieces = Vec::new();
        while self.next < self.cells.len() && self.cells[self.next].delay <= elapsed {
            let cell = &self.cells[self.next];
            let outward = (cell.x - WIDTH / 2.0) / WIDTH;
            let spread = self.random() * 140.0 - 70.0;
            let lift = 30.0 + self.random() * 120.0;
            let cell = &self.cells[self.next];
            pieces.push(Piece {
                x: self.origin.x as f32 + (MARGIN + cell.x) * scale,
                y: self.origin.y as f32 + (MARGIN + cell.y) * scale,
                velocity: V2::new(outward * 140.0 + spread, lift),
                color: cell.color,
            });
            self.next += 1;
        }
        pieces
    }

    /// The window with everything the front has passed faded out.
    fn frame(&self, elapsed: f64) -> Vec<u8> {
        let mut pixels = self.base.clone();
        for (pixel, &front) in pixels.chunks_exact_mut(4).zip(&self.front) {
            let alpha = ((front as f64 - elapsed) / FEATHER).clamp(0.0, 1.0) as f32;
            if alpha < 1.0 {
                for channel in pixel {
                    *channel = (*channel as f32 * alpha) as u8;
                }
            }
        }
        pixels
    }

    fn random(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 40) as f32 / (1u32 << 24) as f32
    }
}

/// The front sweeps down in about half a second, held back here and there by smooth noise
/// and roughened by a fine grain so its edge looks like crumbling snow. `x` and `y` are DIPs from the card's top left.
fn arrival(x: f32, y: f32) -> f64 {
    let from_top = y as f64 / HEIGHT as f64;
    // The noise runs in the Mac's coordinates, up from the bottom, so the front looks the same.
    let (x, y) = (x as f64, (HEIGHT - y) as f64);
    let broad = value_noise(x / 46.0, y / 46.0) * 0.65 + value_noise(x / 15.0 + 7.3, y / 15.0 + 2.1) * 0.35;
    from_top * 0.5 + broad * 0.2 + hash((x * 1.7) as i64, (y * 1.7) as i64) * 0.035
}

fn value_noise(x: f64, y: f64) -> f64 {
    let (x0, y0) = (x.floor() as i64, y.floor() as i64);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let (u, v) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let top = hash(x0, y0) + (hash(x0 + 1, y0) - hash(x0, y0)) * u;
    let bottom = hash(x0, y0 + 1) + (hash(x0 + 1, y0 + 1) - hash(x0, y0 + 1)) * u;
    top + (bottom - top) * v
}

fn hash(x: i64, y: i64) -> f64 {
    let mut n = x.wrapping_mul(374_761_393).wrapping_add(y.wrapping_mul(668_265_263)) as u32;
    n = (n ^ (n >> 13)).wrapping_mul(1_274_126_177);
    n ^= n >> 16;
    (n & 0xffff) as f64 / 65535.0
}

/// Draws the card with Direct2D into a bitmap the window shows with per-pixel alpha.
struct Painter {
    scale: f32,
    d2d: ID2D1Factory,
    text: IDWriteFactory,
    imaging: IWICImagingFactory,
    icon: IWICBitmapSource,
    /// In DIPs from the bitmap's top left.
    button: D2D_RECT_F,
}

impl Painter {
    fn new(scale: f32) -> Result<Painter> {
        unsafe {
            let imaging: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let stream = SHCreateMemStream(Some(ICON)).ok_or_else(Error::empty)?;
            let decoder = imaging.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
            let converter = imaging.CreateFormatConverter()?;
            converter.Initialize(
                &decoder.GetFrame(0)?,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )?;
            Ok(Painter {
                scale,
                d2d: D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?,
                text: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?,
                imaging,
                icon: converter.cast()?,
                button: D2D_RECT_F::default(),
            })
        }
    }

    /// Premultiplied BGRA rows, top down.
    fn paint(&mut self, size: (i32, i32), pressed: bool) -> Result<Vec<u8>> {
        unsafe {
            let bitmap = self.imaging.CreateBitmap(size.0 as u32, size.1 as u32, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)?;
            let properties = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0 * self.scale,
                dpiY: 96.0 * self.scale,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let target = self.d2d.CreateWicBitmapRenderTarget(&bitmap, &properties)?;
            target.BeginDraw();
            target.Clear(Some(&color([0.0; 3], 0.0)));
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            self.draw(&target, pressed)?;
            target.EndDraw(None, None)?;
            drop(target);

            let mut pixels = vec![0u8; (size.0 * size.1 * 4) as usize];
            bitmap.CopyPixels(std::ptr::null(), (size.0 * 4) as u32, &mut pixels)?;
            Ok(pixels)
        }
    }

    unsafe fn draw(&mut self, target: &ID2D1RenderTarget, pressed: bool) -> Result<()> {
        let card = D2D_RECT_F { left: MARGIN, top: MARGIN, right: MARGIN + WIDTH, bottom: MARGIN + HEIGHT };
        unsafe {
            // A soft shadow from stacked translucent outlines, lighter as they spread.
            let shade = target.CreateSolidColorBrush(&color([0.0; 3], 0.022), None)?;
            for step in 0..16 {
                let spread = step as f32 * 1.6;
                let rect = D2D_RECT_F {
                    left: card.left - spread,
                    top: card.top - spread + 10.0,
                    right: card.right + spread,
                    bottom: card.bottom + spread + 10.0,
                };
                target.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect, radiusX: 18.0 + spread, radiusY: 18.0 + spread }, &shade);
            }
            let inset = D2D_RECT_F { left: card.left + 0.5, top: card.top + 0.5, right: card.right - 0.5, bottom: card.bottom - 0.5 };
            let shape = D2D1_ROUNDED_RECT { rect: inset, radiusX: 18.0, radiusY: 18.0 };
            target.FillRoundedRectangle(&shape, &target.CreateSolidColorBrush(&color(BACKGROUND, 1.0), None)?);
            target.DrawRoundedRectangle(&shape, &target.CreateSolidColorBrush(&color([1.0; 3], 0.08), None)?, 1.0, None);

            let title =
                self.layout(TITLE, w!("Bahnschrift"), DWRITE_FONT_WEIGHT_LIGHT, DWRITE_FONT_STRETCH_CONDENSED, 34.0, WIDTH - 64.0, 38.5, 0.6)?;
            let body = self.layout(BODY, w!("Segoe UI"), DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STRETCH_NORMAL, 13.5, 330.0, 23.0, 0.0)?;
            let label = self.layout(BUTTON, w!("Segoe UI"), DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_FONT_STRETCH_NORMAL, 12.5, 190.0, 0.0, 1.8)?;
            label.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            label.SetMaxHeight(44.0)?;
            let (title_height, body_height) = (height(&title)?, height(&body)?);

            let icon_size = 112.0;
            let total = icon_size + 10.0 + title_height + 18.0 + body_height + 28.0 + 44.0;
            let mut y = card.top + (HEIGHT - total) / 2.0;
            let icon = target.CreateBitmapFromWicBitmap(&self.icon, None)?;
            let left = card.left + (WIDTH - icon_size) / 2.0;
            let rect = D2D_RECT_F { left, top: y, right: left + icon_size, bottom: y + icon_size };
            target.DrawBitmap(&icon, Some(&rect), 1.0, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None);
            y += icon_size + 10.0;
            target.DrawTextLayout(
                Vector2 { X: card.left + 32.0, Y: y },
                &title,
                &target.CreateSolidColorBrush(&color(INK, 1.0), None)?,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
            y += title_height + 18.0;
            let muted = target.CreateSolidColorBrush(&color(MUTED, 1.0), None)?;
            target.DrawTextLayout(Vector2 { X: card.left + (WIDTH - 330.0) / 2.0, Y: y }, &body, &muted, D2D1_DRAW_TEXT_OPTIONS_NONE);
            y += body_height + 28.0;

            let left = card.left + (WIDTH - 190.0) / 2.0;
            self.button = D2D_RECT_F { left, top: y, right: left + 190.0, bottom: y + 44.0 };
            let fill = if pressed { INK.map(|c| c * 0.88) } else { INK };
            target.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT { rect: self.button, radiusX: 2.0, radiusY: 2.0 },
                &target.CreateSolidColorBrush(&color(fill, 1.0), None)?,
            );
            target.DrawTextLayout(
                Vector2 { X: left, Y: y },
                &label,
                &target.CreateSolidColorBrush(&color(LABEL, 1.0), None)?,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
        Ok(())
    }

    fn layout(
        &self,
        text: &str,
        family: PCWSTR,
        weight: DWRITE_FONT_WEIGHT,
        stretch: DWRITE_FONT_STRETCH,
        size: f32,
        width: f32,
        line: f32,
        tracking: f32,
    ) -> Result<IDWriteTextLayout> {
        let units: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            let format = self.text.CreateTextFormat(family, None, weight, DWRITE_FONT_STYLE_NORMAL, stretch, size, w!("en-us"))?;
            format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            if line > 0.0 {
                format.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, line, line * 0.8)?;
            }
            let layout = self.text.CreateTextLayout(&units, &format, width, 1000.0)?;
            if tracking > 0.0
                && let Ok(spaced) = layout.cast::<IDWriteTextLayout1>()
            {
                spaced.SetCharacterSpacing(0.0, tracking, 0.0, DWRITE_TEXT_RANGE { startPosition: 0, length: units.len() as u32 })?;
            }
            Ok(layout)
        }
    }
}

fn height(layout: &IDWriteTextLayout) -> Result<f32> {
    let mut metrics = DWRITE_TEXT_METRICS::default();
    unsafe { layout.GetMetrics(&mut metrics)? };
    Ok(metrics.height)
}

fn color(rgb: [f32; 3], alpha: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: rgb[0], g: rgb[1], b: rgb[2], a: alpha }
}
