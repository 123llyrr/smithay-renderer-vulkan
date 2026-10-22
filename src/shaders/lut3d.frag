#version 450

layout(push_constant) uniform LutPushConstants {
    mat4 matrix;
    vec4 uv_bounds;
    float lut_weight;
    float lut_size;
} pc;

layout(binding = 0) uniform sampler2D tex;
layout(binding = 1) uniform sampler3D lut3d;

layout(location = 0) in vec2 uv;
layout(location = 0) out vec4 out_color;

void main() {
    vec4 color = texture(tex, uv);
    float alpha = clamp(color.a, 0.0, 1.0);
    if (alpha <= 0.0) {
        out_color = vec4(0.0);
        return;
    }
    vec3 rgb = color.rgb / alpha;
    float scale = (pc.lut_size - 1.0) / pc.lut_size;
    float offset = 0.5 / pc.lut_size;
    vec3 lut_coord = clamp(rgb, 0.0, 1.0) * scale + vec3(offset);
    vec3 graded = texture(lut3d, lut_coord).rgb;
    vec3 final_rgb = mix(rgb, graded, clamp(pc.lut_weight, 0.0, 1.0));
    out_color = vec4(final_rgb * alpha, alpha);
}
