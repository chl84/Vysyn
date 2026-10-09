use crate::{
    animation::Animation,
    decode::Decoded,
    limits::Limits,
    loader::{Event, Loader},
    navigation,
    render::{Draw, Renderer},
    view::{Point, View},
};
use anyhow::{Result, bail};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    keyboard::{Key, NamedKey},
    window::{Fullscreen, Window, WindowId},
};

pub fn report(message: &str) {
    eprintln!("vysyn: {message}");
    #[cfg(target_os = "windows")]
    {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(std::env::temp_dir().join("vysyn.log"))
        {
            let _ = writeln!(file, "{message}");
        }
    }
}

pub struct Options {
    pub path: Option<PathBuf>,
    pub trace: bool,
    pub exit_after: Option<Duration>,
}

pub fn run(options: Options, limits: Limits) -> Result<()> {
    let started = Instant::now();
    let event_loop = EventLoop::<Event>::with_user_event().build()?;
    let mut app = App {
        proxy: event_loop.create_proxy(),
        window: None,
        renderer: None,
        loader: None,
        limits,
        options,
        view: View::default(),
        cursor: Point::default(),
        dragging: false,
        clicks: DoubleClick::default(),
        image: None,
        delays: Vec::new(),
        animation: Animation::default(),
        generation: 0,
        current: None,
        files: Vec::new(),
        directory: None,
        scanned: false,
        presented_generation: None,
        pending_scan: false,
        retry_at: None,
        started,
        requested: Instant::now(),
        loaded_generation: None,
        first_visible: false,
        hidden: false,
        failure: None,
    };
    event_loop.run_app(&mut app)?;
    if let Some(e) = app.failure {
        bail!("{e}");
    }
    if app.options.exit_after.is_some() && app.current.is_some() && !app.first_visible {
        bail!("smoke test ended before an image was presented");
    }
    Ok(())
}

struct App {
    proxy: EventLoopProxy<Event>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    loader: Option<Loader>,
    limits: Limits,
    options: Options,
    view: View,
    cursor: Point,
    dragging: bool,
    clicks: DoubleClick,
    image: Option<Arc<Decoded>>,
    delays: Vec<Duration>,
    animation: Animation,
    generation: u64,
    current: Option<PathBuf>,
    files: Vec<PathBuf>,
    directory: Option<PathBuf>,
    scanned: bool,
    presented_generation: Option<u64>,
    pending_scan: bool,
    retry_at: Option<Instant>,
    started: Instant,
    requested: Instant,
    first_visible: bool,
    hidden: bool,
    failure: Option<String>,
    loaded_generation: Option<u64>,
}

impl App {
    fn trace(&self, message: &str) {
        if self.options.trace {
            report(message);
        }
    }
    fn redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: impl std::fmt::Display) {
        self.failure = Some(error.to_string());
        event_loop.exit();
    }
    fn open(&mut self, path: PathBuf) {
        self.clicks = DoubleClick::default();
        self.requested = Instant::now();
        self.generation = self.loader.as_ref().map_or(0, |l| l.open(path.clone()));
        self.current = Some(path);
        // Keep the displayed image until the replacement is ready.
        self.pending_scan = false;
    }
    fn click_position(&self, position: Point) -> Point {
        let scale = self.window.as_ref().map_or(1.0, |w| w.scale_factor());
        Point {
            x: position.x / scale,
            y: position.y / scale,
        }
    }
    fn neighbors(&self) -> Vec<PathBuf> {
        let Some(current) = &self.current else {
            return Vec::new();
        };
        [1, -1]
            .into_iter()
            .filter_map(|d| navigation::neighbor(&self.files, current, d))
            .filter(|p| p != current)
            .collect()
    }
    fn after_present(&mut self) {
        if self.presented_generation == Some(self.generation)
            || self.loaded_generation != Some(self.generation)
        {
            return;
        }
        self.presented_generation = Some(self.generation);
        let elapsed = self.requested.elapsed().as_secs_f64() * 1000.0;
        let ram = self.loader.as_ref().map_or(0, Loader::memory_used);
        let gpu = self.renderer.as_ref().map_or(0, |r| r.gpu_bytes);
        let current_gpu = self.renderer.as_ref().map_or(0, Renderer::current_bytes);
        self.trace(&format!("present generation={} navigation_ms={elapsed:.3} buffers_bytes={ram} gpu_image_bytes={current_gpu} gpu_cache_bytes={gpu}", self.generation));
        if !self.first_visible {
            self.first_visible = true;
            self.trace(&format!(
                "first_present_ms={:.3}",
                self.started.elapsed().as_secs_f64() * 1000.0
            ));
        }
        if let (Some(loader), Some(directory)) = (&self.loader, &self.directory) {
            if self.pending_scan {
                loader.scan(self.generation, directory.clone());
                self.pending_scan = false;
            } else if self.scanned {
                loader.preload(self.generation, self.neighbors());
            }
        }
    }
}

