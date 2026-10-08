use anyhow::{Result, bail, ensure};
use std::sync::{Arc, Mutex};

const MIB: u64 = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Limits {
    pub max_pixels: u64,
    pub decoded_bytes: u64,
    pub ram_bytes: u64,
    pub cache_bytes: u64,
    pub gpu_bytes: u64,
    pub file_bytes: u64,
    pub workers: usize,
    pub max_frames: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_pixels: 32_000_000,
            decoded_bytes: 128 * MIB,
            ram_bytes: 512 * MIB,
            cache_bytes: 192 * MIB,
            gpu_bytes: 128 * MIB,
            file_bytes: 64 * MIB,
            workers: 2,
            max_frames: 512,
        }
    }
}

impl Limits {
    pub fn from_env() -> Result<Self> {
        let mut v = Self::default();
        for (key, target) in [
            ("VYSYN_MAX_PIXELS", &mut v.max_pixels),
            ("VYSYN_DECODED_MIB", &mut v.decoded_bytes),
            ("VYSYN_RAM_MIB", &mut v.ram_bytes),
            ("VYSYN_CACHE_MIB", &mut v.cache_bytes),
            ("VYSYN_GPU_MIB", &mut v.gpu_bytes),
            ("VYSYN_FILE_MIB", &mut v.file_bytes),
        ] {
            if let Some(value) = std::env::var_os(key) {
                let n: u64 = value
                    .to_str()
                    .unwrap_or("")
                    .parse()
                    .map_err(|_| anyhow::anyhow!("{key} must be a positive integer"))?;
                *target = if key.ends_with("MIB") {
                    n.checked_mul(MIB)
                        .ok_or_else(|| anyhow::anyhow!("{key} overflows"))?
                } else {
                    n
                };
            }
        }
        if let Some(value) = std::env::var_os("VYSYN_WORKERS") {
            v.workers = value.to_str().unwrap_or("").parse()?;
        }
        v.validate()?;
        Ok(v)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=4).contains(&self.workers),
            "VYSYN_WORKERS must be between 1 and 4"
        );
        ensure!(
            self.max_pixels > 0 && self.max_pixels <= u64::from(u32::MAX),
            "invalid pixel limit"
        );
        ensure!(
            self.decoded_bytes >= 4 && self.gpu_bytes >= 4 && self.file_bytes > 0,
            "limits must be positive"
        );
        ensure!(
            self.ram_bytes >= self.decoded_bytes && self.cache_bytes <= self.ram_bytes,
            "RAM must cover a decoded image and the cache limit"
        );
        ensure!(
            self.ram_bytes <= usize::MAX as u64,
            "RAM budget does not fit this platform"
        );
        Ok(())
    }

    pub fn rgba_bytes(&self, width: u32, height: u32) -> Result<u64> {
        ensure!(width > 0 && height > 0, "image has zero dimensions");
        let pixels = u64::from(width) * u64::from(height);
        ensure!(
            pixels <= self.max_pixels,
            "image exceeds the {} pixel limit",
            self.max_pixels
        );
        let bytes = pixels
            .checked_mul(4)
            .ok_or_else(|| anyhow::anyhow!("image dimensions overflow"))?;
        ensure!(
            bytes <= self.decoded_bytes,
            "image exceeds the decoded buffer limit"
        );
        Ok(bytes)
    }
}

#[derive(Clone, Debug)]
pub struct Budget(Arc<Mutex<(u64, u64)>>);

impl Budget {
    pub fn new(limit: u64) -> Self {
        Self(Arc::new(Mutex::new((0, limit))))
    }

    pub fn reserve(&self, bytes: u64) -> Result<Reservation> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if bytes > state.1.saturating_sub(state.0) {
            bail!(
                "image memory budget exhausted ({} MiB in use)",
                state.0 / MIB
            );
        }
        state.0 += bytes;
        Ok(Reservation {
            budget: self.clone(),
            bytes,
        })
    }

    pub fn used(&self) -> u64 {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).0
    }
}

#[derive(Debug)]
pub struct Reservation {
    budget: Budget,
    bytes: u64,
}

impl Reservation {
    pub fn shrink_to(&mut self, bytes: u64) {
        if bytes < self.bytes {
            let mut state = self.budget.0.lock().unwrap_or_else(|e| e.into_inner());
            state.0 -= self.bytes - bytes;
            self.bytes = bytes;
        }
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.0.lock().unwrap_or_else(|e| e.into_inner()).0 -= self.bytes;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allocation_accounting_survives_sharing() {
        let b = Budget::new(100);
        let r = Arc::new(b.reserve(75).unwrap());
        let other = r.clone();
        drop(r);
        assert!(b.reserve(26).is_err());
        drop(other);
        assert_eq!(b.used(), 0);
        let mut r = b.reserve(100).unwrap();
        r.shrink_to(4);
        assert_eq!(b.used(), 4);
    }
    #[test]
    fn bombs_and_bad_configuration_are_rejected() {
        let l = Limits::default();
        assert!(l.rgba_bytes(u32::MAX, u32::MAX).is_err());
        assert!(l.rgba_bytes(0, 1).is_err());
        assert!(l.rgba_bytes(100_000, 100_000).is_err());
        assert!(Limits { workers: 0, ..l }.validate().is_err());
    }
}
