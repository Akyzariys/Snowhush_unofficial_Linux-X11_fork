use std::collections::{HashMap, HashSet};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageData, ClientMessageEvent, ConfigureWindowAux, ConnectionExt,
    EventMask, MapState, StackMode, Window,
};
use x11rb::rust_connection::RustConnection;

use snowhush::geometry::Rect;
use snowhush::simulation::WindowSurface;

use crate::log;

/// A rectangle in root (physical screen) pixels, y pointing down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

const MAX_FRAME_EXTENT: u32 = 4096;
const MAX_WINDOW_SIZE: i32 = 32_768;

impl Bounds {
    pub fn right(&self) -> i32 {
        self.x.saturating_add(self.width)
    }

    pub fn bottom(&self) -> i32 {
        self.y.saturating_add(self.height)
    }
}

/// Shell windows that are not solid surfaces: the desktop and the panel.
struct Atoms {
    client_list_stacking: Atom,
    current_desktop: Atom,
    wm_desktop: Atom,
    wm_state: Atom,
    state_hidden: Atom,
    state_fullscreen: Atom,
    state_above: Atom,
    state_maximized_vert: Atom,
    state_maximized_horz: Atom,
    state_skip_taskbar: Atom,
    state_skip_pager: Atom,
    state_sticky: Atom,
    wm_window_type: Atom,
    type_desktop: Atom,
    type_dock: Atom,
    type_tooltip: Atom,
    type_menu: Atom,
    type_dropdown_menu: Atom,
    type_popup_menu: Atom,
    type_notification: Atom,
    type_combo: Atom,
    type_splash: Atom,
    frame_extents: Atom,
    wm_pid: Atom,
    workarea: Atom,
}

impl Atoms {
    fn new(connection: &RustConnection) -> Result<Atoms, String> {
        let atom = |name: &str| -> Result<Atom, String> {
            connection
                .intern_atom(false, name.as_bytes())
                .map_err(|error| error.to_string())?
                .reply()
                .map_err(|error| error.to_string())
                .map(|reply| reply.atom)
        };
        Ok(Atoms {
            client_list_stacking: atom("_NET_CLIENT_LIST_STACKING")?,
            current_desktop: atom("_NET_CURRENT_DESKTOP")?,
            wm_desktop: atom("_NET_WM_DESKTOP")?,
            wm_state: atom("_NET_WM_STATE")?,
            state_hidden: atom("_NET_WM_STATE_HIDDEN")?,
            state_fullscreen: atom("_NET_WM_STATE_FULLSCREEN")?,
            state_above: atom("_NET_WM_STATE_ABOVE")?,
            state_maximized_vert: atom("_NET_WM_STATE_MAXIMIZED_VERT")?,
            state_maximized_horz: atom("_NET_WM_STATE_MAXIMIZED_HORZ")?,
            state_skip_taskbar: atom("_NET_WM_STATE_SKIP_TASKBAR")?,
            state_skip_pager: atom("_NET_WM_STATE_SKIP_PAGER")?,
            state_sticky: atom("_NET_WM_STATE_STICKY")?,
            wm_window_type: atom("_NET_WM_WINDOW_TYPE")?,
            type_desktop: atom("_NET_WM_WINDOW_TYPE_DESKTOP")?,
            type_dock: atom("_NET_WM_WINDOW_TYPE_DOCK")?,
            type_tooltip: atom("_NET_WM_WINDOW_TYPE_TOOLTIP")?,
            type_menu: atom("_NET_WM_WINDOW_TYPE_MENU")?,
            type_dropdown_menu: atom("_NET_WM_WINDOW_TYPE_DROPDOWN_MENU")?,
            type_popup_menu: atom("_NET_WM_WINDOW_TYPE_POPUP_MENU")?,
            type_notification: atom("_NET_WM_WINDOW_TYPE_NOTIFICATION")?,
            type_combo: atom("_NET_WM_WINDOW_TYPE_COMBO")?,
            type_splash: atom("_NET_WM_WINDOW_TYPE_SPLASH")?,
            frame_extents: atom("_NET_FRAME_EXTENTS")?,
            wm_pid: atom("_NET_WM_PID")?,
            workarea: atom("_NET_WORKAREA")?,
        })
    }
}

