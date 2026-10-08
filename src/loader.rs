use crate::{
    cache::Cache,
    decode::{self, Decoded, Target},
    limits::{Budget, Limits},
    navigation::{self, FileKey},
};
use anyhow::{Context, Result, ensure};
use std::{
    collections::{HashSet, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Instant,
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
    Load(u64, PathBuf, bool),
    Scan(u64, PathBuf),
}

#[derive(Default)]
struct Queue {
    foreground: Option<(u64, PathBuf)>,
    scan: Option<(u64, PathBuf)>,
    preloads: VecDeque<(u64, PathBuf)>,
    active: HashSet<PathBuf>,
    stopped: bool,
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
            let proxy = proxy.clone();
            loader.threads.push(
                thread::Builder::new()
                    .name(format!("vysyn-decode-{index}"))
                    .spawn(move || worker(state, proxy))?,
            );
        }
        Ok(loader)
    }

    pub fn open(&self, path: PathBuf) -> u64 {
        let mut queue = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        let generation = self.shared.generation.fetch_add(1, Ordering::AcqRel) + 1;
        queue.foreground = Some((generation, path));
        queue.preloads.clear();
        queue.scan = None;
        self.shared.wake.notify_all();
        generation
    }

    pub fn scan(&self, generation: u64, directory: PathBuf) {
        let mut queue = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        queue.scan = Some((generation, directory));
        self.shared.wake.notify_all();
    }

    pub fn preload(&self, generation: u64, paths: impl IntoIterator<Item = PathBuf>) {
        if self.shared.limits.cache_bytes == 0 {
            return;
        }
        let mut queue = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        queue.preloads.clear();
        for path in paths.into_iter().take(2) {
            if !queue.active.contains(&path) && !queue.preloads.iter().any(|(_, p)| p == &path) {
                queue.preloads.push_back((generation, path));
            }
        }
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
        if let Some((generation, path)) = q.foreground.clone() {
            if !q.active.contains(&path) {
                let job = Job::Load(generation, path.clone(), true);
                q.active.insert(path.clone());
                q.foreground = None;
                return Some(job);
            }
        } else if let Some((g, p)) = q.scan.take() {
            return Some(Job::Scan(g, p));
        } else {
            while let Some((g, p)) = q.preloads.pop_front() {
                if q.active.insert(p.clone()) {
                    return Some(Job::Load(g, p, false));
                }
            }
        }
        q = shared.wake.wait(q).unwrap_or_else(|e| e.into_inner());
    }
}

fn worker(shared: Arc<Shared>, proxy: EventLoopProxy<Event>) {
    while let Some(job) = next_job(&shared) {
        let generation = match &job {
            Job::Load(g, _, _) | Job::Scan(g, _) => *g,
        };
        let cancelled = || shared.generation.load(Ordering::Acquire) != generation;
        match job {
            Job::Scan(generation, directory) => {
                let result = navigation::scan(&directory, &cancelled).map_err(|e| format!("{e:#}"));
                if !cancelled() {
                    let _ = proxy.send_event(Event::Directory {
                        generation,
                        directory,
                        result,
                    });
                }
            }
            Job::Load(generation, requested, foreground) => {
                let start = Instant::now();
                let mut path = requested.clone();
                let mut cached = false;
                let result = (|| -> Result<Arc<Decoded>> {
                    ensure!(!cancelled(), "request superseded");
                    if path.is_dir() {
                        let files = navigation::scan(&path, &cancelled)?;
                        ensure!(!files.is_empty(), "directory contains no supported images");
                        let _ = proxy.send_event(Event::Directory {
                            generation,
                            directory: path.clone(),
                            result: Ok(files.clone()),
                        });
                        let mut last_error = None;
                        // Corrupt files do not prevent opening the first valid image.
                        for candidate in files {
                            ensure!(!cancelled(), "request superseded");
                            path = candidate;
                            match load_one(&shared, &path, &cancelled) {
                                Ok((image, hit)) => {
                                    cached = hit;
                                    return Ok(image);
                                }
                                Err(e) => last_error = Some(e),
                            }
                        }
                        return Err(last_error.context("directory contains no decodable images")?);
                    }
                    let (image, hit) = load_one(&shared, &path, &cancelled)?;
                    cached = hit;
                    Ok(image)
                })()
                .map_err(|e| format!("{e:#}"));
                if foreground && !cancelled() {
                    let _ = proxy.send_event(Event::Loaded {
                        generation,
                        path,
                        result,
                        elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
                        cached,
                    });
                }
                let mut queue = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
                queue.active.remove(&requested);
                shared.wake.notify_all();
            }
        }
    }
}

fn load_one(
    shared: &Shared,
    path: &std::path::Path,
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
    let image = match attempt() {
        Ok(image) => image,
        Err(e) if e.to_string().contains("memory budget exhausted") => {
            shared
                .cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
            attempt()?
        }
        Err(e) => return Err(e),
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