impl ApplicationHandler<Event> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            self.redraw();
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("Vysyn")
            .with_decorations(false)
            .with_transparent(false)
            .with_inner_size(PhysicalSize::new(1000, 700));
        let window = match event_loop.create_window(attributes) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.fail(event_loop, e);
                return;
            }
        };
        self.trace(&format!(
            "window_ms={:.3}",
            self.started.elapsed().as_secs_f64() * 1000.0
        ));
        let renderer = match pollster::block_on(Renderer::new(
            window.clone(),
            event_loop.owned_display_handle(),
            &self.limits,
        )) {
            Ok(r) => r,
            Err(e) => {
                self.fail(event_loop, format!("{e:#}"));
                return;
            }
        };
        self.trace(&format!(
            "gpu_ready_ms={:.3} backend={:?} adapter={}",
            self.started.elapsed().as_secs_f64() * 1000.0,
            renderer.backend,
            renderer.adapter_name
        ));
        let loader = match Loader::new(self.limits.clone(), renderer.target, self.proxy.clone()) {
            Ok(l) => l,
            Err(e) => {
                self.fail(event_loop, e);
                return;
            }
        };
        let size = window.inner_size();
        self.view.resize([size.width, size.height]);
        self.window = Some(window);
        self.renderer = Some(renderer);
        self.loader = Some(loader);
        if let Some(path) = self.options.path.take() {
            self.open(path);
        }
        self.redraw();
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: Event) {
        match event {
            Event::Loaded {
                generation,
                path,
                result,
                elapsed_ms,
                cached,
            } if generation == self.generation => {
                match result {
                    Ok(image) => {
                        for warning in &image.warnings {
                            report(warning);
                        }
                        let gpu_cached = match &mut self.renderer {
                            Some(renderer) => match renderer.show_image(&image) {
                                Ok(cached) => cached,
                                Err(e) => {
                                    self.fail(event_loop, e);
                                    return;
                                }
                            },
                            None => false,
                        };
                        self.view.set_image(image.original);
                        self.clicks = DoubleClick::default();
                        self.current = Some(path.clone());
                        let directory = path.parent().map(std::path::Path::to_path_buf);
                        if directory != self.directory {
                            self.files.clear();
                            self.directory = directory;
                            self.scanned = false;
                        }
                        self.pending_scan = !self.scanned;
                        self.delays = image.frames.iter().map(|f| f.delay).collect();
                        self.animation =
                            Animation::start(&self.delays, image.loops, Instant::now());
                        self.trace(&format!(
                            "loaded_ms={elapsed_ms:.3} decode_ms={:.3} cached={cached} gpu_cached={gpu_cached}",
                            image.elapsed.as_secs_f64() * 1000.0
                        ));
                        self.image = Some(image);
                        self.loaded_generation = Some(generation);
                        self.redraw();
                    }
                    Err(e) => {
                        report(&format!("{}: {e}", path.display()));
                        if self.options.exit_after.is_some() {
                            self.fail(event_loop, e);
                            return;
                        }
                        // Refresh the directory after deleted/invalid files; keep the previous image visible.
                        if let (Some(loader), Some(directory)) = (&self.loader, &self.directory) {
                            loader.scan(self.generation, directory.clone());
                        }
                    }
                }
            }
            Event::Directory {
                generation,
                directory,
                result,
            } if generation == self.generation => match result {
                Ok(files) => {
                    self.trace(&format!("directory_files={}", files.len()));
                    self.directory = Some(directory);
                    self.files = files;
                    self.scanned = true;
                    if self.presented_generation == Some(generation)
                        && let Some(loader) = &self.loader
                    {
                        loader.preload(generation, self.neighbors());
                    }
                }
                Err(e) => report(&e),
            },
            _ => {}
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.window.as_ref().is_none_or(|w| w.id() != id) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.view.resize([size.width, size.height]);
                if let Some(r) = &mut self.renderer
                    && let Err(e) = r.resize(size.width, size.height)
                {
                    self.fail(event_loop, e);
                    return;
                }
                self.redraw();
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                self.clicks = DoubleClick::default();
                if let Some(window) = &self.window {
                    let size = window.inner_size();
                    self.view.resize([size.width, size.height]);
                    if let Some(r) = &mut self.renderer
                        && let Err(e) = r.resize(size.width, size.height)
                    {
                        self.fail(event_loop, e);
                        return;
                    }
                }
                self.redraw();
            }
            WindowEvent::Occluded(hidden) => {
                self.hidden = hidden;
                if !hidden {
                    self.redraw();
                }
            }
            WindowEvent::Focused(false) | WindowEvent::CursorLeft { .. } => {
                self.dragging = false;
                self.clicks = DoubleClick::default();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let next = Point {
                    x: position.x,
                    y: position.y,
                };
                self.clicks.moved(self.click_position(next));
                if self.dragging {
                    self.view.drag(Point {
                        x: next.x - self.cursor.x,
                        y: next.y - self.cursor.y,
                    });
                    self.redraw();
                }
                self.cursor = next;
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                self.dragging = state == ElementState::Pressed;
                if self.dragging {
                    if self
                        .clicks
                        .press(Instant::now(), self.click_position(self.cursor))
                    {
                        self.view.fit();
                        self.redraw();
                    }
                } else {
                    self.clicks.release(Instant::now());
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.clicks = DoubleClick::default();
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, y) => f64::from(y),
                    MouseScrollDelta::PixelDelta(p) => p.y / 120.0,
                };
                self.view
                    .zoom(1.2_f64.powf(steps.clamp(-20.0, 20.0)), self.cursor);
                self.redraw();
            }
            WindowEvent::DroppedFile(path) => {
                self.trace(&format!("dropped_file={}", path.display()));
                self.open(path);
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.clicks = DoubleClick::default();
                self.trace(&format!("key={:?}", event.logical_key));
                match event.logical_key {
                    Key::Named(NamedKey::Escape) => event_loop.exit(),
                    Key::Named(NamedKey::F11) if !event.repeat => {
                        if let Some(w) = &self.window {
                            w.set_fullscreen(if w.fullscreen().is_some() {
                                None
                            } else {
                                Some(Fullscreen::Borderless(None))
                            });
                        }
                    }
                    Key::Named(key @ (NamedKey::ArrowLeft | NamedKey::ArrowRight)) => {
                        if let Some(current) = &self.current
                            && let Some(path) = navigation::neighbor(
                                &self.files,
                                current,
                                if key == NamedKey::ArrowRight { 1 } else { -1 },
                            )
                        {
                            self.open(path);
                        }
                    }
                    Key::Character(value) => {
                        match value.as_str() {
                            "+" | "=" => self.view.zoom(1.2, self.view.center()),
                            "-" => self.view.zoom(1.0 / 1.2, self.view.center()),
                            "0" => self.view.fit(),
                            _ => return,
                        }
                        self.redraw();
                    }
                    _ => {}
                }
            }
            WindowEvent::RedrawRequested => {
                let outcome = match (&mut self.renderer, &self.window) {
                    (Some(renderer), Some(window)) => renderer.draw(&self.view, window),
                    _ => return,
                };
                match outcome {
                    Ok(Draw::Presented) => {
                        self.retry_at = None;
                        self.after_present();
                    }
                    Ok(Draw::Retry) => {
                        self.retry_at = Some(Instant::now() + Duration::from_millis(100))
                    }
                    Ok(Draw::Occluded) => self.retry_at = None,
                    Err(e) => self.fail(event_loop, e),
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if !self.hidden && self.animation.advance(&self.delays, now) {
            if let (Some(image), Some(renderer)) = (&self.image, &mut self.renderer)
                && let Err(e) = renderer.upload(&image.frames[self.animation.frame].pixels)
            {
                self.fail(event_loop, e);
                return;
            }
            self.redraw();
        }
        if self.retry_at.is_some_and(|deadline| now >= deadline) {
            self.retry_at = None;
            self.redraw();
        }
        let end = self.options.exit_after.map(|d| self.started + d);
        if end.is_some_and(|deadline| now >= deadline) {
            event_loop.exit();
            return;
        }
        let animation = if self.hidden {
            None
        } else {
            self.animation.deadline
        };
        let next = [animation, self.retry_at, end].into_iter().flatten().min();
        event_loop.set_control_flow(next.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        if let Some(loader) = &mut self.loader {
            loader.stop();
        }
    }
}

const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Copy)]
struct Click {
    at: Instant,
    position: Point,
}