struct WindowInfo {
    id: isize,
    class: String,
    /// The visible frame in physical root pixels, without the window manager's shadow.
    rect: Bounds,
    radius: f32,
    cover_only: bool,
    topmost: bool,
    fullscreen: bool,
}

/// The windows on screen, bottom to top, re-read twice a second, or twenty times a second while
/// something is being dragged, so snow can follow window moves and settle on their tops.
pub struct Tracker {
    pub connection: RustConnection,
    pub root: Window,
    atoms: Atoms,
    windows: Vec<WindowInfo>,
    docks: Vec<Bounds>,
    listed_at: f64,
    now: f64,
    last_rects: HashMap<isize, Bounds>,
    last_desktops: HashMap<isize, u32>,
    elsewhere: HashSet<isize>,
    current_desktop: u32,
    moving_until: f64,
    started_moving: bool,
    pub generation: u64,
    seen: HashSet<isize>,
    pub listings: u32,
    pub cost: f64,
}

impl Tracker {
    pub fn new() -> Result<Tracker, String> {
        let (connection, screen) =
            x11rb::connect(None).map_err(|error| format!("connecting to the display: {error}"))?;
        let root = connection.setup().roots[screen].root;
        let atoms = Atoms::new(&connection)?;
        let current_desktop = property_u32(&connection, root, atoms.current_desktop)
            .first()
            .copied()
            .unwrap_or(0);
        Ok(Tracker {
            connection,
            root,
            atoms,
            windows: Vec::new(),
            docks: Vec::new(),
            listed_at: 0.0,
            now: 0.0,
            last_rects: HashMap::new(),
            last_desktops: HashMap::new(),
            elsewhere: HashSet::new(),
            current_desktop,
            moving_until: 0.0,
            started_moving: false,
            generation: 0,
            seen: HashSet::new(),
            listings: 0,
            cost: 0.0,
        })
    }

    /// Re-reads the window list if anything may have changed; returns whether it did.
    pub fn refresh(&mut self, now: f64) -> bool {
        self.now = now;
        let interval = if now < self.moving_until { 0.05 } else { 0.5 };
        if now - self.listed_at < interval {
            return false;
        }
        self.listings += 1;
        self.listed_at = now;
        let began = std::time::Instant::now();
        self.list();
        self.cost += began.elapsed().as_secs_f64();
        self.generation += 1;
        true
    }

    /// Whether a window is being dragged or resized right now.
    pub fn moving(&self) -> bool {
        self.moving_until > self.now
    }

    /// Whether a drag or resize started since the last call.
    pub fn take_started_moving(&mut self) -> bool {
        std::mem::take(&mut self.started_moving)
    }

