use crate::{
    cache::Cache,
    decode::{self, Decoded, Target},
    limits::{Budget, Limits, MemoryPressure},
    navigation::{self, FileKey},
};
use anyhow::{Context, Result, ensure};
use std::{
    cell::Cell,
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use winit::event_loop::EventLoopProxy;

#[derive(Debug)]
pub enum Event {
    Loaded {
        generation: u64,
        path: PathBuf,
        result: Result<Arc<Decoded>, String>,
        elapsed_ms: f64,
        cached: bool,
    },
    Directory {
        generation: u64,
        directory: PathBuf,
        result: Result<Vec<PathBuf>, String>,
    },
}

enum Job {
    Load(PathBuf),
    Scan(u64, PathBuf),
}

#[derive(Clone)]
struct Request {
    generation: u64,
    path: PathBuf,
    started: Instant,
}

#[derive(Default)]
struct Queue {
    // Remains present while decoding, so an active preload can satisfy it.
    foreground: Option<Request>,
    scan: Option<(u64, PathBuf)>,
    preloads: VecDeque<PathBuf>,
    wanted: HashSet<PathBuf>,
    active: HashSet<PathBuf>,
    // Retry only after reservations have been released, not on a timer alone.
    blocked: HashMap<PathBuf, u64>,
    stopped: bool,
}

impl Queue {
    fn ready(&self, path: &Path, used: u64) -> bool {
        !self.active.contains(path)
            && self.blocked.get(path).is_none_or(|previous| {
                used < *previous
                    || (self.active.is_empty()
                        && self.foreground.as_ref().is_some_and(|r| r.path == path))
            })
    }
}

struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
    generation: AtomicU64,
    cache: Mutex<Cache<FileKey, Decoded>>,
    budget: Budget,
    limits: Limits,
    target: Target,
}

pub struct Loader {
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
}

impl Loader {
    pub fn new(limits: Limits, target: Target, proxy: EventLoopProxy<Event>) -> Result<Self> {
        Self::start(
            limits,
            target,
            Arc::new(move |event| {
                let _ = proxy.send_event(event);
            }),
        )
    }

    fn start(
        limits: Limits,
        target: Target,
        send: Arc<dyn Fn(Event) + Send + Sync>,
    ) -> Result<Self> {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
            generation: AtomicU64::new(0),
            cache: Mutex::new(Cache::new(limits.cache_bytes)),
            budget: Budget::new(limits.ram_bytes),
            target,
            limits,
        });
        let mut loader = Self {
            shared,
            threads: Vec::new(),
        };
        for index in 0..loader.shared.limits.workers {
            let state = loader.shared.clone();
            let send = send.clone();
            loader.threads.push(
                thread::Builder::new()
                    .name(format!("vysyn-decode-{index}"))
                    .spawn(move || worker(state, move |event| send(event), load_one))?,
            );
        }
        Ok(loader)
    }

    pub fn open(&self, path: PathBuf) -> u64 {
        let mut queue = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        let generation = self.shared.generation.fetch_add(1, Ordering::AcqRel) + 1;
        queue.blocked.remove(&path);
        queue.wanted.retain(|p| p == &path);
        queue.blocked.retain(|p, _| p == &path);
        queue.foreground = Some(Request {
            generation,
            path,
            started: Instant::now(),
        });
        queue.preloads.clear();
        queue.scan = None;
        self.shared.wake.notify_all();
        generation
    }

    pub fn scan(&self, generation: u64, directory: PathBuf) {
        let mut queue = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        if self.shared.generation.load(Ordering::Acquire) != generation || queue.stopped {
            return;
        }
        queue.scan = Some((generation, directory));
        self.shared.wake.notify_all();
    }

    pub fn preload(&self, generation: u64, paths: impl IntoIterator<Item = PathBuf>) {
        if self.shared.limits.cache_bytes == 0 {
            return;
        }
        let mut queue = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        if self.shared.generation.load(Ordering::Acquire) != generation || queue.stopped {
            return;
        }
        queue.preloads.clear();
        queue.wanted.clear();
        for path in paths.into_iter().take(2) {
            if queue.wanted.insert(path.clone()) && !queue.active.contains(&path) {
                queue.preloads.push_back(path);
            }
        }
        let wanted = queue.wanted.clone();
        queue.blocked.retain(|p, _| wanted.contains(p));
        self.shared.wake.notify_all();
    }

    pub fn memory_used(&self) -> u64 {
        self.shared.budget.used()
    }

    pub fn stop(&mut self) {
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        let mut queue = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        queue.stopped = true;
        queue.foreground = None;
        queue.preloads.clear();
        queue.wanted.clear();
        queue.blocked.clear();
        queue.scan = None;
        drop(queue);
        self.shared.wake.notify_all();
        // Detach: a native decoder cannot be safely interrupted. Process exit
        // reclaims it without making Esc wait for an uncooperative file.
        self.threads.clear();
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        self.stop();
    }
}

