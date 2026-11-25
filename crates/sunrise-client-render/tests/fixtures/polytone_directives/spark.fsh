#version 330

#define MIX(a, b, t) ((a) * (1.0 - (t)) + (b) * (t))

layout(location = 0) in vec4 tint;
layout(location = 1) in float line;
out vec4 fragColor;

void main() {
    fragColor = vec4(MIX(tint.rgb, vec3(line * 0.01), 0.25), tint.a);
}