    /// The windows on one display that snow can settle on, in overlay coordinates.
    /// `area` is the display's work area in physical pixels, `size` the overlay's size in DIPs.
    pub fn surfaces(&self, area: &Bounds, size: (u32, u32)) -> Vec<WindowSurface> {
        let (width, height) = (size.0 as f32, size.1 as f32);
        let sx = width / area.width.max(1) as f32;
        let sy = height / area.height.max(1) as f32;
        let mut blockers: Vec<Rect> = Vec::new();
        let mut surfaces = Vec::new();
        for window in self.windows.iter().rev() {
            let r = &window.rect;
            if r.right() <= area.x
                || r.x >= area.right()
                || r.bottom() <= area.y
                || r.y >= area.bottom()
            {
                continue;
            }
            let frame = Rect {
                x: (r.x - area.x) as f32 * sx,
                y: (area.bottom() - r.bottom()) as f32 * sy,
                width: r.width as f32 * sx,
                height: r.height as f32 * sy,
            };
            let room_above = (r.y - area.y) as f32 * sy >= 6.0 && frame.max_y() < height;
            let on_screen = frame.max_x() > 0.0 && frame.min_x() < width && frame.max_y() > 0.0;
            let fills_screen = frame.width * frame.height >= width * height * 0.9;
            if room_above
                && on_screen
                && !fills_screen
                && !window.cover_only
                && frame.width >= 80.0
                && frame.height >= 40.0
            {
                let radius = window.radius.min(frame.width / 2.0).min(frame.height / 2.0);
                let region = Rect {
                    x: frame.x,
                    y: frame.max_y() - radius,
                    width: frame.width,
                    height: 40.0 + radius,
                };
                surfaces.push(WindowSurface {
                    id: window.id,
                    frame,
                    blockers: blockers
                        .iter()
                        .filter(|b| intersects(b, &region))
                        .copied()
                        .collect(),
                    corner_radius: radius,
                    corner_exponent: 2.0,
                });
            }
            blockers.push(frame);
        }
        surfaces
    }

    /// The full-screen window covering the display with these bounds, if any.
    pub fn covering(&self, bounds: &Bounds) -> Option<&str> {
        for window in self.windows.iter().rev() {
            let r = &window.rect;
            if r.right() <= bounds.x
                || r.x >= bounds.right()
                || r.bottom() <= bounds.y
                || r.y >= bounds.bottom()
            {
                continue;
            }
            let fills = r.x <= bounds.x
                && r.y <= bounds.y
                && r.right() >= bounds.right()
                && r.bottom() >= bounds.bottom();
            if fills && window.fullscreen {
                return Some(&window.class);
            }
            if !window.topmost {
                return None;
            }
        }
        None
    }

    /// Whether a window that left the list moved to another virtual desktop, rather than closing or hiding.
    pub fn elsewhere(&self, id: isize) -> bool {
        self.elsewhere.contains(&id)
    }

    /// The work area of a display: the display minus the panel and any other docked bars.
    pub fn work_area(&self, bounds: Bounds) -> Bounds {
        let mut work = bounds;
        if let Some(area) = property_u32(&self.connection, self.root, self.atoms.workarea).get(..4)
        {
            // _NET_WORKAREA is global to the virtual screen. Treat only a rectangle that actually
            // fits inside the root as valid, and do the arithmetic in i64 so a malformed property
            // from another X client cannot overflow the overlay geometry.
            let (ax, ay, aw, ah) = (
                area[0] as i64,
                area[1] as i64,
                area[2] as i64,
                area[3] as i64,
            );
            let root_width = self
                .connection
                .setup()
                .roots
                .iter()
                .find(|root| root.root == self.root)
                .map_or(0i64, |root| root.width_in_pixels as i64);
            let root_height = self
                .connection
                .setup()
                .roots
                .iter()
                .find(|root| root.root == self.root)
                .map_or(0i64, |root| root.height_in_pixels as i64);
            if aw > 0
                && ah > 0
                && ax >= 0
                && ay >= 0
                && ax + aw <= root_width
                && ay + ah <= root_height
            {
                let right = root_width - (ax + aw);
                let bottom = root_height - (ay + ah);
                if bounds.x <= 0 && ax > 0 {
                    let cut = ax as i32;
                    work.x = work.x.saturating_add(cut);
                    work.width = work.width.saturating_sub(cut);
                }
                if bounds.y <= 0 && ay > 0 {
                    let cut = ay as i32;
                    work.y = work.y.saturating_add(cut);
                    work.height = work.height.saturating_sub(cut);
                }
                if bounds.right() >= root_width as i32 && right > 0 {
                    work.width = work.width.saturating_sub(right as i32);
                }
                if bounds.bottom() >= root_height as i32 && bottom > 0 {
                    work.height = work.height.saturating_sub(bottom as i32);
                }
            }
        }
        for dock in &self.docks {
            let horizontally = dock.width as f32 >= bounds.width as f32 * 0.6;
            let vertically = dock.height as f32 >= bounds.height as f32 * 0.6;
            let spans_x = dock.right() > bounds.x && dock.x < bounds.right();
            let spans_y = dock.bottom() > bounds.y && dock.y < bounds.bottom();
            if horizontally && spans_x && dock.y <= bounds.y + 2 && dock.bottom() > bounds.y {
                let cut = dock.bottom().saturating_sub(work.y);
                if cut > 0 {
                    work.y = work.y.saturating_add(cut);
                    work.height = work.height.saturating_sub(cut);
                }
            } else if horizontally
                && spans_x
                && dock.bottom() >= bounds.bottom() - 2
                && dock.y < bounds.bottom()
            {
                let cut = work.bottom().saturating_sub(dock.y);
                if cut > 0 {
                    work.height = work.height.saturating_sub(cut);
                }
            } else if vertically && spans_y && dock.x <= bounds.x + 2 && dock.right() > bounds.x {
                let cut = dock.right().saturating_sub(work.x);
                if cut > 0 {
                    work.x = work.x.saturating_add(cut);
                    work.width = work.width.saturating_sub(cut);
                }
            } else if vertically
                && spans_y
                && dock.right() >= bounds.right() - 2
                && dock.x < bounds.right()
            {
                let cut = work.right().saturating_sub(dock.x);
                if cut > 0 {
                    work.width = work.width.saturating_sub(cut);
                }
            }
        }
        work.width = work.width.max(1);
        work.height = work.height.max(1);
        work
    }

