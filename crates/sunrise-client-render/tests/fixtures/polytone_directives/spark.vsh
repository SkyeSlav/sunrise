#version 330

// A Polytone GPU particle's vertex stage that takes its shape from directives: a function-like
// macro, a condition over integers and `defined`, a macro defined in one branch, and `__LINE__`.
#define SCALE(x) ((x) * 0.5)
#define HALF 2
#if HALF == 2 && !defined(MISSING) && __LINE__ > 3
#define TINT vec4(1.0, 0.5, 0.25, 1.0)
#else
#define TINT vec4(0.0)
#endif
#ifdef SCALE
#undef HALF
#define HALF 4
#endif

layout(location = 0) out vec4 tint;
layout(location = 1) out float line;

void main() {
    gl_Position = vec4(SCALE(Position.xy), Position.z, 1.0);
    tint = Color * TINT;
    line = float(__LINE__ + HALF);
}
