//! Letter grades: Canvas's rounding and grading schemes.
/// Rounds the way Ruby does (half away from zero), as Canvas does before assigning letters.
/// A tiny relative nudge undoes binary noise, so 89.995 (stored as 89.99499…) rounds to 90.00.
pub fn canvas_round(value: f64, digits: i32) -> f64 {
    let factor = 10f64.powi(digits);
    let scaled = value * factor;
    let nudged = scaled + scaled.signum() * scaled.abs().max(1.0) * 1e-9;
    nudged.round() / factor
}

/// A letter-grade scheme: (letter, minimum fraction), highest first.
#[derive(Debug, Clone, PartialEq)]
pub struct Scheme {
    pub entries: Vec<(String, f64)>,
    /// Points-based schemes (e.g. a 4.0 scale) round in points before picking a letter.
    pub points_based: bool,
    pub scaling_factor: f64,
}

impl Scheme {
    pub fn new(mut entries: Vec<(String, f64)>, points_based: bool, scaling_factor: f64) -> Self {
        entries.sort_by(|a, b| b.1.total_cmp(&a.1));
        Self {
            entries,
            points_based,
            scaling_factor: if scaling_factor > 0.0 {
                scaling_factor
            } else {
                1.0
            },
        }
    }

    /// Canvas's built-in scheme, used when a course enables letter grades without its own.
    pub fn canvas_default() -> Self {
        let entries = [
            ("A", 0.94),
            ("A-", 0.90),
            ("B+", 0.87),
            ("B", 0.84),
            ("B-", 0.80),
            ("C+", 0.77),
            ("C", 0.74),
            ("C-", 0.70),
            ("D+", 0.67),
            ("D", 0.64),
            ("D-", 0.61),
            ("F", 0.0),
        ];
        Self::new(
            entries.iter().map(|(l, v)| (l.to_string(), *v)).collect(),
            false,
            1.0,
        )
    }

    /// The letter Canvas shows for an (unrounded) course percent.
    pub fn letter(&self, percent: f64) -> &str {
        let found = if self.points_based {
            let points = canvas_round(percent * self.scaling_factor / 100.0, 2);
            self.entries
                .iter()
                .find(|(_, min)| points >= canvas_round(min * self.scaling_factor, 2) - 1e-9)
        } else {
            let rounded = canvas_round(percent, 2);
            self.entries
                .iter()
                .find(|(_, min)| rounded >= canvas_round(min * 100.0, 2) - 1e-9)
        };
        found
            .or(self.entries.last())
            .map_or("", |(letter, _)| letter.as_str())
    }

    /// The lowest percent that earns `letter` (case-insensitive), if the scheme has it.
    pub fn min_percent(&self, letter: &str) -> Option<f64> {
        self.entries
            .iter()
            .find(|(l, _)| l.eq_ignore_ascii_case(letter.trim()))
            .map(|(_, min)| min * 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_is_half_away_from_zero_despite_binary_noise() {
        assert_eq!(canvas_round(89.995, 2), 90.0);
        assert_eq!(canvas_round(17999.0 / 20000.0 * 100.0, 2), 90.0);
        assert_eq!(canvas_round(89.994, 2), 89.99);
        assert_eq!(canvas_round(-1.005, 2), -1.01);
    }

    #[test]
    fn letters_use_the_rounded_score() {
        let s = Scheme::canvas_default();
        assert_eq!(s.letter(93.995), "A");
        assert_eq!(s.letter(93.994), "A-");
        assert_eq!(s.letter(86.999), "B+");
        assert_eq!(s.letter(12.0), "F");
        assert_eq!(s.min_percent(" b+ "), Some(87.0));
        assert_eq!(s.min_percent("A+"), None);
    }

    #[test]
    fn points_based_schemes_round_in_points() {
        let entries = vec![
            ("A".to_string(), 0.925),
            ("A-".to_string(), 0.9),
            ("F".to_string(), 0.0),
        ];
        // 92.4% is 3.696 points, which rounds to 3.70 = the A cutoff on a 4-point scale.
        assert_eq!(Scheme::new(entries.clone(), true, 4.0).letter(92.4), "A");
        assert_eq!(Scheme::new(entries, false, 1.0).letter(92.4), "A-");
    }
}
