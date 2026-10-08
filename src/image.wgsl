struct Uniforms { transform: vec4<f32>, options: vec4<f32> };
@group(0) @binding(0) var<uniform> view: Uniforms;
@group(0) @binding(1) var image: texture_2d<f32>;
@group(0) @binding(2) var filtering: sampler;
struct Vertex { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex fn vs_main(@builtin(vertex_index) index: u32) -> Vertex {
    let corners = array<vec2<f32>, 6>(vec2(-1.0,-1.0), vec2(1.0,-1.0), vec2(-1.0,1.0),
                                     vec2(-1.0,1.0), vec2(1.0,-1.0), vec2(1.0,1.0));
    let p = corners[index];
    var out: Vertex;
    out.position = vec4(p * view.transform.xy + view.transform.zw, 0.0, 1.0);
    out.uv = vec2((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5);
    return out;
}

@fragment fn fs_main(in: Vertex) -> @location(0) vec4<f32> {
    // Texture RGB is premultiplied in linear light. The background is black.
    var rgb = textureSample(image, filtering, in.uv).rgb;
    if view.options.x > 0.5 {
        rgb = select(1.055 * pow(rgb, vec3(1.0 / 2.4)) - vec3(0.055),
                     12.92 * rgb, rgb <= vec3(0.0031308));
    }
    return vec4(rgb, 1.0);
}
