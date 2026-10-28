use crate::{
    Map,
    color::{Argb, Lab},
    hct::Hct,
    utils::math::sanitize_degrees_double,
};
use core::cmp::Ordering;

/// Warm and cool relationships of a colour, cached per input.
pub struct TemperatureCache {
    input: Hct,

    _hcts_by_temp: Vec<Hct>,
    _hcts_by_hue: Vec<Hct>,
    _temps_by_hct: Map<Hct, f64>,
    _input_relative_temperature: f64,
    _complement: Option<Hct>,
}

impl TemperatureCache {
    /// Warmest colour among the hues at the input chroma and tone.
    pub fn warmest(&mut self) -> Hct {
        let hcts = self.hcts_by_temp();

        *hcts.last().unwrap()
    }

    /// Coldest colour among the hues at the input chroma and tone.
    pub fn coldest(&mut self) -> Hct {
        let hcts = self.hcts_by_temp();

        *hcts.first().unwrap()
    }

    /// Cache for `input`.
    pub fn new(input: Hct) -> Self {
        Self {
            input,
            _hcts_by_temp: vec![],
            _hcts_by_hue: vec![],
            _temps_by_hct: Map::default(),
            _input_relative_temperature: -1.0,
            _complement: None,
        }
    }

    /// Colours around the input hue that are `count` (default 5) steps of `divisions` (default 12) apart in temperature.
    pub fn analogous(&mut self, count: Option<i32>, divisions: Option<i32>) -> Vec<Hct> {
        let count = count.unwrap_or(5);
        let divisions = divisions.unwrap_or(12);
        let start_hue = self.input.get_hue().round() as i32;
        let start_hct = &self.hcts_by_hue()[start_hue as usize];
        let mut last_temp = self.relative_temperature(start_hct);
        let mut all_colors = vec![*start_hct];

        let mut absolute_total_temp_delta = 0.0;

        for i in 0..360 {
            let hue = sanitize_degrees_double((start_hue + i).into());
            let hct = &self.hcts_by_hue()[hue as usize];
            let temp = self.relative_temperature(hct);
            let temp_delta = (temp - last_temp).abs();

            last_temp = temp;
            absolute_total_temp_delta += temp_delta;
        }

        let mut hue_addend = 1;
        let temp_step = absolute_total_temp_delta / f64::from(divisions);

        let mut total_temp_delta = 0.0;

        last_temp = self.relative_temperature(start_hct);

        while all_colors.len() < divisions as usize {
            let hue = sanitize_degrees_double((start_hue + hue_addend).into());
            let hct = &self.hcts_by_hue()[hue as usize];
            let temp = self.relative_temperature(hct);
            let temp_delta = (temp - last_temp).abs();

            total_temp_delta += temp_delta;

            let desired_total_temp_delta_for_index = all_colors.len() as f64 * temp_step;

            let mut index_satisfied = total_temp_delta >= desired_total_temp_delta_for_index;
            let mut index_addend = 1;

            while index_satisfied && all_colors.len() < divisions as usize {
                all_colors.push(*hct);

                let desired_total_temp_delta_for_index = (all_colors.len() + index_addend) as f64 * temp_step;

                index_satisfied = total_temp_delta >= desired_total_temp_delta_for_index;
                index_addend += 1;
            }

            last_temp = temp;
            hue_addend += 1;

            if hue_addend > 360 {
                while all_colors.len() < divisions as usize {
                    all_colors.push(*hct);
                }

                break;
            }
        }

        let mut answers = vec![self.input];

        let increase_hue_count = ((f64::from(count) - 1.0) / 2.0).floor() as isize;

        for i in 1..=increase_hue_count {
            let mut index = 0_isize - i;

            while index < 0 {
                index += all_colors.len() as isize;
            }

            if index >= all_colors.len() as isize {
                index %= all_colors.len() as isize;
            }

            answers.insert(0, all_colors[index as usize]);
        }

        let decrease_hue_count = (count - (increase_hue_count as i32) - 1) as isize;

        for i in 1..=decrease_hue_count {
            let mut index = i;

            while index < 0 {
                index += all_colors.len() as isize;
            }

            if index >= all_colors.len() as isize {
                index %= all_colors.len() as isize;
            }

            answers.push(all_colors[index as usize]);
        }

        answers
    }

