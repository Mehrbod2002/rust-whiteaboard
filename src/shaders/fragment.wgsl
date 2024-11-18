@fragment
fn fs_main() -> @location(0) vec4<f32> {
    // Return a solid red color for all fragments
    return vec4<f32>(1.0, 0.0, 0.0, 1.0); // RGBA: Red, no transparency
}