fn next_job(shared: &Shared) -> Option<Job> {
    let mut q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if q.stopped {
            return None;
        }
        let used = shared.budget.used();
        if let Some(request) = q.foreground.clone() {
            let path = request.path;
            if q.ready(&path, used) {
                let job = Job::Load(path.clone());
                q.active.insert(path.clone());
                q.blocked.remove(&path);
                return Some(job);
            }
        } else if let Some((g, p)) = q.scan.take() {
            return Some(Job::Scan(g, p));
        } else {
            if let Some(index) = q.preloads.iter().position(|p| q.ready(p, used)) {
                let path = q.preloads.remove(index).expect("existing preload");
                q.active.insert(path.clone());
                q.blocked.remove(&path);
                return Some(Job::Load(path));
            }
        }
        // A foreground image can release its last Arc on the UI thread, which
        // does not hold this queue. Poll only reservation counts while deferred.
        q = if q.blocked.is_empty() {
            shared.wake.wait(q).unwrap_or_else(|e| e.into_inner())
        } else {
            shared
                .wake
                .wait_timeout(q, Duration::from_millis(25))
                .unwrap_or_else(|e| e.into_inner())
                .0
        };
    }
}

fn worker(
    shared: Arc<Shared>,
    send: impl Fn(Event),
    load: impl Fn(&Shared, &Path, &Path, &dyn Fn() -> bool) -> Result<(Arc<Decoded>, bool)>,
) {
    while let Some(job) = next_job(&shared) {
        match job {
            Job::Scan(generation, directory) => {
                let cancelled = || shared.generation.load(Ordering::Acquire) != generation;
                let result = navigation::scan(&directory, &cancelled).map_err(|e| format!("{e:#}"));
                if !cancelled() {
                    send(Event::Directory {
                        generation,
                        directory,
                        result,
                    });
                }
            }
            Job::Load(requested) => {
                let was_cancelled = Cell::new(false);
                let cancelled = || {
                    let q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
                    let stopped = q.stopped
                        || !(q.foreground.as_ref().is_some_and(|r| r.path == requested)
                            || q.wanted.contains(&requested));
                    if stopped {
                        was_cancelled.set(true);
                    }
                    stopped
                };
                let mut path = requested.clone();
                let mut cached = false;
                let result = (|| -> Result<Arc<Decoded>> {
                    ensure!(!cancelled(), "request superseded");
                    if path.is_dir() {
                        let files = navigation::scan(&path, &cancelled)?;
                        ensure!(!files.is_empty(), "directory contains no supported images");
                        let q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
                        if let Some(request) = q.foreground.as_ref().filter(|r| r.path == requested)
                        {
                            send(Event::Directory {
                                generation: request.generation,
                                directory: path.clone(),
                                result: Ok(files.clone()),
                            });
                        }
                        drop(q);
                        let mut last_error = None;
                        // Corrupt files do not prevent opening the first valid image.
                        for candidate in files {
                            ensure!(!cancelled(), "request superseded");
                            path = candidate;
                            match load(&shared, &path, &requested, &cancelled) {
                                Ok((image, hit)) => {
                                    cached = hit;
                                    return Ok(image);
                                }
                                Err(e) if e.is::<Deferred>() => return Err(e),
                                Err(e) => last_error = Some(e),
                            }
                        }
                        return Err(last_error.context("directory contains no decodable images")?);
                    }
                    let (image, hit) = load(&shared, &path, &requested, &cancelled)?;
                    cached = hit;
                    Ok(image)
                })();
                let mut queue = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
                queue.active.remove(&requested);
                let foreground = queue
                    .foreground
                    .as_ref()
                    .is_some_and(|r| r.path == requested);
                if !queue.stopped && (foreground || queue.wanted.contains(&requested)) {
                    if let Some(deferred) = result
                        .as_ref()
                        .err()
                        .and_then(|e| e.downcast_ref::<Deferred>())
                    {
                        queue.blocked.insert(requested.clone(), deferred.used);
                        if !foreground && !queue.preloads.contains(&requested) {
                            queue.preloads.push_back(requested);
                        }
                    } else if foreground && !(result.is_err() && was_cancelled.get()) {
                        let request = queue.foreground.take().expect("foreground request");
                        send(Event::Loaded {
                            generation: request.generation,
                            path,
                            result: result.map_err(|e| format!("{e:#}")),
                            elapsed_ms: request.started.elapsed().as_secs_f64() * 1000.0,
                            cached,
                        });
                    }
                }
                shared.wake.notify_all();
            }
        }
    }
}

