use crate::color::{lstar_from_y, y_from_lstar};

pub fn ratio_of_tones(tone_a: f64, tone_b: f64) -> f64 {
    let tone_a = tone_a.clamp(0.0, 100.0);
    let tone_b = tone_b.clamp(0.0, 100.0);

    ratio_of_ys(y_from_lstar(tone_a), y_from_lstar(tone_b))
}

fn ratio_of_ys(y1: f64, y2: f64) -> f64 {
    let lighter = if y1 > y2 { y1 } else { y2 };
    let darker = if (lighter - y2).abs() < f64::EPSILON { y1 } else { y2 };

    (lighter + 5.0) / (darker + 5.0)
}

pub fn lighter(tone: f64, ratio: f64) -> f64 {
    if !(0.0..=100.0).contains(&tone) {
        return -1.0;
    }

    let dark_y = y_from_lstar(tone);
    let light_y = ratio.mul_add(dark_y + 5.0, -5.0);
    let real_contrast = ratio_of_ys(light_y, dark_y);
    let delta = (real_contrast - ratio).abs();

    if real_contrast < ratio && delta > 0.04 {
        return -1.0;
    }

    let return_value = lstar_from_y(light_y) + 0.4;

    if !(0.0..=100.0).contains(&return_value) {
        return -1.0;
    }

    return_value
}

pub fn darker(tone: f64, ratio: f64) -> f64 {
    if !(0.0..=100.0).contains(&tone) {
        return -1.0;
    }

    let light_y = y_from_lstar(tone);
    let dark_y = ((light_y + 5.0) / ratio) - 5.0;
    let real_contrast = ratio_of_ys(light_y, dark_y);

    let delta = (real_contrast - ratio).abs();

    if real_contrast < ratio && delta > 0.04 {
        return -1.0;
    }

    let return_value = lstar_from_y(dark_y) - 0.4;

    if !(0.0..=100.0).contains(&return_value) {
        return -1.0;
    }

    return_value
}

pub fn lighter_unsafe(tone: f64, ratio: f64) -> f64 {
    let lighter_safe = lighter(tone, ratio);

    if lighter_safe < 0.0 { 100.0 } else { lighter_safe }
}

pub fn darker_unsafe(tone: f64, ratio: f64) -> f64 {
    let darker_safe = darker(tone, ratio);

    if darker_safe < 0.0 { 0.0 } else { darker_safe }
}
