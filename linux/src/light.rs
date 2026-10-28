use snowhush::geometry::smoothstep;

/// The scene turns warmer at dawn and dusk, cooler and dimmer at night.
/// Clock-based rather than solar: close enough for mood, and needs no location.
pub fn daylight() -> [f32; 3] {
    let hour = local_hour();
    let day = smoothstep(6.0, 8.5, hour) * (1.0 - smoothstep(17.5, 20.5, hour));
    let golden =
        ((-((hour - 7.6) / 1.1).powi(2)).exp() + (-((hour - 18.6) / 1.2).powi(2)).exp()).min(1.0);
    let night = [0.86, 0.89, 0.97];
    let warm = [1.0, 0.965, 0.915];
    std::array::from_fn(|i| {
        let base = night[i] + (1.0 - night[i]) * day;
        base + (warm[i] - base) * golden * 0.55
    })
}

fn local_hour() -> f32 {
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut time: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut time).is_null() {
            return 12.0;
        }
        time.tm_hour as f32 + time.tm_min as f32 / 60.0
    }
}
