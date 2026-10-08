# Project: Minimalist Image Viewer in Rust – Vysyn

**GitHub:** https://github.com/chl84/Vysyn

You are an experienced senior software engineer specializing in Rust, GPU rendering, memory safety, image processing, performance optimization, and cross-platform desktop applications.

Develop **Vysyn**, a complete, production-ready image viewer for **Linux and Windows**.

The application must be extremely fast, resource-efficient, secure, and minimalist.

The primary goal is to display images as quickly as possible, without unnecessary user interface elements or functionality.

Inspect the existing code in the GitHub repository before implementation. Build upon working code if it exists, and avoid unnecessary rewrites.

## 1. Technology Stack

Use the following technologies:

- **Rust:** Primary programming language, focusing on memory safety, stability, and performance.
- **winit:** Window management and user input.
- **wgpu:** Cross-platform GPU-accelerated rendering.
- **WGSL:** GPU shaders when necessary.
- **image:** Decoding JPEG, PNG, WebP, GIF, BMP, TIFF, ICO, and AVIF, with the required format features enabled.
- **libheif:** HEIC/HEIF decoding.
- **resvg:** SVG rendering.
- **rayon:** Optional parallel image processing, only if benchmarks demonstrate a measurable benefit.

### GPU Backends

**Linux:**
1. Vulkan (primary backend).
2. OpenGL/GLES (fallback).

**Windows:**
1. DirectX 12 or Vulkan, depending on compatibility and measured performance.
2. OpenGL/GLES (fallback).

Use wgpu to manage the available graphics backends.

Implement automatic backend selection with GPU capability detection and robust fallback when the preferred backend is unavailable.

Do not hardcode assumptions about users' GPUs or drivers.

Avoid initializing multiple backends unless necessary.

### Dependencies

- Prefer pure Rust libraries when they provide sufficient functionality and performance.
- Use `image` for AVIF decoding if the selected version and available codecs meet the requirements.
- Use `libheif` for HEIC/HEIF, with proper management of native dependencies.
- Avoid implementing duplicate decoders for the same format unless technically necessary.
- Select the latest stable, mutually compatible library versions during implementation.
- Avoid unnecessary dependencies.

Do not use Electron, Qt, GTK, egui, Iced, or any other GUI frameworks.

## 2. Design and User Interface

The application must be completely minimalist.

- No buttons.
- No menu bars.
- No toolbars.
- No side panels.
- No unnecessary visual elements.
- No welcome screen.
- No user interface animations.

**Only the image and a simple borderless window.**

The background must be completely black (`#000000`).

### Image Display

- Automatically fit images within the available window space.
- Preserve the correct aspect ratio.
- Do not automatically upscale small images beyond their original resolution.
- Center images within the window.
- Avoid unnecessary scaling and image distortion.
- Use bilinear filtering for normal scaling, and consider alternative filters only when they provide demonstrable benefits.

### HiDPI and Display Scaling

Implement proper handling of:

- HiDPI displays.
- Different screen resolutions.
- Windows Display Scaling.
- Wayland fractional scaling.
- Windows moved between monitors with different scaling factors.

Correctly distinguish between logical and physical pixels.

At 100% zoom, image pixels should map 1:1 to physical display pixels whenever possible.

Avoid unintended blurriness caused by double scaling.

## 3. Supported Image Formats

The application must support at least the following 10 image formats:

1. JPEG / JPG
2. PNG
3. WebP
4. GIF
5. BMP
6. TIFF
7. HEIC / HEIF
8. AVIF
9. SVG
10. ICO

### Requirements

- Support both static images and animated GIFs.
- Preserve correct animation timing.
- Handle transparency correctly.
- Detect image formats from file contents whenever possible, rather than relying solely on file extensions.
- Handle invalid and incomplete files without crashing.
- Avoid unnecessary conversions between image formats.

### Color Management

Implement correct handling of:

- ICC color profiles.
- sRGB.
- EXIF orientation and rotation.
- Alpha channels and transparency.
- Correct color blending against a black background.

Convert between color profiles when necessary for accurate display. Use an appropriate, maintained color management library if the selected image and rendering libraries do not provide the required functionality.

Account for the difference between linear color values and sRGB during rendering.

Avoid double gamma correction or duplicate color conversions.

Camera images must automatically display with the correct orientation.

