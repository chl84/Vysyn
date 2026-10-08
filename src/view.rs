#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// All dimensions and input coordinates are physical display pixels.
#[derive(Debug)]
pub struct View {
    pub viewport: [u32; 2],
    pub image: [u32; 2],
    pub scale: f64,
    pub pan: Point,
    fitted: bool,
}

impl Default for View {
    fn default() -> Self {
        Self {
            viewport: [1, 1],
            image: [1, 1],
            scale: 1.0,
            pan: Point::default(),
            fitted: true,
        }
    }
}

impl View {
    pub fn set_image(&mut self, image: [u32; 2]) {
        self.image = image;
        self.fit();
    }
    pub fn resize(&mut self, viewport: [u32; 2]) {
        self.viewport = viewport;
        if self.fitted {
            self.fit();
        }
    }
    pub fn fit(&mut self) {
        self.scale = (f64::from(self.viewport[0]) / f64::from(self.image[0].max(1)))
            .min(f64::from(self.viewport[1]) / f64::from(self.image[1].max(1)))
            .min(1.0);
        self.pan = Point::default();
        self.fitted = true;
    }
    pub fn center(&self) -> Point {
        Point {
            x: f64::from(self.viewport[0]) / 2.0,
            y: f64::from(self.viewport[1]) / 2.0,
        }
    }
    pub fn zoom(&mut self, factor: f64, cursor: Point) {
        if !factor.is_finite() || factor <= 0.0 || !cursor.x.is_finite() || !cursor.y.is_finite() {
            return;
        }
        let next = (self.scale * factor).clamp(1.0 / 4096.0, 64.0);
        let ratio = next / self.scale.max(f64::MIN_POSITIVE);
        let c = self.center();
        self.pan.x = cursor.x - c.x - (cursor.x - c.x - self.pan.x) * ratio;
        self.pan.y = cursor.y - c.y - (cursor.y - c.y - self.pan.y) * ratio;
        self.scale = next;
        self.fitted = false;
    }
    pub fn drag(&mut self, delta: Point) {
        if delta.x.is_finite() && delta.y.is_finite() {
            self.pan.x = (self.pan.x + delta.x).clamp(-1e9, 1e9);
            self.pan.y = (self.pan.y + delta.y).clamp(-1e9, 1e9);
            self.fitted = false;
        }
    }
    pub fn shader_transform(&self) -> [f32; 4] {
        let w = f64::from(self.viewport[0].max(1));
        let h = f64::from(self.viewport[1].max(1));
        let mut pan = self.pan;
        // Align texel centers at 100% even with odd image/window dimensions.
        if (self.scale - 1.0).abs() < 1e-9 {
            let left = (w - f64::from(self.image[0])) / 2.0 + pan.x;
            let top = (h - f64::from(self.image[1])) / 2.0 + pan.y;
            pan.x += left.round() - left;
            pan.y += top.round() - top;
        }
        [
            (f64::from(self.image[0]) * self.scale / w) as f32,
            (f64::from(self.image[1]) * self.scale / h) as f32,
            (2.0 * pan.x / w) as f32,
            (-2.0 * pan.y / h) as f32,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fit_preserves_aspect_and_never_upscales() {
        let mut v = View::default();
        v.resize([1000, 500]);
        v.set_image([2000, 2000]);
        assert_eq!(v.scale, 0.25);
        v.set_image([100, 50]);
        assert_eq!(v.scale, 1.0);
        assert_eq!(v.shader_transform(), [0.1, 0.1, 0.0, -0.0]);
    }
    #[test]
    fn zoom_keeps_cursor_image_point_fixed() {
        let mut v = View::default();
        v.resize([800, 600]);
        v.set_image([800, 600]);
        let p = Point { x: 600.0, y: 200.0 };
        v.zoom(2.0, p);
        assert_eq!(
            v.pan,
            Point {
                x: -200.0,
                y: 100.0
            }
        );
        v.zoom(0.5, p);
        assert_eq!(v.pan, Point::default());
        v.drag(Point { x: 10.0, y: -5.0 });
        assert_eq!(v.pan, Point { x: 10.0, y: -5.0 });
    }
    #[test]
    fn monitor_scale_changes_do_not_double_scale_image() {
        let mut v = View::default();
        v.resize([800, 600]);
        v.set_image([300, 200]);
        v.resize([1600, 1200]);
        assert_eq!(v.scale, 1.0);
        v.zoom(2.0, v.center());
        v.resize([1200, 900]);
        assert_eq!(v.scale, 2.0);
        v.zoom(f64::NAN, v.center());
        assert_eq!(v.scale, 2.0);
    }
    #[test]
    fn odd_sized_images_align_to_physical_pixels_at_one_to_one() {
        let mut view = View::default();
        view.resize([1000, 700]);
        view.set_image([301, 201]);
        let transform = view.shader_transform();
        assert_eq!(transform[2], 0.001);
        assert_eq!(transform[3], -1.0 / 700.0);
    }
}