    /// The pointer in root pixels.
    pub fn cursor(&self) -> Option<(i32, i32)> {
        let reply = self
            .connection
            .query_pointer(self.root)
            .ok()?
            .reply()
            .ok()?;
        Some((reply.root_x as i32, reply.root_y as i32))
    }

    /// Empties the pointer's input region, so clicks and drags reach the windows below. The window
    /// manager's own hint does not always survive mapping; the shape does.
    pub fn set_input_transparent(&self, window: u32) {
        use x11rb::protocol::shape::{ConnectionExt as ShapeExt, SK, SO};
        let result = self.connection.shape_rectangles(
            SO::SET,
            SK::INPUT,
            x11rb::protocol::xproto::ClipOrdering::UNSORTED,
            window,
            0,
            0,
            &[],
        );
        match result {
            Ok(_) => {
                let _ = self.connection.flush();
            }
            Err(error) => log!("input passthrough failed: {error}"),
        }
    }

    /// An override-redirect overlay is outside the window manager's stacking rules, so it is raised
    /// by hand whenever the window list is read.
    pub fn raise_overlay(&self, window: u32) {
        let aux = ConfigureWindowAux::new().stack_mode(StackMode::ABOVE);
        let _ = self.connection.configure_window(window, &aux);
        let _ = self.connection.flush();
    }

    /// Keeps one of our overlays above normal windows, out of the taskbar and on every desktop.
    pub fn decorate_overlay(&self, window: u32) {
        let state = [
            self.atoms.state_above,
            self.atoms.state_skip_taskbar,
            self.atoms.state_skip_pager,
            self.atoms.state_sticky,
        ];
        for atom in state {
            let event = ClientMessageEvent::new(
                32,
                window,
                self.atoms.wm_state,
                ClientMessageData::from([1, atom, 0, 1, 0]),
            );
            let _ = self.connection.send_event(
                false,
                self.root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                event,
            );
        }
        let _ = self.connection.flush();
    }

