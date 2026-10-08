use crate::{decode::Target, limits::Limits, view::View};
use anyhow::{Context, Result, bail, ensure};
use image::RgbaImage;
use std::sync::{Arc, Mutex};
use wgpu::util::DeviceExt;
use winit::{event_loop::OwnedDisplayHandle, window::Window};

pub fn backend_order(windows: bool, forced: Option<&str>) -> Result<Vec<wgpu::Backends>> {
    if let Some(value) = forced {
        return Ok(vec![match value {
            "vulkan" => wgpu::Backends::VULKAN,
            "dx12" if windows => wgpu::Backends::DX12,
            "gles" | "gl" => wgpu::Backends::GL,
            _ => bail!("VYSYN_BACKEND must be vulkan, gles, or (on Windows) dx12"),
        }]);
    }
    Ok(if windows {
        vec![
            wgpu::Backends::DX12,
            wgpu::Backends::VULKAN,
            wgpu::Backends::GL,
        ]
    } else {
        vec![wgpu::Backends::VULKAN, wgpu::Backends::GL]
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Draw {
    Presented,
    Retry,
    Occluded,
}

pub struct Renderer {
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
    texture: Option<wgpu::Texture>,
    bind: Option<wgpu::BindGroup>,
    dimensions: [u32; 2],
    pub target: Target,
    pub gpu_bytes: u64,
    pub adapter_name: String,
    pub backend: wgpu::Backend,
    failure: Arc<Mutex<Option<String>>>,
    drawable: bool,
}

impl Renderer {
    pub async fn new(
        window: Arc<Window>,
        display: OwnedDisplayHandle,
        limits: &Limits,
    ) -> Result<Self> {
        let forced = std::env::var("VYSYN_BACKEND").ok();
        let order = backend_order(cfg!(target_os = "windows"), forced.as_deref())?;
        let mut errors = Vec::new();
        for backend in order {
            match Self::try_backend(window.clone(), display.clone(), limits, backend).await {
                Ok(r) => return Ok(r),
                Err(e) => errors.push(format!("{backend:?}: {e:#}")),
            }
        }
        bail!("no compatible GPU backend: {}", errors.join("; "))
    }

    async fn try_backend(
        window: Arc<Window>,
        display: OwnedDisplayHandle,
        limits: &Limits,
        backend: wgpu::Backends,
    ) -> Result<Self> {
        let mut descriptor = wgpu::InstanceDescriptor::new_with_display_handle(Box::new(display));
        descriptor.backends = backend;
        let instance = wgpu::Instance::new(descriptor);
        let surface = instance.create_surface(window.clone())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await?;
        let info = adapter.get_info();
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| caps.formats.first().copied())
            .context("surface has no usable formats")?;
        let adapter_limits = adapter.limits();
        let device_limits =
            wgpu::Limits::downlevel_defaults().using_resolution(adapter_limits.clone());
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Vysyn"),
                required_limits: device_limits,
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                ..Default::default()
            })
            .await?;
        let failure = Arc::new(Mutex::new(None));
        let error_slot = failure.clone();
        device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            *error_slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(error.to_string());
        }));
        let error_slot = failure.clone();
        device.set_device_lost_callback(move |reason, message| {
            if reason != wgpu::DeviceLostReason::Destroyed {
                *error_slot.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(format!("GPU device lost: {message}"));
            }
        });
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps
                .alpha_modes
                .iter()
                .copied()
                .find(|x| *x == wgpu::CompositeAlphaMode::Opaque)
                .or_else(|| caps.alpha_modes.first().copied())
                .context("surface has no alpha mode")?,
            view_formats: vec![],
            color_space: Default::default(),
        };
        ensure!(
            config.width <= device.limits().max_texture_dimension_2d
                && config.height <= device.limits().max_texture_dimension_2d,
            "window exceeds GPU limits"
        );
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        surface.configure(&device, &config);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("image"),
            source: wgpu::ShaderSource::Wgsl(include_str!("image.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("image"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        if let Some(e) = scope.pop().await {
            bail!("GPU initialization failed: {e}");
        }
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("view"),
            contents: bytemuck::cast_slice(&[0.0_f32; 8]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bilinear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let target = Target {
            max_dimension: device.limits().max_texture_dimension_2d,
            gpu_bytes: limits.gpu_bytes,
        };
        Ok(Self {
            instance,
            surface,
            device,
            queue,
            config,
            pipeline,
            uniform,
            sampler,
            texture: None,
            bind: None,
            dimensions: [0, 0],
            target,
            gpu_bytes: 0,
            adapter_name: info.name,
            backend: info.backend,
            failure,
            drawable: size.width > 0 && size.height > 0,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        self.drawable = width > 0 && height > 0;
        if !self.drawable {
            return Ok(());
        }
        ensure!(
            width <= self.target.max_dimension && height <= self.target.max_dimension,
            "window exceeds GPU texture dimensions"
        );
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        Ok(())
    }

    pub fn upload(&mut self, image: &RgbaImage) -> Result<()> {
        self.check_failure()?;
        let [w, h] = [image.width(), image.height()];
        ensure!(
            w > 0 && h > 0 && w <= self.target.max_dimension && h <= self.target.max_dimension,
            "image exceeds GPU texture dimensions"
        );
        let bytes = u64::from(w) * u64::from(h) * 4;
        ensure!(
            bytes <= self.target.gpu_bytes,
            "image exceeds GPU memory budget"
        );
        if self.dimensions != [w, h] {
            self.bind = None;
            if let Some(old) = self.texture.take() {
                old.destroy();
            }
            let allocation_scope = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("current image"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            if let Some(e) = pollster::block_on(allocation_scope.pop()) {
                bail!("cannot allocate GPU texture: {e}");
            }
            let view = texture.create_view(&Default::default());
            self.bind = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("image"),
                layout: &self.pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            }));
            self.texture = Some(texture);
            self.dimensions = [w, h];
            self.gpu_bytes = bytes;
        }
        let texture = self.texture.as_ref().context("texture allocation failed")?;
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            image.as_raw(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * w),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }

    fn check_failure(&self) -> Result<()> {
        if let Some(e) = self
            .failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            bail!("{e}");
        }
        Ok(())
    }

    pub fn draw(&mut self, view: &View, window: &Arc<Window>) -> Result<Draw> {
        self.check_failure()?;
        if !self.drawable {
            return Ok(Draw::Occluded);
        }
        let (output, suboptimal) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => (t, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => (t, true),
            wgpu::CurrentSurfaceTexture::Timeout => return Ok(Draw::Retry),
            wgpu::CurrentSurfaceTexture::Occluded => return Ok(Draw::Occluded),
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return Ok(Draw::Retry);
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.surface = self.instance.create_surface(window.clone())?;
                self.surface.configure(&self.device, &self.config);
                return Ok(Draw::Retry);
            }
            wgpu::CurrentSurfaceTexture::Validation => bail!("GPU surface validation error"),
        };
        let mut data = [0.0_f32; 8];
        data[..4].copy_from_slice(&view.shader_transform());
        data[4] = if self.config.format.is_srgb() {
            0.0
        } else {
            1.0
        };
        self.queue
            .write_buffer(&self.uniform, 0, bytemuck::cast_slice(&data));
        let target = output.texture.create_view(&Default::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("image"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("image"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            if let Some(bind) = &self.bind {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, bind, &[]);
                pass.draw(0..6, 0..1);
            }
        }
        self.queue.submit([encoder.finish()]);
        window.pre_present_notify();
        self.queue.present(output);
        if suboptimal {
            self.surface.configure(&self.device, &self.config);
        }
        Ok(Draw::Presented)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn platform_order_and_forced_backend() {
        assert_eq!(
            backend_order(false, None).unwrap(),
            [wgpu::Backends::VULKAN, wgpu::Backends::GL]
        );
        assert_eq!(
            backend_order(true, None).unwrap(),
            [
                wgpu::Backends::DX12,
                wgpu::Backends::VULKAN,
                wgpu::Backends::GL
            ]
        );
        assert_eq!(
            backend_order(false, Some("gles")).unwrap(),
            [wgpu::Backends::GL]
        );
        assert!(backend_order(false, Some("dx12")).is_err());
    }
}