fn load_one(
    shared: &Shared,
    path: &Path,
    requested: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<(Arc<Decoded>, bool)> {
    let key = FileKey::read(path)?;
    if let Some(image) = shared
        .cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
    {
        return Ok((image, true));
    }
    let attempt = || {
        decode::decode(
            path,
            &shared.limits,
            &shared.budget,
            shared.target,
            cancelled,
        )
    };
    let image = loop {
        ensure!(!cancelled(), "request superseded");
        match attempt() {
            Ok(image) => break image,
            Err(e) if e.is::<MemoryPressure>() => {
                let pressure = e.downcast_ref::<MemoryPressure>().expect("memory pressure");
                if pressure.requested > pressure.limit {
                    return Err(e);
                }
                let q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
                let foreground = q.foreground.as_ref().is_some_and(|r| r.path == requested);
                let other_active = q.active.iter().any(|p| p != requested);
                drop(q);
                if foreground && !other_active {
                    // Evict only as much unpinned LRU data as necessary. A
                    // background decode never empties the foreground cache.
                    if shared
                        .cache
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .evict_unshared()
                    {
                        continue;
                    }
                    return Err(e);
                }
                let used = shared.budget.used();
                if used == 0 {
                    return Err(e);
                }
                return Err(e.context(Deferred { used }));
            }
            Err(e) => return Err(e),
        }
    };
    ensure!(
        FileKey::read(path)? == key,
        "file changed during decode; try again"
    );
    ensure!(!cancelled(), "request superseded");
    let image = Arc::new(image);
    shared
        .cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, image.clone(), image.bytes);
    Ok((image, false))
}

#[derive(Debug)]
struct Deferred {
    used: u64,
}

impl std::fmt::Display for Deferred {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("decode deferred until memory is released")
    }
}