    fn list(&mut self) {
        let roots = property_u32(&self.connection, self.root, self.atoms.client_list_stacking);
        let roots = if roots.is_empty() {
            self.children()
        } else {
            roots
        };
        self.current_desktop =
            property_u32(&self.connection, self.root, self.atoms.current_desktop)
                .first()
                .copied()
                .unwrap_or(0);
        let own = std::process::id();
        let mut windows = Vec::with_capacity(roots.len());
        let mut docks = Vec::new();
        let mut rects = HashMap::new();
        let mut desktops = HashMap::new();

        for id in roots {
            let Some(window) = self.describe(id, own) else {
                continue;
            };
            if window.dock {
                docks.push(window.info.rect);
                continue;
            }
            if window.desktop_away {
                self.elsewhere.insert(window.info.id);
                continue;
            }
            self.elsewhere.remove(&window.info.id);
            rects.insert(window.info.id, window.info.rect);
            desktops.insert(window.info.id, window.desktop);
            windows.push(window.info);
        }

        // A window that left the list went somewhere: another desktop counts as elsewhere,
        // so its snow is dropped rather than carried to another display.
        let present: HashSet<isize> = rects.keys().copied().collect();
        for (&id, &desktop) in &self.last_desktops {
            if present.contains(&id) {
                continue;
            }
            if desktop != self.current_desktop && window_exists(&self.connection, id as u32) {
                self.elsewhere.insert(id);
            }
        }

        let moved = self
            .last_rects
            .iter()
            .any(|(id, before)| rects.get(id).is_some_and(|after| before != after));
        if moved {
            self.moving_until = self.now + 0.12;
            self.started_moving = true;
        }
        for id in rects.keys() {
            if self.seen.insert(*id) {
                if let Some(window) = windows.iter().find(|window| window.id == *id) {
                    let r = &window.rect;
                    log!(
                        "window {} of pid: {}x{} at {},{}",
                        window.class,
                        r.width,
                        r.height,
                        r.x,
                        r.y
                    );
                }
            }
        }
        if self.seen.len() > 4096 {
            self.seen.clear();
        }
        // Destroyed windows on another desktop would otherwise stay in this set forever; X reuses
        // window IDs, so a stale entry could later be mistaken for a live window.
        if self.elsewhere.len() > 256 {
            let stale: Vec<isize> = self
                .elsewhere
                .iter()
                .copied()
                .filter(|&id| !window_exists(&self.connection, id as u32))
                .collect();
            for id in stale {
                self.elsewhere.remove(&id);
            }
        }
        self.last_rects = rects;
        self.last_desktops = desktops;
        self.windows = windows;
        self.docks = docks;
    }