When color profiles are missing or invalid, use a documented and consistent default behavior.

## 4. User Interaction

### Mouse

- Mouse wheel: Zoom in and out.
- Left mouse button + drag: Pan the image.
- Zoom must be centered around the mouse cursor.
- Drag and drop an image file into the window to open it.

### Keyboard

- `→`: Next image.
- `←`: Previous image.
- `+`: Zoom in.
- `-`: Zoom out.
- `0`: Fit image to window.
- `F11`: Toggle fullscreen.
- `Esc`: Close the application.

### Navigation

Navigation must work between supported image files in the same directory, sorted by filename.

- Navigation must be fast and predictable.
- Do not block the user interface when switching images.
- Maintain correct zoom and panning behavior.
- Reset zoom when switching images by default.
- Handle directories without valid image files.
- Handle files that are deleted or modified while the application is running.

## 5. Performance

Performance is one of the project's highest priorities.

Optimize specifically for:

1. Minimal startup time.
2. Fastest possible display of the first image.
3. Near-instant image switching.
4. Low RAM usage.
5. Low CPU utilization.
6. Efficient GPU usage.

### Startup Optimization

**Always prioritize displaying the first image.**

Implement startup in the following order:

1. Start the application and create the window.
2. Initialize the required rendering components.
3. Decode and display the first image as quickly as possible.
4. Only then begin preloading and other background operations.

Avoid scanning large directories, initializing unnecessary components, or preloading images before the first image becomes visible, unless required for correct functionality.

Do not perform expensive operations on the main thread.

### Image Decoding and Caching

Implement:

- Asynchronous image decoding.
- Preloading of the next and previous images.
- A bounded LRU cache.
- Reuse of decoded images when appropriate.
- Reuse of GPU textures when it improves performance.
- Efficient handling of large images.
- Cancellation or safe discarding of obsolete decoding tasks.
- Minimization of unnecessary memory copies.

Do not introduce `rayon` unless benchmarks demonstrate a performance advantage over a simpler solution.

### Rendering

- Render only when the image or view changes.
- Render animated GIFs according to their frame timing.
- Avoid continuous rendering when displaying a static image.
- Avoid unnecessary GPU transfers.
- Minimize GPU resource usage and draw calls.

### Large Images

Handle large images without unnecessary resource consumption.

- Check the GPU's maximum supported texture dimensions.
- Check available memory resources.
- Use downscaling or another bounded representation when the original image cannot be loaded safely.
- Avoid oversized intermediate image buffers.
- Do not implement complex image tiling systems unless testing demonstrates a clear need.

### Performance Measurements

Establish measurable benchmarks for:

- Startup time.
- Time to first visible image.
- Decoding time per image format.
- Image navigation latency.
- RAM usage.
- GPU memory usage.
- CPU utilization while idle.

Benchmark Release builds, not Debug builds.

Document the results and test environment.

Prioritize measurable improvements over theoretical micro-optimizations.

## 6. Security and Stability

- Use safe Rust code wherever possible.
- Avoid `unsafe` in application code unless strictly necessary.
- Validate image dimensions before allocating resources.
- Limit the number of pixels that can be decoded.
- Protect against decompression bombs.
- Handle invalid, corrupted, and potentially malicious image files.
- Avoid panics during common user errors.
- Avoid blocking the main thread.
- Limit parallel processing.
- Control memory usage and resource consumption.
- Isolate native libraries behind clearly defined interfaces.
- Document security limitations associated with native decoders.
- Do not allow SVG files to fetch external network resources.

### Memory Limits

Implement configurable, reasonable default limits for:

1. Maximum image pixel count.
2. Maximum decoded image buffer size.
3. Maximum RAM usage for image buffers and caches.
4. Maximum GPU cache size.
5. Maximum number of concurrent decoding tasks.

Use controlled resource cleanup when limits are reached.

The application must provide a controlled error message or use a safely downscaled representation when an image exceeds available resources.

Extremely large or invalid images must not cause uncontrolled memory consumption.

Remember that Rust's memory safety guarantees do not automatically prevent vulnerabilities in native libraries.

## 7. Cross-Platform Support

### Linux

- Support Wayland.
- Support X11.
- Prioritize Vulkan.
- Use OpenGL/GLES as a fallback when available.
- Handle display scaling correctly.
- Support common desktop environments and compositors.