    /// Colour opposite the input on the temperature axis.
    pub fn complement(&mut self) -> Hct {
        if let Some(_complement) = &self._complement {
            return *_complement;
        }

        let coldest_hct = self.coldest();
        let warmest_hct = self.warmest();

        let temps_by_hct = self.temps_by_hct();

        let coldest_hue = coldest_hct.get_hue();
        let coldest_temp = temps_by_hct[&coldest_hct];

        let warmest_hue = warmest_hct.get_hue();
        let warmest_temp = temps_by_hct[&warmest_hct];

        let range = warmest_temp - coldest_temp;
        let start_hue_is_coldest_to_warmest = Self::is_between(self.input.get_hue(), coldest_hue, warmest_hue);
        let start_hue = if start_hue_is_coldest_to_warmest { warmest_hue } else { coldest_hue };
        let end_hue = if start_hue_is_coldest_to_warmest { coldest_hue } else { warmest_hue };
        let direction_of_rotation = 1.0_f64;
        let mut smallest_error = 1000.0;
        let mut answer = self.hcts_by_hue()[self.input.get_hue().round() as usize];

        let complement_relative_temp = 1.0 - self.input_relative_temperature();

        for hue_addend in 0..=360 {
            let hue = sanitize_degrees_double(direction_of_rotation.mul_add(f64::from(hue_addend), start_hue));

            if !Self::is_between(hue, start_hue, end_hue) {
                continue;
            }

            let possible_answer = &self.hcts_by_hue()[hue.round() as usize];
            let relative_temp = (self._temps_by_hct[possible_answer] - coldest_temp) / range;
            let error = (complement_relative_temp - relative_temp).abs();

            if error < smallest_error {
                smallest_error = error;
                answer = *possible_answer;
            }
        }

        self._complement = Some(answer);

        self._complement.unwrap()
    }

    /// Relative temperature of `hct`, from 0.0 (coldest) to 1.0 (warmest).
    pub fn relative_temperature(&mut self, hct: &Hct) -> f64 {
        let coldest = self.coldest();
        let warmest = self.warmest();

        let temps_by_hct = self.temps_by_hct();

        let range = temps_by_hct[&warmest] - temps_by_hct[&coldest];
        let difference_from_coldest = temps_by_hct[hct] - temps_by_hct[&coldest];

        if range == 0.0 {
            return 0.5;
        }

        difference_from_coldest / range
    }

    /// Relative temperature of the input.
    pub fn input_relative_temperature(&mut self) -> f64 {
        if self._input_relative_temperature >= 0.0 {
            return self._input_relative_temperature;
        }

        let coldest = self.coldest();
        let warmest = self.warmest();
        let input = self.input;

        let temps_by_hct = self.temps_by_hct();

        let coldest_temp = temps_by_hct[&coldest];

        let range = temps_by_hct[&warmest] - coldest_temp;
        let difference_from_coldest = temps_by_hct[&input] - coldest_temp;
        let input_relative_temp = if range == 0.0 { 0.5 } else { difference_from_coldest / range };

        self._input_relative_temperature = input_relative_temp;

        self._input_relative_temperature
    }

    /// Hues at the input chroma and tone, coldest first.
    pub fn hcts_by_temp(&mut self) -> &[Hct] {
        if !self._hcts_by_temp.is_empty() {
            return &self._hcts_by_temp;
        }

        let mut hcts = self.hcts_by_hue();

        hcts.push(self.input);
        hcts.sort_by(|a, b| self.sort_by_temp(a, b));

        self._hcts_by_temp = hcts;

        &self._hcts_by_temp
    }

    fn sort_by_temp(&mut self, this: &Hct, that: &Hct) -> Ordering {
        let a = self.temps_by_hct()[this];
        let b = self.temps_by_hct()[that];

        a.partial_cmp(&b).unwrap()
    }

    /// Temperature of every hue at the input chroma and tone.
    pub fn temps_by_hct(&mut self) -> &Map<Hct, f64> {
        if !self._temps_by_hct.is_empty() {
            return &self._temps_by_hct;
        }

        let mut all_hcts = self.hcts_by_hue();

        all_hcts.push(self.input);

        let mut temperatures_by_hct = Map::default();

        for e in all_hcts {
            temperatures_by_hct.insert(e, Self::raw_temperature(e));
        }

        self._temps_by_hct = temperatures_by_hct;

        &self._temps_by_hct
    }

    /// One colour per degree of hue at the input chroma and tone.
    pub fn hcts_by_hue(&mut self) -> Vec<Hct> {
        if !self._hcts_by_hue.is_empty() {
            return self._hcts_by_hue.clone();
        }

        let mut hcts = vec![];

        for hue in 0..=360 {
            let color_at_hue = Hct::from(f64::from(hue), self.input.get_chroma(), self.input.get_tone());

            hcts.push(color_at_hue);
        }

        self._hcts_by_hue = hcts;

        self._hcts_by_hue.clone()
    }

    /// Whether `angle` lies on the arc from `a` to `b`.
    pub fn is_between(angle: f64, a: f64, b: f64) -> bool {
        if a < b { a <= angle && angle <= b } else { a <= angle || angle <= b }
    }

    /// Raw (unscaled) temperature of `color`: negative is cool, positive is warm.
    pub fn raw_temperature(color: Hct) -> f64 {
        let lab = Lab::from(Argb::from(color));
        let hue = sanitize_degrees_double(lab.b.atan2(lab.a).to_degrees());

        let chroma = lab.a.hypot(lab.b);

        (0.02 * chroma.powf(1.07)).mul_add((sanitize_degrees_double(hue - 50.0).to_radians()).cos(), -0.5)
    }
}