impl Click {
    fn near(self, position: Point) -> bool {
        // Logical pixels keep the tolerance consistent across display scales.
        (self.position.x - position.x).hypot(self.position.y - position.y) <= 5.0
    }
}

#[derive(Default)]
struct DoubleClick {
    pressed: Option<Click>,
    previous: Option<Click>,
}

impl DoubleClick {
    fn press(&mut self, now: Instant, position: Point) -> bool {
        let double = self.previous.take().is_some_and(|click| {
            now.duration_since(click.at) <= DOUBLE_CLICK_INTERVAL && click.near(position)
        });
        // Consume both clicks so a third click cannot retrigger the same pair.
        self.pressed = (!double).then_some(Click { at: now, position });
        double
    }

    fn moved(&mut self, position: Point) {
        if self.pressed.is_some_and(|click| !click.near(position)) {
            self.pressed = None;
        }
    }

    fn release(&mut self, now: Instant) {
        // A drag or a long-held button cannot start a double-click.
        self.previous = self
            .pressed
            .take()
            .filter(|click| now.duration_since(click.at) <= DOUBLE_CLICK_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click(clicks: &mut DoubleClick, at: Instant, position: Point) -> bool {
        let double = clicks.press(at, position);
        clicks.release(at + Duration::from_millis(20));
        double
    }

    #[test]
    fn rapid_nearby_clicks_trigger_once_per_pair() {
        let mut clicks = DoubleClick::default();
        let at = Instant::now();
        assert!(!click(&mut clicks, at, Point::default()));
        assert!(click(
            &mut clicks,
            at + Duration::from_millis(180),
            Point { x: 3.0, y: 4.0 }
        ));
        assert!(!click(
            &mut clicks,
            at + Duration::from_millis(300),
            Point::default()
        ));
        assert!(click(
            &mut clicks,
            at + Duration::from_millis(420),
            Point::default()
        ));
    }

    #[test]
    fn slow_or_distant_clicks_stay_single() {
        let at = Instant::now();
        for (delay, position) in [(501, Point::default()), (100, Point { x: 6.0, y: 0.0 })] {
            let mut clicks = DoubleClick::default();
            assert!(!click(&mut clicks, at, Point::default()));
            assert!(!click(
                &mut clicks,
                at + Duration::from_millis(delay),
                position
            ));
        }
    }

    #[test]
    fn dragging_away_and_back_does_not_start_a_double_click() {
        let at = Instant::now();
        let mut clicks = DoubleClick::default();
        assert!(!clicks.press(at, Point::default()));
        clicks.moved(Point { x: 30.0, y: 0.0 });
        clicks.moved(Point::default());
        clicks.release(at + Duration::from_millis(100));
        assert!(!click(
            &mut clicks,
            at + Duration::from_millis(200),
            Point::default()
        ));
    }

    #[test]
    fn long_press_is_not_a_click() {
        let at = Instant::now();
        let mut clicks = DoubleClick::default();
        assert!(!clicks.press(at, Point::default()));
        clicks.release(at + Duration::from_millis(600));
        assert!(!click(
            &mut clicks,
            at + Duration::from_millis(700),
            Point::default()
        ));
    }
}