impl std::error::Error for Deferred {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicUsize, mpsc};

    fn test_loader(limits: Limits) -> Loader {
        let target = Target {
            max_dimension: 8192,
            gpu_bytes: limits.gpu_bytes,
        };
        Loader {
            shared: Arc::new(Shared {
                queue: Mutex::new(Queue::default()),
                wake: Condvar::new(),
                generation: AtomicU64::new(0),
                cache: Mutex::new(Cache::new(limits.cache_bytes)),
                budget: Budget::new(limits.ram_bytes),
                limits,
                target,
            }),
            threads: Vec::new(),
        }
    }

    fn png(directory: &Path, name: &str) -> PathBuf {
        let path = directory.join(name);
        image::RgbaImage::from_pixel(128, 128, image::Rgba([80, 120, 160, 255]))
            .save(&path)
            .unwrap();
        path
    }

    #[test]
    fn active_preload_satisfies_latest_request_without_duplicate_decoding() {
        let dir = tempfile::tempdir().unwrap();
        let path = png(dir.path(), "a.png");
        let mut loader = test_loader(Limits::default());
        loader.preload(0, [path.clone()]);
        let (entered, observed) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let (send, events) = mpsc::channel();
        let count = Arc::new(AtomicUsize::new(0));
        let state = loader.shared.clone();
        let calls = count.clone();
        let sink = send.clone();
        let first = thread::spawn(move || {
            worker(
                state,
                move |event| {
                    sink.send(event).unwrap();
                },
                move |shared, path, requested, cancelled| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    entered.send(()).unwrap();
                    gate.recv_timeout(Duration::from_secs(5)).unwrap();
                    load_one(shared, path, requested, cancelled)
                },
            )
        });
        observed.recv_timeout(Duration::from_secs(5)).unwrap();
        loader.open(path.clone());
        let latest = loader.open(path.clone());
        let state = loader.shared.clone();
        let calls = count.clone();
        let second = thread::spawn(move || {
            worker(
                state,
                move |event| {
                    send.send(event).unwrap();
                },
                move |shared, path, requested, cancelled| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    load_one(shared, path, requested, cancelled)
                },
            )
        });
        release.send(()).unwrap();
        match events.recv_timeout(Duration::from_secs(5)).unwrap() {
            Event::Loaded {
                generation,
                path: loaded,
                result,
                cached,
                ..
            } => {
                assert_eq!(generation, latest);
                assert_eq!(loaded, path);
                assert_eq!(result.unwrap().original, [128, 128]);
                assert!(!cached);
            }
            _ => panic!("expected promoted image"),
        }
        loader.stop();
        first.join().unwrap();
        second.join().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(events.try_recv().is_err());
    }

    #[test]
    fn obsolete_decode_never_delivers_over_a_new_selection() {
        let dir = tempfile::tempdir().unwrap();
        let old = png(dir.path(), "old.png");
        let new = png(dir.path(), "new.png");
        let mut loader = test_loader(Limits::default());
        loader.open(old.clone());
        let (entered, observed) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let (send, events) = mpsc::channel();
        let state = loader.shared.clone();
        let delayed = old.clone();
        let worker = thread::spawn(move || {
            worker(
                state,
                move |event| {
                    send.send(event).unwrap();
                },
                move |shared, path, requested, cancelled| {
                    if path == delayed {
                        entered.send(()).unwrap();
                        gate.recv_timeout(Duration::from_secs(5)).unwrap();
                    }
                    load_one(shared, path, requested, cancelled)
                },
            )
        });
        observed.recv_timeout(Duration::from_secs(5)).unwrap();
        let generation = loader.open(new.clone());
        release.send(()).unwrap();
        match events.recv_timeout(Duration::from_secs(5)).unwrap() {
            Event::Loaded {
                generation: actual,
                path,
                result,
                ..
            } => {
                assert_eq!(actual, generation);
                assert_eq!(path, new);
                assert!(result.is_ok());
            }
            _ => panic!("expected latest image"),
        }
        loader.stop();
        worker.join().unwrap();
        assert!(events.try_recv().is_err());
        assert!(
            loader
                .shared
                .cache
                .lock()
                .unwrap()
                .get(&FileKey::read(&old).unwrap())
                .is_none()
        );
    }

    #[test]
    fn background_memory_pressure_preserves_cache_and_retries_after_release() {
        let dir = tempfile::tempdir().unwrap();
        let first = png(dir.path(), "a.png");
        let next = png(dir.path(), "b.png");
        let loader = test_loader(Limits {
            ram_bytes: 12 * 1024 * 1024,
            decoded_bytes: 1024 * 1024,
            cache_bytes: 1024 * 1024,
            ..Limits::default()
        });
        load_one(&loader.shared, &first, &first, &|| false).unwrap();
        let memory = loader.shared.budget.reserve(4 * 1024 * 1024).unwrap();
        let used = loader.memory_used();
        let error = load_one(&loader.shared, &next, &next, &|| false).unwrap_err();
        assert!(error.is::<Deferred>());
        assert!(error.is::<MemoryPressure>());
        assert_eq!(loader.memory_used(), used);
        assert!(
            loader
                .shared
                .cache
                .lock()
                .unwrap()
                .get(&FileKey::read(&first).unwrap())
                .is_some()
        );
        let mut queue = loader.shared.queue.lock().unwrap();
        queue
            .blocked
            .insert(next.clone(), error.downcast_ref::<Deferred>().unwrap().used);
        assert!(!queue.ready(&next, loader.memory_used()));
        drop(memory);
        assert!(queue.ready(&next, loader.memory_used()));
        drop(queue);
        load_one(&loader.shared, &next, &next, &|| false).unwrap();
        assert!(
            loader
                .shared
                .cache
                .lock()
                .unwrap()
                .get(&FileKey::read(&first).unwrap())
                .is_some()
        );
    }

    #[test]
    fn foreground_evicts_only_needed_lru_buffers() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ["a.png", "b.png", "c.png", "d.png", "e.png"].map(|name| png(dir.path(), name));
        let loader = test_loader(Limits {
            ram_bytes: 8 * 1024 * 1024 + 640 * 1024,
            decoded_bytes: 1024 * 1024,
            cache_bytes: 1024 * 1024,
            ..Limits::default()
        });
        let mut current = None;
        for path in &paths[..4] {
            current = Some(load_one(&loader.shared, path, path, &|| false).unwrap().0);
        }
        loader.open(paths[4].clone());
        load_one(&loader.shared, &paths[4], &paths[4], &|| false).unwrap();
        let mut cache = loader.shared.cache.lock().unwrap();
        assert!(cache.get(&FileKey::read(&paths[0]).unwrap()).is_none());
        for path in &paths[1..] {
            assert!(cache.get(&FileKey::read(path).unwrap()).is_some());
        }
        assert_eq!(cache.used(), 4 * 128 * 128 * 4);
        assert_eq!(current.unwrap().original, [128, 128]);
    }

    #[test]
    fn foreground_retries_when_other_work_finishes_without_lower_memory_use() {
        let path = PathBuf::from("wanted.gif");
        let mut queue = Queue {
            foreground: Some(Request {
                generation: 1,
                path: path.clone(),
                started: Instant::now(),
            }),
            ..Queue::default()
        };
        queue.active.insert(PathBuf::from("other.gif"));
        queue.blocked.insert(path.clone(), 100);
        assert!(!queue.ready(&path, 120));
        queue.active.clear();
        assert!(queue.ready(&path, 120));
    }
}