### Windows

- Support Windows 10 and 11.
- Use DirectX 12 or Vulkan.
- Use OpenGL/GLES as a fallback when available.
- Handle Windows Display Scaling.
- Run without an unnecessary console window.

### General Requirements

Use the same Rust codebase for both platforms.

Minimize platform-specific code.

Document installation, compilation, and distribution of required native libraries.

Ensure that the application can be distributed without requiring end users to manually configure development tools or codec libraries.

## 8. Architecture

Keep the project structure simple and modular.

Separate responsibilities for:

- Application lifecycle and event handling.
- Image decoding and format detection.
- Color management and image orientation.
- GPU rendering.
- File navigation.
- Caching and preloading.
- Zooming and panning.
- Resource limits.

Use clear module boundaries, but do not create more modules than necessary.

Prefer a simple, maintainable architecture over unnecessary abstractions.

Avoid overengineering and complex design patterns.

## 9. Startup and File Opening

The application must support:

- Opening an image file from the command line.
- Opening image files from the operating system's file manager.
- Opening a directory and displaying its first supported image.
- Navigating between images in the same directory.
- Opening images through drag and drop.

Example:

`vysyn /path/to/image.jpg`

Display the image as quickly as possible.

Document file manager integration for both Linux and Windows.

## 10. Testing and Quality Assurance

Implement relevant tests for:

- Image format detection.
- File navigation.
- Zoom calculations.
- Panning.
- Cache management.
- Memory limits.
- Invalid image files.
- Large image dimensions.
- EXIF rotation.
- ICC color management.
- Transparency.
- HiDPI scaling.
- GPU backend selection and fallback.

Use:

- `cargo fmt`
- `cargo clippy`
- `cargo test`

### Cross-Platform Testing

Verify that the application compiles and functions on both Linux and Windows.

Create GitHub Actions CI workflows for automated builds and tests on both platforms.

Where GPUs or graphical sessions are unavailable in CI, separate tests so that non-graphical tests can still run.

Do not report a platform as functionally tested if only compilation has been verified.

Document any tests that require manual verification.

## 11. Deliverables

Provide:

1. Complete Rust project.
2. All required source files.
3. `Cargo.toml` and `Cargo.lock`.
4. Build instructions for Linux and Windows.
5. README with a concise user guide.
6. Optimized Release build.
7. GitHub Actions workflows for automated building and testing.
8. Documentation of known limitations.
9. Results from relevant performance benchmarks.

Use appropriate Release build settings, but document their impact on binary size, compilation time, and compatibility.

Do not enable aggressive optimizations without evaluating their actual performance benefits.

## 12. Priority Order

When making technical trade-offs, follow this priority order:

1. **Security and stability.**
2. **Fast startup and image display.**
3. **Simplicity and minimalism.**
4. **Low resource consumption.**
5. **Accurate image quality and color reproduction.**
6. **Cross-platform compatibility.**
7. **Code maintainability.**

## 13. Development Instructions

Follow this development sequence:

1. Inspect the existing code in the GitHub repository.
2. Create a brief architecture plan.
3. Select and verify compatible library versions.
4. Implement window management and GPU rendering.
5. Implement image format support and accurate image reproduction.
6. Implement navigation, zooming, and panning.
7. Implement security measures and resource limits.
8. Optimize startup and image switching.
9. Implement automated tests and CI.
10. Run available build checks and performance benchmarks.
11. Update the README and documentation.

Perform development in logical, testable steps.

Do not introduce unnecessary dependencies or advanced optimizations without a documented need.

Preserve working code and avoid unnecessary rewrites.

### Final Requirements

**Vysyn must not become an image editor, image management application, or complex desktop application.**

It must be an ultra-fast, minimalist image viewer supporting at least 10 image formats.

No buttons, menus, side panels, or visible controls.

Use Rust, winit, and wgpu as the foundation.

Prioritize Vulkan on Linux and DirectX 12/Vulkan on Windows, with automatic fallback.

Ensure accurate color reproduction, proper display scaling, and safe resource usage.

**The goal is maximum real-world performance with minimum complexity.**

Implement the simplest robust solution that meets these requirements.

Do not stop at pseudocode, an architecture plan, or an incomplete example. Deliver working code, run the available tests, and accurately document what has been verified.