    fn describe(&self, id: u32, own: u32) -> Option<Described> {
        let attributes = self
            .connection
            .get_window_attributes(id)
            .ok()?
            .reply()
            .ok()?;
        if attributes.map_state != MapState::VIEWABLE || attributes.override_redirect {
            return None;
        }
        let pid = property_u32(&self.connection, id, self.atoms.wm_pid)
            .first()
            .copied();
        if pid == Some(own) {
            return None;
        }
        let types = property_u32(&self.connection, id, self.atoms.wm_window_type);
        let kind = types.first().copied();
        if kind == Some(self.atoms.type_desktop) {
            return None;
        }
        let geometry = self.connection.get_geometry(id).ok()?.reply().ok()?;
        if geometry.width < 2 || geometry.height < 2 {
            return None;
        }
        let translated = self
            .connection
            .translate_coordinates(id, self.root, 0, 0)
            .ok()?
            .reply()
            .ok()?;
        let extents = property_u32(&self.connection, id, self.atoms.frame_extents);
        // A window manager supplies small values here. Another X client can write anything, and a
        // four-billion-pixel frame would make the snow simulation allocate an enormous layer.
        let extent = |value: u32| value.min(MAX_FRAME_EXTENT) as i32;
        let (left, right, top, bottom) = if extents.len() >= 4 {
            (
                extent(extents[0]),
                extent(extents[1]),
                extent(extents[2]),
                extent(extents[3]),
            )
        } else {
            (0, 0, 0, 0)
        };
        let rect = Bounds {
            x: (translated.dst_x as i32).saturating_sub(left),
            y: (translated.dst_y as i32).saturating_sub(top),
            width: (geometry.width as i32)
                .saturating_add(left)
                .saturating_add(right)
                .clamp(1, MAX_WINDOW_SIZE),
            height: (geometry.height as i32)
                .saturating_add(top)
                .saturating_add(bottom)
                .clamp(1, MAX_WINDOW_SIZE),
        };
        let states = property_u32(&self.connection, id, self.atoms.wm_state);
        let has = |atom: Atom| states.contains(&atom);
        if has(self.atoms.state_hidden) {
            return None;
        }
        let desktop = property_u32(&self.connection, id, self.atoms.wm_desktop)
            .first()
            .copied()
            .unwrap_or(0xFFFF_FFFF);
        let sticky = desktop == 0xFFFF_FFFF || has(self.atoms.state_sticky);
        let desktop_away = !sticky && desktop != self.current_desktop;
        let fullscreen = has(self.atoms.state_fullscreen);
        let zoomed = has(self.atoms.state_maximized_vert) && has(self.atoms.state_maximized_horz);
        let class = window_class(&self.connection, id);
        let cover_only = matches!(
            kind,
            Some(kind)
                if kind == self.atoms.type_tooltip
                    || kind == self.atoms.type_menu
                    || kind == self.atoms.type_dropdown_menu
                    || kind == self.atoms.type_popup_menu
                    || kind == self.atoms.type_notification
                    || kind == self.atoms.type_combo
                    || kind == self.atoms.type_splash
        );
        let dock = kind == Some(self.atoms.type_dock);
        Some(Described {
            info: WindowInfo {
                id: id as isize,
                class,
                rect,
                radius: if fullscreen || zoomed { 0.0 } else { 8.0 },
                cover_only,
                topmost: has(self.atoms.state_above),
                fullscreen,
            },
            desktop,
            dock,
            desktop_away,
        })
    }

    fn children(&self) -> Vec<u32> {
        let Ok(cookie) = self.connection.query_tree(self.root) else {
            return Vec::new();
        };
        let Ok(reply) = cookie.reply() else {
            return Vec::new();
        };
        reply.children
    }
}

struct Described {
    info: WindowInfo,
    desktop: u32,
    dock: bool,
    desktop_away: bool,
}

fn property_u32(connection: &RustConnection, window: Window, atom: Atom) -> Vec<u32> {
    let Ok(cookie) = connection.get_property(false, window, atom, AtomEnum::ANY, 0, 4096) else {
        return Vec::new();
    };
    let Ok(reply) = cookie.reply() else {
        return Vec::new();
    };
    reply
        .value32()
        .map(|values| values.collect())
        .unwrap_or_default()
}

fn property_string(connection: &RustConnection, window: Window, atom: Atom) -> String {
    let Ok(cookie) = connection.get_property(false, window, atom, AtomEnum::ANY, 0, 4096) else {
        return String::new();
    };
    let Ok(reply) = cookie.reply() else {
        return String::new();
    };
    String::from_utf8_lossy(&reply.value).into_owned()
}

fn window_class(connection: &RustConnection, window: Window) -> String {
    let text = property_string(connection, window, Atom::from(AtomEnum::WM_CLASS));
    // WM_CLASS comes from another client and may contain control characters. Keep the log readable
    // instead of allowing a class name to forge extra log lines or terminal escape sequences.
    text.split('\0')
        .filter(|part| !part.is_empty())
        .nth(1)
        .unwrap_or("")
        .chars()
        .filter(|character| !character.is_control())
        .take(80)
        .collect()
}

fn window_exists(connection: &RustConnection, window: Window) -> bool {
    connection
        .get_window_attributes(window)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .is_some()
}

fn intersects(a: &Rect, b: &Rect) -> bool {
    a.min_x() < b.max_x() && b.min_x() < a.max_x() && a.min_y() < b.max_y() && b.min_y() < a.max_y()
}
