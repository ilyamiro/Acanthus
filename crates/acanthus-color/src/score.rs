use crate::{
    IndexMap,
    color::Argb,
    hct::Hct,
    utils::math::{difference_degrees, sanitize_degrees_int},
};

#[derive(Debug)]
struct ScoredHCT {
    hct: Hct,
    score: f64,
}

/// Ranks quantized image colours into source-colour candidates.
pub struct Score;

impl Score {
    const TARGET_CHROMA: f64 = 48.0;
    const WEIGHT_PROPORTION: f64 = 0.7;
    const WEIGHT_CHROMA_ABOVE: f64 = 0.3;
    const WEIGHT_CHROMA_BELOW: f64 = 0.1;
    const CUTOFF_CHROMA: f64 = 5.0;
    const CUTOFF_EXCITED_PROPORTION: f64 = 0.01;
    /// Returns up to `desired` (default 4) suitable source colours from a colour-to-population map, best first; `filter` (default true) drops dull and rare hues and `fallback_color_argb` (default blue) is returned when none qualify.
    pub fn score(
        colors_to_population: &IndexMap<Argb, u32>,
        desired: Option<i32>,
        fallback_color_argb: Option<Argb>,
        filter: Option<bool>,
    ) -> Vec<Argb> {
        let desired = desired.unwrap_or(4);
        let fallback_color_argb = fallback_color_argb.unwrap_or(Argb::new(255, 66, 133, 244));
        let filter = filter.unwrap_or(true);
        let mut colors_hct = vec![];
        let mut hue_population = [0; 360];
        let mut population_sum = 0.0;

        for (argb, population) in colors_to_population {
            let hct: Hct = (*argb).into();

            let hue = hct.get_hue().floor() as i32;

            colors_hct.push(hct);

            hue_population[hue as usize] += population;
            population_sum += f64::from(*population);
        }

        let mut hue_excited_proportions = [0.0; 360];

        for (hue, population) in hue_population.into_iter().enumerate().take(360) {
            let proportion = f64::from(population) / population_sum;

            for i in ((hue as i32) - 14)..((hue as i32) + 16) {
                let neighbor_hue = sanitize_degrees_int(i);

                hue_excited_proportions[neighbor_hue as usize] += proportion;
            }
        }

        let mut scored_hcts = vec![];

        for hct in colors_hct {
            let hue = hct.get_hue().round() as i32;

            let hue = sanitize_degrees_int(hue);
            let proportion = hue_excited_proportions[hue as usize];

            if filter && (hct.get_chroma() < Self::CUTOFF_CHROMA || proportion <= Self::CUTOFF_EXCITED_PROPORTION) {
                continue;
            }

            let proportion_score = proportion * 100.0 * Self::WEIGHT_PROPORTION;
            let chroma_weight = if hct.get_chroma() < Self::TARGET_CHROMA {
                Self::WEIGHT_CHROMA_BELOW
            } else {
                Self::WEIGHT_CHROMA_ABOVE
            };
            let chroma_score = (hct.get_chroma() - Self::TARGET_CHROMA) * chroma_weight;
            let score = proportion_score + chroma_score;

            scored_hcts.push(ScoredHCT { hct, score });
        }

        scored_hcts.sort_by(|a, b| unsafe { b.score.partial_cmp(&a.score).unwrap_unchecked() });

        let mut chosen_colors: Vec<Hct> = vec![];

        for difference_degree in (15..=90).rev() {
            chosen_colors.clear();

            for entry in &scored_hcts {
                let hct = entry.hct;

                if !chosen_colors.iter().any(|color| {
                    difference_degrees(entry.hct.get_hue(), color.get_hue()) < f64::from(difference_degree)
                }) {
                    chosen_colors.push(hct);
                }

                if chosen_colors.len() >= desired as usize {
                    break;
                }
            }

            if chosen_colors.len() >= desired as usize {
                break;
            }
        }

        let mut colors = vec![];

        if chosen_colors.is_empty() {
            colors.push(fallback_color_argb);
        }

        for chosen_hct in chosen_colors {
            colors.push(Argb::from(chosen_hct));
        }

        colors
    }
}
