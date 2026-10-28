pub const BAR_HEIGHT: u32 = 32;

pub fn battery_label(ready: bool, percentage: f64, charging: bool) -> String {
    if !ready {
        return String::new();
    }
    let level = percentage.round().clamp(0.0, 100.0) as u32;
    if charging { format!("{level}% +") } else { format!("{level}%") }
}
